//! `ci-fail`: pick a failed GitHub Actions run and copy its failed-step logs.
//!
//! Owned by migration track B. Reference: libexec/ci-fail and focal/gh_ci_fail.py.

use std::io;

#[derive(clap::Args)]
pub struct Args {
    /// Show all historical failed runs (bypasses the 24-hour filter).
    #[arg(short, long)]
    pub all: bool,
}

pub fn run(_args: Args) -> io::Result<()> {
    Err(crate::output::fail(
        "`ci-fail` is not ported to the Rust preview yet",
        "use the installed focal dispatcher",
    ))
}
