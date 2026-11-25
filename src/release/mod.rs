//! `release-context`: release membership from local Git ancestry, enriched with GitHub data.
//!
//! Owned by migration track C. Reference: libexec/release-context and the focal/release_*.py
//! and focal/gh_release_context.py modules. Milestone 6 in RUST_MIGRATION.md is the checklist.

mod collect;
mod limit;
mod notes;
mod range;
mod report;

use crate::file::fenced;
use crate::{git, output};
use std::fs;
use std::io;
use std::path::Path;

const LEVELS: [&str; 5] = ["level", "minor", "major", "patch", "level_flag"];

/// Every level form overrides the others, so the last one given wins, as in the legacy
/// argument loop.
#[derive(clap::Args)]
#[command(args_override_self = true)]
pub struct Args {
    /// minor, major, or patch (default: patch).
    #[arg(value_name = "LEVEL", overrides_with_all = LEVELS)]
    pub level: Vec<String>,
    /// Since the previous reachable stable x.y.0 tag.
    #[arg(short = 'm', long, overrides_with_all = LEVELS)]
    pub minor: bool,
    /// Since the previous reachable stable x.0.0 tag.
    #[arg(long, overrides_with_all = LEVELS)]
    pub major: bool,
    /// Since the previous reachable stable SemVer tag (default).
    #[arg(long, overrides_with_all = LEVELS)]
    pub patch: bool,
    /// Release level, as an alias for the positional level.
    #[arg(
        long = "level",
        visible_alias = "since",
        value_name = "LEVEL",
        overrides_with_all = LEVELS
    )]
    pub level_flag: Option<String>,
    /// Explicit exclusive base Git ref.
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,
    /// Explicit inclusive target; overrides --version tag selection.
    #[arg(long, value_name = "REF")]
    pub head: Option<String>,
    /// Release version; targets the local tag if present, otherwise HEAD.
    #[arg(long, value_name = "TAG")]
    pub version: Option<String>,
    /// Select the GitHub release used as a style reference.
    #[arg(long, value_name = "TAG")]
    pub previous_release: Option<String>,
    /// summary (default), include, or exclude.
    #[arg(long, value_name = "MODE")]
    pub bots: Option<String>,
    /// Maximum seconds per GitHub request (default: 10).
    #[arg(long, value_name = "SECONDS")]
    pub request_timeout: Option<String>,
    /// Total GitHub collection budget in seconds (default: 60).
    #[arg(long, value_name = "SECONDS")]
    pub github_timeout: Option<String>,
    /// Approximate output budget (default: 24000; 0 disables).
    #[arg(long, value_name = "N")]
    pub max_tokens: Option<String>,
    /// Include the net diff between resolved endpoints.
    #[arg(long)]
    pub diff: bool,
    /// Focus the diff; repeatable, requires --diff.
    #[arg(long = "path", value_name = "PATHSPEC", action = clap::ArgAction::Append)]
    pub paths: Vec<String>,
    /// Append current checkout tree, status, and core manifests.
    #[arg(long)]
    pub project_context: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Bots {
    Summary,
    Include,
    Exclude,
}

impl Bots {
    fn name(self) -> &'static str {
        match self {
            Bots::Summary => "summary",
            Bots::Include => "include",
            Bots::Exclude => "exclude",
        }
    }
}

pub struct Options {
    pub level: String,
    pub base: Option<String>,
    pub head: Option<String>,
    pub version: Option<String>,
    pub previous_release: Option<String>,
    pub bots: Bots,
    pub request_timeout: f64,
    pub github_timeout: f64,
    pub max_tokens: usize,
    pub diff: bool,
    pub paths: Vec<String>,
}

const USAGE_HINT: &str = "run 'focal release-context --help' for usage";

fn usage(message: String) -> io::Error {
    output::fail(message, USAGE_HINT)
}

/// Rejects an empty value, as the legacy wrapper does for every value option.
fn value(flag: &str, value: Option<String>) -> io::Result<Option<String>> {
    match value {
        Some(text) if text.is_empty() => Err(usage(format!("{flag} requires an argument"))),
        other => Ok(other),
    }
}

fn seconds(flag: &str, text: Option<String>, default: f64) -> io::Result<f64> {
    let Some(text) = value(flag, text)? else {
        return Ok(default);
    };
    let parsed: f64 = text.trim().parse().map_err(|_| {
        usage(format!(
            "Invalid value for '{flag}': '{text}' is not a valid float."
        ))
    })?;
    // NaN fails this comparison too.
    if parsed > 0.0 {
        Ok(parsed)
    } else {
        Err(usage(format!(
            "Invalid value for '{flag}': {text} is not in the range x>0."
        )))
    }
}

impl Options {
    fn from_args(args: Args) -> io::Result<Self> {
        let level = if let Some(level) = args.level_flag {
            if level.is_empty() {
                return Err(output::fail(
                    "--level requires an argument (major, minor, or patch)",
                    "specify a release level",
                ));
            }
            if !matches!(level.as_str(), "major" | "minor" | "patch") {
                return Err(output::fail(
                    format!(
                        "invalid release level '{level}'. Expected 'major', 'minor', or 'patch'."
                    ),
                    "valid options: major, minor, patch",
                ));
            }
            level
        } else if args.minor {
            "minor".into()
        } else if args.major {
            "major".into()
        } else {
            let mut level = "patch".to_owned();
            for word in args.level {
                if !matches!(word.as_str(), "major" | "minor" | "patch") {
                    return Err(usage(format!("unknown argument: {word}")));
                }
                level = word;
            }
            level
        };
        let max_tokens = match value("--max-tokens", args.max_tokens) {
            Err(_) => {
                return Err(output::fail(
                    "--max-tokens requires an argument: a numeric token budget",
                    "use '--max-tokens 0' for full output, or '--max-tokens 100000' for a larger approximate budget (default: 24000)",
                ));
            }
            Ok(None) => 24_000,
            Ok(Some(text)) => match text.trim().parse::<i64>() {
                Ok(n) if n < 0 => {
                    return Err(usage(format!(
                        "Invalid value for '--max-tokens': {text} is not in the range x>=0."
                    )));
                }
                Ok(n) if n > 0 && n < 1000 => {
                    return Err(usage("--max-tokens must be 0 or at least 1000".into()));
                }
                Ok(n) => n as usize,
                Err(_) => {
                    return Err(usage(format!(
                        "Invalid value for '--max-tokens': '{text}' is not a valid integer."
                    )));
                }
            },
        };
        let bots = match value("--bots", args.bots)?.as_deref() {
            None | Some("summary") => Bots::Summary,
            Some("include") => Bots::Include,
            Some("exclude") => Bots::Exclude,
            Some(other) => {
                return Err(usage(format!(
                    "Invalid value for '--bots': '{other}' is not one of 'summary', 'include', 'exclude'."
                )));
            }
        };
        let paths = args
            .paths
            .into_iter()
            .map(|path| value("--path", Some(path)).map(Option::unwrap_or_default))
            .collect::<io::Result<Vec<_>>>()?;
        if !paths.is_empty() && !args.diff {
            return Err(output::fail(
                "--path requires --diff",
                "add --diff to include the focused diff between the release endpoints",
            ));
        }
        Ok(Self {
            level,
            base: value("--base", args.base)?,
            head: value("--head", args.head)?,
            version: value("--version", args.version)?,
            previous_release: value("--previous-release", args.previous_release)?,
            bots,
            request_timeout: seconds("--request-timeout", args.request_timeout, 10.0)?,
            github_timeout: seconds("--github-timeout", args.github_timeout, 60.0)?,
            max_tokens,
            diff: args.diff,
            paths,
        })
    }
}

/// Manifests appended by `--project-context` when present in the invocation directory.
const CORE_MANIFESTS: [&str; 6] = [
    "README.md",
    "pyproject.toml",
    "package.json",
    "requirements.txt",
    "CONTRIBUTING.md",
    "Cargo.toml",
];

/// The current checkout: six-level tree, Git status, and 500-line manifest excerpts.
fn project_context(cwd: &Path) -> io::Result<String> {
    let mut text = format!(
        "# Repository Tree\n```text\n{}\n```\n\n",
        crate::tree::render()?
    );
    if let Some(repo) = git::open(cwd).map_err(io::Error::other)? {
        let (branch, commit) = git::head(&repo).map_err(io::Error::other)?;
        let status = git::porcelain(&repo).map_err(io::Error::other)?;
        text.push_str(&format!(
            "# Git Info\n```text\nBranch: {branch}\nCommit: {commit}\n\nModified files:\n{status}```\n\n"
        ));
    }
    for name in CORE_MANIFESTS {
        let path = Path::new(name);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes);
        let excerpt: String = content.split_inclusive('\n').take(500).collect();
        let language = match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("toml") => "toml",
            Some("json") => "json",
            Some("md") => "markdown",
            Some("yaml" | "yml") => "yaml",
            _ => "text",
        };
        text.push_str(&format!(
            "# {name}\n{}\n\n",
            fenced(excerpt.trim_end_matches('\n'), language)
        ));
    }
    Ok(text)
}

pub fn run(args: Args) -> io::Result<()> {
    let project = args.project_context;
    let options = Options::from_args(args)?;
    let cwd = std::env::current_dir()?;
    let repo = git::open(&cwd).map_err(io::Error::other)?.ok_or_else(|| {
        output::fail(
            "not a git repository",
            "run 'focal release-context' inside a Git checkout, or pass --repo <url>",
        )
    })?;
    let report = report::render(&repo, &cwd, &options)?;
    let mut formatted = limit::limit(&report, options.max_tokens)
        .trim_end_matches('\n')
        .to_owned();
    if project {
        formatted.push_str("\n# Project Background (current checkout, not the release snapshot)\n\nTree is limited to six levels; core files are limited to their first 500 lines.\n\n");
        formatted.push_str(project_context(&cwd)?.trim_end_matches('\n'));
        // The background shares the output budget with the release evidence.
        formatted = limit::limit(&formatted, options.max_tokens)
            .trim_end_matches('\n')
            .to_owned();
    }
    // The report records immutable release endpoints; omit unrelated checkout metadata.
    output::deliver(output::bare() + &formatted, "Copied release context")
}
