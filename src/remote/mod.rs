//! `--repo`: run a command inside a cached checkout of a remote repository.
//!
//! The first use clones into a scratch directory and renames it into place, so an
//! entry only ever appears complete. Later uses fetch, refresh origin/HEAD to the
//! remote's default branch, and hard-reset to it, or reuse the checkout when the
//! remote is unreachable. An update lock serializes clones and updates of one entry;
//! a shared use lock, held until the process exits, keeps cleanup away from entries in
//! use. See `cache` for the layout shared with the legacy dispatcher.

mod cache;
mod sync;

pub use sync::redact;

use std::fs::{self, File};
use std::io;
use std::path::Path;

const OFFLINE: &str = "warning: Could not update the cached repository; using the cached copy.";

/// Prepares the cached checkout for `url` and changes the working directory into it.
pub fn enter(url: &str) -> io::Result<()> {
    let root = cache::root()?;
    fs::create_dir_all(&root).map_err(|error| {
        crate::output::fail(
            format!("could not create the --repo cache at {}: {error}", root.display()),
            "set XDG_CACHE_HOME to a writable directory",
        )
    })?;
    let name = cache::entry_name(url);
    let entry = root.join(&name);

    let usage = cache::lock_file(&root, &name, "use")?;
    usage.lock_shared()?;
    let update = cache::lock_file(&root, &name, "lock")?;
    if !cache::try_exclusive(&update)? {
        eprintln!("info: Waiting for another focal process to update the cached repository...");
        update.lock()?;
    }
    cache::remove_scratch(&root, &name);

    prepare(url, &root, &name, &entry, &usage)?;
    cache::mark(&entry, cache::ACCESSED)?;
    drop(update);
    // Keep the shared use lock until the process exits.
    std::mem::forget(usage);

    cache::cleanup(&root, &name);
    std::env::set_current_dir(&entry)
}

fn prepare(url: &str, root: &Path, name: &str, entry: &Path, usage: &File) -> io::Result<()> {
    if !entry.is_dir() {
        eprintln!("info: Cloning remote repository into cache...");
        let temp = clone_scratch(url, root, name)?;
        return fs::rename(&temp, entry).inspect_err(|_| {
            let _ = fs::remove_dir_all(&temp);
        });
    }

    eprintln!("info: Updating cached remote repository...");
    // Refresh the access time first so a concurrent cleanup cannot expire the entry.
    cache::mark(entry, cache::ACCESSED)?;

    if sync::is_partial(entry) {
        // A legacy blob-filtered clone: libgit2 can neither open it nor fetch missing
        // blobs, so replace it with a full clone, or keep using it while offline.
        return match replace(url, root, name, entry, usage) {
            Ok(true) => Ok(()),
            Ok(false) => {
                sync::allow_partial_clones();
                eprintln!(
                    "warning: The cached repository is in use by another focal process; using the cached copy."
                );
                Ok(())
            }
            Err(error) => {
                sync::allow_partial_clones();
                offline(&error.to_string(), url);
                Ok(())
            }
        };
    }

    let repo = match sync::open(entry) {
        Ok(repo) => repo,
        // Not a usable repository (for example a legacy clone that was killed): clone
        // it again, or fail when that is impossible.
        Err(_) => {
            return match replace(url, root, name, entry, usage)? {
                true => Ok(()),
                false => Err(crate::output::fail(
                    "the cached repository is damaged and in use by another focal process",
                    "try again when the other command finishes",
                )),
            };
        }
    };
    match sync::fetch(&repo) {
        Ok(default) => {
            if let Err(error) = sync::reset_to_default(&repo, default.as_deref()) {
                eprintln!(
                    "warning: Could not reset the cached repository to the remote's default branch: {}",
                    sync::redact(error.message(), url)
                );
            }
        }
        Err(error) => offline(error.message(), url),
    }
    Ok(())
}

fn offline(reason: &str, url: &str) {
    eprintln!("{OFFLINE}\n  reason: {}", sync::redact(reason, url));
}

/// Clones `url` into a fresh scratch directory marked as Focal's and returns it.
fn clone_scratch(url: &str, root: &Path, name: &str) -> io::Result<std::path::PathBuf> {
    let temp = cache::scratch(root, "tmp", name);
    if let Err(error) = sync::clone(url, &temp) {
        let _ = fs::remove_dir_all(&temp);
        return Err(crate::output::fail(
            format!(
                "could not clone the remote repository: {}",
                sync::redact(error.message(), url)
            ),
            "check the URL, your network connection, and your Git credentials",
        ));
    }
    cache::mark(&temp, cache::CREATED)?;
    cache::mark(&temp, cache::ACCESSED)?;
    Ok(temp)
}

/// Replaces an existing entry with a fresh clone. Returns `Ok(false)`, leaving the
/// entry alone, when another process is working in it.
fn replace(url: &str, root: &Path, name: &str, entry: &Path, usage: &File) -> io::Result<bool> {
    let temp = clone_scratch(url, root, name)?;
    // Swapping the directory under another process would pull its checkout away.
    usage.unlock()?;
    if !cache::try_exclusive(usage)? {
        usage.lock_shared()?;
        let _ = fs::remove_dir_all(&temp);
        return Ok(false);
    }
    let trash = cache::scratch(root, "trash", name);
    let swapped = fs::rename(entry, &trash).and_then(|()| {
        fs::rename(&temp, entry).inspect_err(|_| {
            let _ = fs::rename(&trash, entry);
        })
    });
    let _ = fs::remove_dir_all(&trash);
    let _ = fs::remove_dir_all(&temp);
    usage.unlock()?;
    usage.lock_shared()?;
    swapped.map(|()| true)
}
