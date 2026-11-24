//! `issues`: list open (or all) issues, let the user pick, and copy each with its comments.
//!
//! Owned by migration track A. Reference: libexec/issues and focal/gh_issues.py.

use std::io;

#[derive(clap::Args)]
pub struct Args {
    /// Show both open and closed issues.
    #[arg(long)]
    pub all: bool,
}

pub fn run(_args: Args) -> io::Result<()> {
    Err(crate::output::fail(
        "`issues` is not ported to the Rust preview yet",
        "use the installed focal dispatcher",
    ))
}
