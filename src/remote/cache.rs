//! The on-disk cache: entry naming, lock files, access markers, and TTL cleanup.
//!
//! Layout under `${XDG_CACHE_HOME:-$HOME/.cache}/focal/repos`, shared with the legacy
//! dispatcher:
//!
//! - `<basename>_<16 hex>/`: a checkout; `.git/.focal_created` and `.git/.focal_accessed`
//!   mark it as Focal's and record its last use.
//! - `.last_cleanup`: epoch seconds of the last cleanup pass (one pass per day).
//! - `.locks/<entry>.lock`: held exclusively while an entry is cloned or updated.
//! - `.locks/<entry>.use`: held shared for as long as a process works in the entry;
//!   cleanup only removes an entry it can lock exclusively.
//! - `.tmp-<pid>-<entry>/` and `.trash-<pid>-<entry>/`: a clone in progress and an entry
//!   being deleted. Neither is ever visible under the entry's own name.
//!
//! The hidden names keep these out of the legacy cleanup's `*` glob.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;
pub const CREATED: &str = ".focal_created";
pub const ACCESSED: &str = ".focal_accessed";

/// `${XDG_CACHE_HOME:-$HOME/.cache}/focal/repos`.
pub fn root() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => match std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            Some(home) => PathBuf::from(home).join(".cache"),
            None => {
                return Err(crate::output::fail(
                    "cannot locate the cache directory for --repo",
                    "set HOME or XDG_CACHE_HOME",
                ));
            }
        },
    };
    Ok(base.join("focal").join("repos"))
}

/// The entry name legacy derives with `basename "$url" .git` and
/// `echo -n "$url" | shasum -a 256 | head -c 16`.
pub fn entry_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}_{hex}", basename(url))
}

/// POSIX `basename string .git`.
fn basename(url: &str) -> &str {
    let trimmed = url.trim_end_matches('/');
    if trimmed.is_empty() {
        return if url.is_empty() { "" } else { "/" };
    }
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    match base.strip_suffix(".git") {
        Some(stem) if !stem.is_empty() => stem,
        _ => base,
    }
}

/// Whether `name` has the `<basename>_<16 lowercase hex>` shape of a Focal entry.
pub fn is_entry_name(name: &str) -> bool {
    name.rsplit_once('_').is_some_and(|(stem, hash)| {
        !stem.is_empty()
            && !name.starts_with('.')
            && hash.len() == 16
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A lock file under `.locks/`.
pub fn lock_file(root: &Path, entry: &str, kind: &str) -> io::Result<File> {
    let dir = root.join(".locks");
    fs::create_dir_all(&dir)?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{entry}.{kind}")))
}

/// Tries an exclusive lock without waiting; `Ok(false)` when another process holds it.
pub fn try_exclusive(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// A scratch directory name for `entry` owned by this process.
pub fn scratch(root: &Path, kind: &str, entry: &str) -> PathBuf {
    root.join(format!(".{kind}-{}-{entry}", std::process::id()))
}

/// The entry a `.tmp-<pid>-<entry>` or `.trash-<pid>-<entry>` directory belongs to.
fn scratch_owner(name: &str) -> Option<&str> {
    let rest = name
        .strip_prefix(".tmp-")
        .or_else(|| name.strip_prefix(".trash-"))?;
    let (pid, entry) = rest.split_once('-')?;
    (pid.bytes().all(|b| b.is_ascii_digit()) && !entry.is_empty()).then_some(entry)
}

/// Removes leftover scratch directories of `entry` from interrupted runs. The caller
/// holds the entry's update lock, so none of them belongs to a live process.
pub fn remove_scratch(root: &Path, entry: &str) {
    let Ok(dir) = fs::read_dir(root) else { return };
    for item in dir.flatten() {
        let name = item.file_name();
        if scratch_owner(&name.to_string_lossy()) == Some(entry) {
            let _ = fs::remove_dir_all(item.path());
        }
    }
}

/// Creates or touches a marker in the checkout's `.git` directory.
pub fn mark(entry: &Path, marker: &str) -> io::Result<()> {
    let git_dir = entry.join(".git");
    if !git_dir.is_dir() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(git_dir.join(marker))?;
    file.set_modified(SystemTime::now())
}

/// Whole days since `path` was modified, as `find -mtime` counts them.
fn age_days(path: &Path, now: SystemTime) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
    Some(age.as_secs() / DAY)
}

/// Whether an entry has outlived the TTL. Entries with an access marker age from their
/// last use; older entries without one age from the directory itself, as in legacy.
fn is_stale(entry: &Path, ttl_days: u64, now: SystemTime) -> bool {
    let marker = entry.join(".git").join(ACCESSED);
    let reference = if marker.is_file() {
        marker
    } else {
        entry.to_path_buf()
    };
    age_days(&reference, now).is_some_and(|days| days > ttl_days)
}

fn ttl_days() -> Option<u64> {
    match std::env::var("FOCAL_REPO_TTL_DAYS") {
        Ok(value) if !value.is_empty() => value.trim().parse().ok(),
        _ => Some(30),
    }
}

/// Prunes entries unused for longer than `FOCAL_REPO_TTL_DAYS` (30 by default), at most
/// once a day. Only `<basename>_<hash>` checkouts are considered, never `active`, and
/// never one another process is using or updating.
pub fn cleanup(root: &Path, active: &str) {
    let Some(ttl) = ttl_days() else { return };
    let now = SystemTime::now();
    let epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let sentinel = root.join(".last_cleanup");
    if let Ok(text) = fs::read_to_string(&sentinel)
        && let Ok(last) = text.trim().parse::<u64>()
        && epoch.saturating_sub(last) < DAY
    {
        return;
    }
    let _ = fs::write(&sentinel, format!("{epoch}\n"));

    let Ok(dir) = fs::read_dir(root) else { return };
    let mut names: Vec<String> = dir
        .flatten()
        .filter(|item| item.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|item| item.file_name().into_string().ok())
        .collect();
    names.sort();
    for name in names {
        if let Some(owner) = scratch_owner(&name) {
            // Leftovers of a killed run; a live clone holds its entry's update lock.
            if owner != active
                && let Some(_guard) = claim(root, owner)
            {
                let _ = fs::remove_dir_all(root.join(&name));
            }
            continue;
        }
        if name == active || !is_entry_name(&name) {
            continue;
        }
        let entry = root.join(&name);
        if !entry.join(".git").is_dir() || !is_stale(&entry, ttl, now) {
            continue;
        }
        let Some(_guard) = claim(root, &name) else {
            continue;
        };
        // Re-check under the locks: a process may have used it since the first look.
        if !is_stale(&entry, ttl, now) {
            continue;
        }
        let label = if entry.join(".git").join(ACCESSED).is_file() {
            "stale"
        } else {
            "legacy"
        };
        eprintln!("info: Pruning {label} cached repository: {name}");
        let trash = scratch(root, "trash", &name);
        if fs::rename(&entry, &trash).is_ok() {
            let _ = fs::remove_dir_all(&trash);
        }
    }
}

/// Both locks of an entry, taken without waiting, or `None` when it is busy.
fn claim(root: &Path, entry: &str) -> Option<(File, File)> {
    let usage = lock_file(root, entry, "use").ok()?;
    if !try_exclusive(&usage).ok()? {
        return None;
    }
    let update = lock_file(root, entry, "lock").ok()?;
    if !try_exclusive(&update).ok()? {
        return None;
    }
    Some((usage, update))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_match_the_legacy_scheme() {
        // printf %s https://github.com/octo/widget.git | shasum -a 256 | head -c 16
        assert_eq!(
            entry_name("https://github.com/octo/widget.git"),
            "widget_cae6c83790b988f1"
        );
        assert_eq!(basename("https://github.com/octo/widget.git"), "widget");
        assert_eq!(basename("https://github.com/octo/widget/"), "widget");
        assert_eq!(basename("git@github.com:octo/widget.git"), "widget");
        assert_eq!(basename("git@host:widget.git"), "git@host:widget");
        assert_eq!(basename("/srv/.git"), ".git");
        assert_eq!(basename("/"), "/");
    }

    #[test]
    fn entry_names_are_recognized() {
        assert!(is_entry_name("widget_0123456789abcdef"));
        assert!(is_entry_name("my_repo_0123456789abcdef"));
        assert!(!is_entry_name("widget_0123456789ABCDEF"));
        assert!(!is_entry_name("widget"));
        assert!(!is_entry_name("_0123456789abcdef"));
        assert!(!is_entry_name(".tmp-1-widget_0123456789abcdef"));
    }

    #[test]
    fn scratch_names_name_their_entry() {
        assert_eq!(scratch_owner(".tmp-42-a-b_0123"), Some("a-b_0123"));
        assert_eq!(scratch_owner(".trash-7-x"), Some("x"));
        assert_eq!(scratch_owner(".tmp-x-y"), None);
        assert_eq!(scratch_owner("widget"), None);
    }
}
