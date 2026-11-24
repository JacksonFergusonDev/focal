//! `release-context`: release membership from local Git ancestry, enriched with GitHub data.
//!
//! Owned by migration track C. Reference: libexec/release-context and the focal/release_*.py
//! and focal/gh_release_context.py modules. Milestone 6 in RUST_MIGRATION.md is the checklist.

use std::io;

#[derive(clap::Args)]
pub struct Args {
    /// minor, major, or patch (default: patch).
    #[arg(value_name = "LEVEL")]
    pub level: Option<String>,
    /// Since the previous reachable stable x.y.0 tag.
    #[arg(short = 'm', long)]
    pub minor: bool,
    /// Since the previous reachable stable x.0.0 tag.
    #[arg(long)]
    pub major: bool,
    /// Since the previous reachable stable SemVer tag (default).
    #[arg(long)]
    pub patch: bool,
    /// Release level, as an alias for the positional level.
    #[arg(long = "level", visible_alias = "since", value_name = "LEVEL")]
    pub level_flag: Option<String>,
    /// Explicit exclusive base Git ref.
    #[arg(long)]
    pub base: Option<String>,
    /// Explicit inclusive target; overrides --version tag selection.
    #[arg(long)]
    pub head: Option<String>,
    /// Release version; targets the local tag if present, otherwise HEAD.
    #[arg(long)]
    pub version: Option<String>,
    /// Select the GitHub release used as a style reference.
    #[arg(long)]
    pub previous_release: Option<String>,
    /// summary (default), include, or exclude.
    #[arg(long)]
    pub bots: Option<String>,
    /// Maximum seconds per GitHub request (default: 10).
    #[arg(long)]
    pub request_timeout: Option<f64>,
    /// Total GitHub collection budget in seconds (default: 60).
    #[arg(long)]
    pub github_timeout: Option<f64>,
    /// Approximate output budget (default: 24000; 0 disables).
    #[arg(long)]
    pub max_tokens: Option<u64>,
    /// Include the net diff between resolved endpoints.
    #[arg(long)]
    pub diff: bool,
    /// Focus the diff; repeatable, requires --diff.
    #[arg(long = "path", value_name = "PATHSPEC")]
    pub paths: Vec<String>,
    /// Append current checkout tree, status, and core manifests.
    #[arg(long)]
    pub project_context: bool,
}

pub fn run(_args: Args) -> io::Result<()> {
    Err(crate::output::fail(
        "`release-context` is not ported to the Rust preview yet",
        "use the installed focal dispatcher",
    ))
}
