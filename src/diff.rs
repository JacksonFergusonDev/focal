//! Git patch, stat, and selection plumbing shared by `diff` and `wip-context`, plus `diff` itself.

use crate::file::{self, Noise};
use crate::git;
use git2::{
    Delta, Diff, DiffDelta, DiffFindOptions, DiffFormat, DiffOptions, DiffStatsFormat, Repository,
    Tree,
};
use std::collections::HashSet;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Renders a diff as unified patch text.
pub fn patch(diff: &Diff) -> Result<String, git2::Error> {
    let mut out = Vec::new();
    diff.print(DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            out.push(line.origin() as u8);
        }
        out.extend_from_slice(line.content());
        true
    })?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Renders a `git diff --stat` style summary for an 80-column display.
pub fn stat(diff: &Diff) -> Result<String, git2::Error> {
    let buf = diff.stats()?.to_buf(DiffStatsFormat::FULL, 80)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

pub fn options() -> DiffOptions {
    let mut options = DiffOptions::new();
    options.id_abbrev(7);
    options
}

pub fn renames(diff: &mut Diff) -> Result<(), git2::Error> {
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))
}

pub fn head_tree(repo: &Repository) -> Result<Option<Tree<'_>>, git2::Error> {
    match repo.head() {
        Ok(head) => Ok(Some(head.peel_to_tree()?)),
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn delta_path(delta: &DiffDelta) -> Option<PathBuf> {
    delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(Path::to_path_buf)
}

/// Limits a diff to user pathspecs, or to non-noise paths when none were given.
struct Filter<'a> {
    specs: Vec<String>,
    explicit: bool,
    noise: &'a Noise,
    /// Unmerged paths, rendered separately because libgit2 has no combined diff.
    conflicts: HashSet<PathBuf>,
}

impl Filter<'_> {
    fn keep(&self, path: &Path) -> bool {
        if self.conflicts.contains(path) {
            return false;
        }
        self.explicit
            || !path
                .file_name()
                .is_some_and(|name| self.noise.matches(&name.to_string_lossy()))
    }

    /// Status lines keep unmerged paths, which [`Filter::keep`] leaves out of patches.
    fn keep_status(&self, path: &[u8]) -> bool {
        let path = String::from_utf8_lossy(path);
        let path = Path::new(&*path);
        self.conflicts.contains(path) || self.keep(path)
    }

    /// Builds the diff twice: once to find candidate paths, then limited to the kept paths
    /// so that rename detection never pairs a kept file with an excluded one.
    fn diff<'r>(
        &self,
        build: impl Fn(&mut DiffOptions) -> Result<Diff<'r>, git2::Error>,
        want: impl Fn(&DiffDelta) -> bool,
    ) -> Result<Option<Diff<'r>>, git2::Error> {
        let mut first = options();
        for spec in &self.specs {
            first.pathspec(spec);
        }
        let diff = build(&mut first)?;
        let deltas = diff.deltas().len();
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        for delta in diff.deltas().filter(|delta| want(delta)) {
            if let Some(path) = delta_path(&delta)
                && self.keep(&path)
                && seen.insert(path.clone())
            {
                kept.push(path);
            }
        }
        if kept.is_empty() {
            return Ok(None);
        }
        let mut second = options();
        second.disable_pathspec_match(true);
        for path in &kept {
            second.pathspec(path);
        }
        let mut diff = if kept.len() == deltas {
            diff
        } else {
            build(&mut second)?
        };
        renames(&mut diff)?;
        Ok(Some(diff))
    }
}

fn any(_: &DiffDelta) -> bool {
    true
}

fn untracked(delta: &DiffDelta) -> bool {
    delta.status() == Delta::Untracked
}

/// Unmerged index paths matching the user's pathspecs.
fn conflicted(repo: &Repository, specs: &[String]) -> Result<Vec<PathBuf>, git2::Error> {
    let index = repo.index()?;
    if !index.has_conflicts() {
        return Ok(Vec::new());
    }
    let pathspec = (!specs.is_empty()).then(|| git2::Pathspec::new(specs.iter())).transpose()?;
    let mut paths = Vec::new();
    for conflict in index.conflicts()? {
        let conflict = conflict?;
        let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor) else {
            continue;
        };
        let path = PathBuf::from(String::from_utf8_lossy(&entry.path).into_owned());
        if pathspec
            .as_ref()
            .is_none_or(|spec| spec.matches_path(&path, git2::PathspecFlags::DEFAULT))
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// The conflict regions of a working-tree file with three lines of context, standing in
/// for Git's combined diff.
fn conflict_regions(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("<<<<<<<") {
            start.get_or_insert(index);
        } else if line.starts_with(">>>>>>>")
            && let Some(begin) = start.take()
        {
            let from = begin.saturating_sub(3);
            let to = (index + 4).min(lines.len());
            match regions.last_mut() {
                Some(last) if from <= last.1 => last.1 = to,
                _ => regions.push((from, to)),
            }
        }
    }
    let mut out = String::new();
    for (from, to) in regions {
        out.push_str(&format!("@@ lines {}-{} @@\n", from + 1, to));
        for line in &lines[from..to] {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Text standing in for the patch of an unmerged path.
fn unmerged(repo: &Repository, path: &Path, mode: Mode) -> String {
    let mut out = format!("* Unmerged path {}\n", path.display());
    if mode == Mode::Staged {
        return out;
    }
    let file = repo.workdir().map(|root| root.join(path));
    match file.and_then(|file| std::fs::read(file).ok()) {
        Some(bytes) if bytes.contains(&0) => out.push_str("(binary file)\n"),
        Some(bytes) => {
            let regions = conflict_regions(&String::from_utf8_lossy(&bytes));
            if regions.is_empty() {
                out.push_str("(no conflict markers in the working tree)\n");
            }
            out.push_str(&regions);
        }
        None => out.push_str("(deleted in the working tree)\n"),
    }
    out
}

/// Resolves a cwd-relative pathspec against the repository root, collapsing `.` and `..`.
fn root_spec(prefix: &Path, arg: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in prefix.join(arg).components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    let mut spec = parts.join("/");
    if arg.ends_with('/') && !spec.is_empty() {
        spec.push('/');
    }
    spec
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Unstaged,
    Staged,
    All,
}

/// Renders the `diff` payload body, or `None` when the selection has no changes.
pub fn render(
    mode: Mode,
    thick: bool,
    paths: Vec<String>,
) -> io::Result<(Option<String>, &'static str)> {
    let cwd = std::env::current_dir()?;
    let repo = git::required(&cwd)?;
    let prefix = git::prefix(&repo, &cwd);
    let noise = Noise::load();
    let explicit = !paths.is_empty();
    let specs: Vec<String> = paths
        .iter()
        .map(|arg| root_spec(&prefix, arg))
        .filter(|spec| !spec.is_empty())
        .collect();
    let conflicts = conflicted(&repo, &specs).map_err(io::Error::other)?;
    let filter = Filter {
        specs,
        explicit,
        noise: &noise,
        conflicts: conflicts.iter().cloned().collect(),
    };
    let target = match (explicit, mode) {
        (true, _) => "specified files",
        (_, Mode::Unstaged) => "working tree",
        (_, Mode::Staged) => "staged changes",
        (_, Mode::All) => "all changes",
    };
    let run = || -> Result<Option<String>, git2::Error> {
        let head = head_tree(&repo)?;
        let unstaged = |o: &mut DiffOptions| repo.diff_index_to_workdir(None, Some(o));
        let staged = |o: &mut DiffOptions| repo.diff_tree_to_index(head.as_ref(), None, Some(o));
        let all =
            |o: &mut DiffOptions| repo.diff_tree_to_workdir_with_index(head.as_ref(), Some(o));
        let new_files = filter.diff(
            |o| {
                o.include_untracked(true)
                    .recurse_untracked_dirs(true)
                    .show_untracked_content(true);
                repo.diff_index_to_workdir(None, Some(o))
            },
            untracked,
        )?;
        let untracked_warning =
            "You have untracked files not included in this diff. Use --all to include them.";
        let diff = match mode {
            Mode::Staged => {
                if filter.diff(unstaged, any)?.is_some() {
                    eprintln!(
                        "warning: You have unstaged changes not included in this diff. Use --all to include everything."
                    );
                }
                if new_files.is_some() {
                    eprintln!("warning: {untracked_warning}");
                }
                filter.diff(staged, any)?
            }
            Mode::Unstaged => {
                if filter.diff(staged, any)?.is_some() {
                    eprintln!(
                        "warning: You have staged changes not included in this diff. Use --staged or --all."
                    );
                }
                if new_files.is_some() {
                    eprintln!("warning: {untracked_warning}");
                }
                filter.diff(unstaged, any)?
            }
            Mode::All => filter.diff(all, any)?,
        };
        let extra = new_files.filter(|_| mode == Mode::All);
        let mut patches = String::new();
        if let Some(diff) = &diff {
            patches.push_str(&patch(diff)?);
        }
        if let Some(extra) = &extra {
            patches.push_str(&patch(extra)?);
        }
        for path in &conflicts {
            patches.push_str(&unmerged(&repo, path, mode));
        }
        let patches = patches.trim_end_matches('\n');
        if patches.is_empty() {
            return Ok(None);
        }

        let mut out = String::new();
        let status = git::porcelain_where(&repo, &filter.specs, |path| filter.keep_status(path))?;
        if !status.is_empty() {
            out.push_str(&format!("### Status\n```\n{status}```\n\n"));
        }
        let mut summary = match &diff {
            Some(diff) => stat(diff)?,
            None => String::new(),
        };
        if !conflicts.is_empty() {
            // Unmerged rows go above the totals line, as in `git diff --stat`.
            let totals = summary
                .trim_end_matches('\n')
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let rows: String = conflicts
                .iter()
                .map(|path| format!(" {} | Unmerged\n", path.display()))
                .collect();
            summary.insert_str(totals, &rows);
        }
        {
            if !summary.trim().is_empty() {
                out.push_str(&format!("### Summary\n```\n{summary}```\n\n"));
            }
        }
        out.push_str(&format!("### Diff\n{}\n\n", file::fenced(patches, "diff")));

        if thick {
            eprintln!("info: Generating thick diff contextual grounding...");
            out.push_str(&format!(
                "### Macroscopic Map\n```text\n{}\n```\n\n### Contextual Grounding\n",
                crate::tree::render().map_err(|e| git2::Error::from_str(&e.to_string()))?
            ));
            let mut files: Vec<PathBuf> = Vec::new();
            for delta in diff.iter().chain(extra.iter()).flat_map(|d| d.deltas()) {
                if delta.status() != Delta::Deleted
                    && let Some(path) = delta.new_file().path()
                {
                    files.push(git::relative(path, &prefix));
                }
            }
            files.extend(conflicts.iter().map(|path| git::relative(path, &prefix)));
            let mut sections = String::new();
            for path in files.iter().filter(|path| path.is_file()) {
                match file::format(path, &noise) {
                    Ok(section) => sections.push_str(&section.markdown),
                    Err(error) => eprintln!("warning: {}: {error}", path.display()),
                }
            }
            out.push_str(sections.trim_end_matches('\n'));
        }
        Ok(Some(out))
    };
    Ok((run().map_err(io::Error::other)?, target))
}
