//! `pr-diff`: copy a pull request's title, body, URL, and patch.
//!
//! Owned by migration track A. Reference: libexec/pr-diff and focal/gh_pr_diff.py.

use std::io;

#[derive(clap::Args)]
pub struct Args {
    /// Pull request number; omit to use the current branch's pull request.
    #[arg(value_name = "PR_ID")]
    pub pr: Option<String>,
}

pub fn run(_args: Args) -> io::Result<()> {
    Err(crate::output::fail(
        "`pr-diff` is not ported to the Rust preview yet",
        "use the installed focal dispatcher",
    ))
}
