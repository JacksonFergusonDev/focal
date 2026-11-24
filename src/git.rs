use git2::{Delta, ErrorCode, Repository, Status, StatusEntry, StatusOptions};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Opens the repository whose working tree contains `cwd`, mirroring
/// `git rev-parse --is-inside-work-tree`.
pub fn open(cwd: &Path) -> Result<Option<Repository>, git2::Error> {
    let repo = match Repository::open_from_env() {
        Ok(repo) => repo,
        Err(error) if error.code() == ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let inside = repo
        .workdir()
        .is_some_and(|workdir| cwd.starts_with(workdir) && !cwd.starts_with(repo.path()));
    Ok(inside.then_some(repo))
}

/// The current branch (empty when detached) and abbreviated HEAD (`none` when unborn).
pub fn head(repo: &Repository) -> Result<(String, String), git2::Error> {
    let reference = repo.find_reference("HEAD")?;
    let branch = reference
        .symbolic_target()
        .and_then(|name| name.strip_prefix("refs/heads/"))
        .unwrap_or("")
        .to_owned();
    let commit = match repo.head() {
        Ok(head) => head
            .peel_to_commit()?
            .as_object()
            .short_id()?
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        Err(error) if error.code() == ErrorCode::UnbornBranch => "none".into(),
        Err(error) => return Err(error),
    };
    Ok((branch, commit))
}

/// Quotes a path the way Git's default `core.quotePath` does in short status.
fn quote(path: &[u8]) -> String {
    let plain = path
        .iter()
        .all(|&byte| (0x21..0x7f).contains(&byte) && byte != b'"' && byte != b'\\');
    if plain {
        return String::from_utf8_lossy(path).into_owned();
    }
    let mut out = String::from("\"");
    for &byte in path {
        match byte {
            7 => out.push_str("\\a"),
            8 => out.push_str("\\b"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            11 => out.push_str("\\v"),
            12 => out.push_str("\\f"),
            b'\r' => out.push_str("\\r"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..0x7f => out.push(byte as char),
            _ => write!(out, "\\{byte:03o}").unwrap(),
        }
    }
    out.push('"');
    out
}

fn conflict_code(repo: &Repository, path: &[u8]) -> Result<&'static str, git2::Error> {
    let index = repo.index()?;
    let Some(conflict) = index.conflicts()?.filter_map(Result::ok).find(|conflict| {
        [&conflict.ancestor, &conflict.our, &conflict.their]
            .into_iter()
            .flatten()
            .any(|entry| entry.path == path)
    }) else {
        return Ok("UU");
    };
    Ok(
        match (
            conflict.ancestor.is_some(),
            conflict.our.is_some(),
            conflict.their.is_some(),
        ) {
            (true, false, false) => "DD",
            (false, true, false) => "AU",
            (true, true, false) => "UD",
            (false, false, true) => "UA",
            (true, false, true) => "DU",
            (false, true, true) => "AA",
            _ => "UU",
        },
    )
}

fn line(repo: &Repository, entry: &StatusEntry) -> Result<Option<(Vec<u8>, String)>, git2::Error> {
    let status = entry.status();
    let path = entry.path_bytes().to_owned();
    if status.is_conflicted() {
        let code = conflict_code(repo, &path)?;
        return Ok(Some((path.clone(), format!("{code} {}", quote(&path)))));
    }
    if status.is_wt_new() {
        return Ok(Some((path.clone(), format!("?? {}", quote(&path)))));
    }
    let x = if status.is_index_new() {
        'A'
    } else if status.is_index_deleted() {
        'D'
    } else if status.is_index_renamed() {
        'R'
    } else if status.is_index_typechange() {
        'T'
    } else if status.is_index_modified() {
        'M'
    } else {
        ' '
    };
    let y = if status.is_wt_deleted() {
        'D'
    } else if status.is_wt_typechange() {
        'T'
    } else if status.is_wt_modified() {
        'M'
    } else {
        ' '
    };
    if x == ' ' && y == ' ' {
        return Ok(None);
    }
    let renamed = entry.head_to_index().filter(|_| x == 'R');
    let text = match renamed {
        Some(delta) => {
            let old = delta.old_file().path_bytes().unwrap_or_default();
            let new = delta.new_file().path_bytes().unwrap_or_default();
            return Ok(Some((
                new.to_owned(),
                format!("{x}{y} {} -> {}", quote(old), quote(new)),
            )));
        }
        None => format!("{x}{y} {}", quote(&path)),
    };
    Ok(Some((path, text)))
}

/// Renders `git status --porcelain` (v1) with Git's default untracked and rename settings.
pub fn porcelain(repo: &Repository) -> Result<String, git2::Error> {
    porcelain_where(repo, &[], |_| true)
}

/// Porcelain status limited to root-relative `pathspecs` (all when empty) and paths `keep` accepts.
pub fn porcelain_where(
    repo: &Repository,
    pathspecs: &[String],
    keep: impl Fn(&[u8]) -> bool,
) -> Result<String, git2::Error> {
    let mut options = StatusOptions::new();
    for spec in pathspecs {
        options.pathspec(spec);
    }
    options
        .include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false)
        .exclude_submodules(false)
        .renames_head_to_index(true);
    let statuses = repo.statuses(Some(&mut options))?;
    let mut lines = Vec::new();
    for entry in statuses.iter() {
        if entry.status() == Status::CURRENT || entry.status().is_ignored() {
            continue;
        }
        if let Some(line) = line(repo, &entry)?
            && keep(&line.0)
        {
            lines.push(line);
        }
    }
    // Git lists tracked changes before untracked paths, each group sorted by path.
    lines.sort_by(|a, b| (a.1.starts_with("??"), &a.0).cmp(&(b.1.starts_with("??"), &b.0)));
    Ok(lines
        .into_iter()
        .map(|(_, text)| text + "\n")
        .collect::<String>())
}

pub fn relative(path: &Path, prefix: &Path) -> PathBuf {
    let mut base = prefix;
    let mut ups = PathBuf::new();
    loop {
        if let Ok(rest) = path.strip_prefix(base) {
            return ups.join(rest);
        }
        ups.push("..");
        base = base.parent().unwrap_or(Path::new(""));
    }
}

pub fn prefix(repo: &Repository, cwd: &Path) -> PathBuf {
    repo.workdir()
        .and_then(|workdir| cwd.strip_prefix(workdir).ok())
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

pub fn required(cwd: &Path) -> std::io::Result<Repository> {
    open(cwd)
        .map_err(std::io::Error::other)?
        .ok_or_else(|| std::io::Error::other("not a git repository"))
}

/// Staged paths other than deletions, as in `git diff --name-only --cached --diff-filter=d`,
/// relative to `cwd`.
pub fn staged(cwd: &Path) -> std::io::Result<Vec<PathBuf>> {
    let repo = required(cwd)?;
    let run = || -> Result<Vec<PathBuf>, git2::Error> {
        let head = match repo.head() {
            Ok(head) => Some(head.peel_to_tree()?),
            Err(error) if error.code() == ErrorCode::UnbornBranch => None,
            Err(error) => return Err(error),
        };
        let mut diff = repo.diff_tree_to_index(head.as_ref(), None, None)?;
        diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
        let prefix = prefix(&repo, cwd);
        let mut paths: Vec<PathBuf> = diff
            .deltas()
            .filter(|delta| delta.status() != Delta::Deleted)
            .filter_map(|delta| delta.new_file().path().map(|path| relative(path, &prefix)))
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    };
    run().map_err(std::io::Error::other)
}

/// Tracked paths under `cwd` whose working-tree contents differ from the index,
/// as in `git ls-files -m`.
pub fn modified(cwd: &Path) -> std::io::Result<Vec<PathBuf>> {
    let repo = required(cwd)?;
    let run = || -> Result<Vec<PathBuf>, git2::Error> {
        let diff = repo.diff_index_to_workdir(None, None)?;
        let prefix = prefix(&repo, cwd);
        let mut paths: Vec<PathBuf> = diff
            .deltas()
            .filter(|delta| {
                matches!(
                    delta.status(),
                    Delta::Modified | Delta::Deleted | Delta::Typechange | Delta::Conflicted
                )
            })
            .filter_map(|delta| delta.old_file().path())
            .filter_map(|path| path.strip_prefix(&prefix).ok().map(Path::to_path_buf))
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    };
    run().map_err(std::io::Error::other)
}
