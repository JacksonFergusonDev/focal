//! `--repo`: run a command inside a cached checkout of a remote repository.
//!
//! Owned by migration track D. Reference: bin/focal (the --repo branch) and the
//! mark_repo_* / cleanup_stale_cached_repos helpers in lib/core.sh.

use std::io;

/// Prepares the cached checkout for `url` and changes the working directory into it.
pub fn enter(_url: &str) -> io::Result<()> {
    Err(crate::output::fail(
        "--repo is not supported by the Rust preview yet",
        "use the installed focal dispatcher",
    ))
}
