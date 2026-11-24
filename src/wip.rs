//! `wip-context`: branch topology, status, and budgeted patches since a base branch.

use crate::diff;
use crate::file::{Noise, fenced};
use crate::git;
use git2::{Commit, Delta, DiffDelta, Oid, Patch, Repository, Sort};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Character budget for the patch section (~8k tokens).
const MAX_DIFF_CHARS: usize = 30_000;

fn fail(message: String, hint: &str) -> io::Error {
    io::Error::other(format!("{message}\n  hint: {hint}"))
}

fn commit_ref(repo: &Repository, branch: &str) -> Option<String> {
    [branch.to_owned(), format!("origin/{branch}")]
        .into_iter()
        .find(|name| repo.revparse_single(&format!("{name}^{{commit}}")).is_ok())
}

/// Resolves an explicit base, the remote default branch, or main/master/develop.
fn base_ref(repo: &Repository, target: Option<&str>) -> io::Result<String> {
    if let Some(target) = target {
        return commit_ref(repo, target).ok_or_else(|| {
            fail(
                format!("specified base branch '{target}' does not exist"),
                "verify the branch name with 'git branch -a'",
            )
        });
    }
    let remote_default = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.symbolic_target().map(str::to_owned))
        .map(|name| match name.strip_prefix("refs/remotes/origin/") {
            Some(branch) => branch.to_owned(),
            None => name.rsplit('/').next().unwrap_or_default().to_owned(),
        });
    remote_default
        .iter()
        .map(String::as_str)
        .chain(["main", "master", "develop"])
        .find_map(|branch| commit_ref(repo, branch))
        .ok_or_else(|| {
            fail(
                "could not automatically detect a base branch (tried main, master, develop)".into(),
                "specify it explicitly: focal wip-context <branch>",
            )
        })
}

fn plural(n: i64, unit: &str) -> String {
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// Git's `%cr` relative date.
fn relative_date(time: i64, now: i64) -> String {
    if time > now {
        return "in the future".into();
    }
    let mut diff = now - time;
    if diff < 90 {
        return format!("{} ago", plural(diff, "second"));
    }
    diff = (diff + 30) / 60;
    if diff < 90 {
        return format!("{} ago", plural(diff, "minute"));
    }
    diff = (diff + 30) / 60;
    if diff < 36 {
        return format!("{} ago", plural(diff, "hour"));
    }
    diff = (diff + 12) / 24;
    if diff < 14 {
        return format!("{} ago", plural(diff, "day"));
    }
    if diff < 70 {
        return format!("{} ago", plural((diff + 3) / 7, "week"));
    }
    if diff < 365 {
        return format!("{} ago", plural((diff + 15) / 30, "month"));
    }
    if diff < 1825 {
        let months = (diff * 12 * 2 + 365) / (365 * 2);
        let (years, months) = (months / 12, months % 12);
        if months > 0 {
            return format!("{}, {} ago", plural(years, "year"), plural(months, "month"));
        }
        return format!("{} ago", plural(years, "year"));
    }
    format!("{} ago", plural((diff + 183) / 365, "year"))
}

fn status_letter(status: Delta) -> char {
    match status {
        Delta::Added => 'A',
        Delta::Deleted => 'D',
        Delta::Renamed => 'R',
        Delta::Copied => 'C',
        Delta::Typechange => 'T',
        _ => 'M',
    }
}

fn path_of(delta: &DiffDelta) -> String {
    diff::delta_path(delta)
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// One block per commit in `base..HEAD`, newest first, with paths changed against the
/// first parent. Merge commits list no paths, as with `git log --name-status`.
fn topology(repo: &Repository, base: Oid, now: i64) -> Result<String, git2::Error> {
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    walk.push_head()?;
    walk.hide(base)?;
    let mut blocks = Vec::new();
    for oid in walk {
        let commit = repo.find_commit(oid?)?;
        let mut block = format!(
            "[{}] {} ({})",
            short(&commit)?,
            commit.summary().unwrap_or_default(),
            relative_date(commit.committer().when().seconds(), now)
        );
        if commit.parent_count() <= 1 {
            let parent = commit.parents().next().map(|p| p.tree()).transpose()?;
            let mut changes = repo.diff_tree_to_tree(
                parent.as_ref(),
                Some(&commit.tree()?),
                Some(&mut diff::options()),
            )?;
            diff::renames(&mut changes)?;
            for delta in changes.deltas() {
                block.push('\n');
                block.push(status_letter(delta.status()));
                if delta.status() == Delta::Renamed {
                    let old = delta.old_file().path().unwrap_or(Path::new(""));
                    block.push_str(&format!("\t{}", old.display()));
                }
                block.push_str(&format!("\t{}", path_of(&delta)));
            }
        }
        blocks.push(block);
    }
    Ok(blocks.join("\n\n"))
}

fn short(commit: &Commit) -> Result<String, git2::Error> {
    Ok(commit
        .as_object()
        .short_id()?
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// Human-readable size, as in the legacy `wip-context` metadata.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b}B"),
        b if b < 1024 * 1024 => format!("{:.1}K", b as f64 / 1024.0),
        b => format!("{:.1}M", b as f64 / (1024.0 * 1024.0)),
    }
}

/// A short content-type description from magic bytes, standing in for `file -b`.
fn describe(path: &Path) -> String {
    let mut head = Vec::with_capacity(512);
    if let Ok(file) = fs::File::open(path) {
        let _ = file.take(512).read_to_end(&mut head);
    }
    let magic: [(&[u8], &str); 12] = [
        (b"\x89PNG\r\n\x1a\n", "PNG image data"),
        (b"\xff\xd8\xff", "JPEG image data"),
        (b"GIF8", "GIF image data"),
        (b"%PDF-", "PDF document"),
        (b"PK\x03\x04", "Zip archive data"),
        (b"\x1f\x8b", "gzip compressed data"),
        (b"BZh", "bzip2 compressed data"),
        (b"\xfd7zXZ\x00", "XZ compressed data"),
        (b"SQLite format 3\x00", "SQLite 3.x database"),
        (b"\x7fELF", "ELF executable"),
        (b"\xcf\xfa\xed\xfe", "Mach-O 64-bit executable"),
        (b"\x93NUMPY", "NumPy array data"),
    ];
    if let Some((_, name)) = magic.iter().find(|(prefix, _)| head.starts_with(prefix)) {
        return (*name).into();
    }
    if head.is_empty() {
        return "empty".into();
    }
    if head.contains(&0) {
        return "data".into();
    }
    if head.is_ascii() {
        "ASCII text".into()
    } else if std::str::from_utf8(&head).is_ok() {
        "Unicode text, UTF-8 text".into()
    } else {
        "data".into()
    }
}

/// Appends `### path` patch blocks within the remaining budget; returns how many were cut or skipped.
fn patches(
    diff: &git2::Diff,
    indices: &[usize],
    remaining: &mut usize,
    out: &mut Vec<String>,
) -> Result<usize, git2::Error> {
    let mut omitted = 0;
    for &index in indices {
        if *remaining == 0 {
            omitted += 1;
            continue;
        }
        let Some(mut patch) = Patch::from_diff(diff, index)? else {
            continue;
        };
        let text = String::from_utf8_lossy(&patch.to_buf()?).into_owned();
        let cleaned = text
            .trim()
            .split('\n')
            .filter(|line| !line.starts_with("index "))
            .collect::<Vec<_>>()
            .join("\n");
        if cleaned.is_empty() {
            continue;
        }
        let header = format!("### `{}`\n", path_of(&patch.delta()));
        let block = header.clone() + &fenced(&cleaned, "diff");
        let length = block.chars().count();
        if length > *remaining {
            if *remaining > 100 {
                let keep = remaining.saturating_sub(header.chars().count() + 60);
                let mut sliced: String = cleaned.chars().take(keep).collect();
                sliced.push_str("\n...[diff truncated: context limit reached]");
                out.push(header + &fenced(&sliced, "diff"));
            }
            omitted += 1;
            *remaining = 0;
        } else {
            out.push(block);
            *remaining -= length;
        }
    }
    Ok(omitted)
}

pub fn render(target: Option<&str>) -> io::Result<String> {
    let cwd = std::env::current_dir()?;
    let repo = git::required(&cwd)?;
    let resolved = base_ref(&repo, target)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let noise = Noise::load();
    let run = || -> Result<Result<String, io::Error>, git2::Error> {
        let base_oid = repo
            .revparse_single(&format!("{resolved}^{{commit}}"))?
            .id();
        let head = repo.head()?.peel_to_commit()?;
        let base = repo.find_commit(repo.merge_base(base_oid, head.id())?)?;
        let topology = topology(&repo, base.id(), now)?;
        if topology.is_empty() {
            return Ok(Err(fail(
                format!("no divergent commits found between '{resolved}' and HEAD"),
                "create commits on this branch or specify a different base branch: focal wip-context <branch>",
            )));
        }
        let mut changes = repo.diff_tree_to_tree(
            Some(&base.tree()?),
            Some(&head.tree()?),
            Some(&mut diff::options()),
        )?;
        diff::renames(&mut changes)?;

        let (mut priority, mut standard, mut omitted) = (Vec::new(), Vec::new(), Vec::new());
        for (index, delta) in changes.deltas().enumerate() {
            let path = path_of(&delta);
            let name = path.rsplit('/').next().unwrap_or(&path);
            if noise.matches(name) {
                omitted.push(path);
            } else if !path.contains('/') || path.starts_with(".github/") {
                priority.push(index);
            } else {
                standard.push(index);
            }
        }
        let mut blocks = Vec::new();
        let mut remaining = MAX_DIFF_CHARS;
        let cut = patches(&changes, &priority, &mut remaining, &mut blocks)?
            + patches(&changes, &standard, &mut remaining, &mut blocks)?;

        let mut parts = vec![
            "# WIP Branch Context".to_owned(),
            format!(
                "**Base:** `{resolved}` ({}) | **HEAD:** ({})",
                &base.id().to_string()[..7],
                short(&head)?
            ),
        ];
        let status = git::porcelain(&repo)?;
        if !status.trim().is_empty() {
            parts.push("\n## 0. Uncommitted Changes".into());
            parts.push(fenced(status.trim(), "text"));
        }
        parts.push("\n## 1. Branch Topology".into());
        parts.push(fenced(&topology, "text"));
        parts.push("\n## 2. Macroscopic Map".into());
        parts.push(fenced(diff::stat(&changes)?.trim(), "text"));
        parts.push("\n## 3. Microscopic Diffs".into());
        if !omitted.is_empty() {
            parts.push("\n## 4. Omitted Files Metadata".into());
            let root = repo.workdir().unwrap_or(Path::new("."));
            for path in &omitted {
                let full = root.join(path);
                parts.push(match fs::metadata(&full) {
                    Ok(meta) => format!(
                        "- `{path}` (Size: {})\n  - Type: {}",
                        size(meta.len()),
                        describe(&full)
                    ),
                    Err(_) => format!("- `{path}` (Deleted or inaccessible)"),
                });
            }
        }
        if blocks.is_empty() {
            parts.push("*No text diffs available or all changes were in noise files.*".into());
        } else {
            parts.extend(blocks);
        }
        if cut > 0 {
            parts.push(format!(
                "\n*...diffs for {cut} remaining files omitted (context limit reached).*"
            ));
        }
        Ok(Ok(parts.join("\n")))
    };
    run().map_err(io::Error::other)?
}
