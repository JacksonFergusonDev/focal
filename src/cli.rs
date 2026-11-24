//! Command-line surface: global flags, the legacy help menu, `--commands`, and the
//! per-command argument definitions.
//!
//! Each command owns its `Args` type in its own module, so new commands register here
//! once and then evolve without touching this file.

use clap::{Parser, Subcommand};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

/// Public commands and their one-line descriptions, in help-menu order.
pub const COMMANDS: [(&str, &str); 13] = [
    (
        "api",
        "Extracts and copies an overview of Python classes and functions.",
    ),
    (
        "ci-fail",
        "Fetches and formats GitHub Actions CI failure logs.",
    ),
    (
        "context",
        "Generates a project context summary and allows selecting additional files.",
    ),
    (
        "diff",
        "Copies a formatted git diff of uncommitted, staged, or all changes.",
    ),
    (
        "files",
        "Copies the contents of one or more files, with metadata.",
    ),
    (
        "issues",
        "Copies GitHub issues alongside their comment threads.",
    ),
    (
        "pr-diff",
        "Copies metadata, intent, and code diff for a GitHub PR.",
    ),
    (
        "release-context",
        "Copies Git release membership, PR intent, and net impact.",
    ),
    (
        "search",
        "Searches the codebase for a regex pattern and copies the results.",
    ),
    (
        "symbol",
        "Locates files containing a specific symbol and copies full contents.",
    ),
    ("tree", "Copies the repository directory tree."),
    (
        "web",
        "Extracts a webpage (via URL or piped HTML) and formats its semantic content.",
    ),
    (
        "wip-context",
        "Copies a comprehensive diff, status, and topological context.",
    ),
];

#[derive(Parser)]
#[command(name = "focal", disable_help_subcommand = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand)]
pub enum Action {
    /// Extract the repository directory tree, up to six levels deep.
    Tree,
    /// Search for a regex and extract matches with five lines of context.
    Search {
        #[arg(allow_hyphen_values = true, value_name = "PATTERN")]
        pattern: Option<String>,
    },
    /// Extract every file containing a regex, in full.
    Symbol {
        #[arg(allow_hyphen_values = true, value_name = "SYMBOL")]
        pattern: Option<String>,
    },
    /// Extract a Git diff of unstaged (default), staged, or all changes.
    Diff {
        /// Diff staged changes only.
        #[arg(long, visible_alias = "cached", overrides_with = "all")]
        staged: bool,
        /// Diff all changes, including untracked files, against HEAD.
        #[arg(short, long, overrides_with = "staged")]
        all: bool,
        /// Append the repository tree and full contents of changed files.
        #[arg(long)]
        thick: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
    },
    /// Extract branch topology, status, and prioritized patches since a base branch.
    WipContext {
        #[arg(value_name = "BASE")]
        base: Option<String>,
    },
    /// Extract top-level Python class and function definitions.
    Api,
    /// Convert a web page, or HTML piped on stdin, to Markdown.
    Web {
        #[arg(value_name = "URL")]
        url: Option<String>,
    },
    /// Extract the tree, Git status, core manifests, and requested files.
    Context {
        /// Extract core context only, plus any named paths.
        #[arg(short, long, overrides_with = "all")]
        core: bool,
        /// Extract core context and append all discovered files.
        #[arg(short, long, overrides_with = "core")]
        all: bool,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Extract named files, directory contents (dir/), or all discovered files.
    Files {
        #[arg(short, long, group = "mode")]
        all: bool,
        /// Extract staged files (excluding deletions) from the working tree.
        #[arg(short, long, group = "mode")]
        staged: bool,
        /// Extract tracked files with unstaged changes under the current directory.
        #[arg(short, long, group = "mode")]
        modified: bool,
        /// Files or directories (dir/) to extract; omit to choose interactively.
        #[arg(conflicts_with = "mode", value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Copy GitHub issues alongside their comment threads.
    Issues(crate::issues::Args),
    /// Copy metadata, intent, and code diff for a GitHub pull request.
    PrDiff(crate::pr_diff::Args),
    /// Fetch and format GitHub Actions failure logs.
    CiFail(crate::ci_fail::Args),
    /// Copy Git release membership, PR intent, and net impact.
    ReleaseContext(Box<crate::release::Args>),
    /// Print a shell completion script (internal).
    #[command(hide = true)]
    Completions(crate::completions::Args),
}

/// What `main` should do after global flags are handled.
pub enum Route {
    /// Run a command, optionally inside the cached checkout of `repo`.
    Run { repo: Option<String>, cli: Box<Cli> },
    /// Print text to stdout and exit successfully.
    Print(String),
}

fn fail(message: &str, hint: &str) -> io::Error {
    io::Error::other(format!("{message}\n  hint: {hint}"))
}

/// Removes `--repo <url>` / `--repo=<url>` from anywhere before a `--`, as the legacy
/// dispatcher does, so the flag works on either side of the subcommand.
pub fn split_repo(args: Vec<String>) -> io::Result<(Option<String>, Vec<String>)> {
    let mut repo = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            rest.push(arg);
            rest.extend(iter.by_ref());
            break;
        }
        if arg == "--repo" {
            match iter.next().filter(|value| !value.is_empty()) {
                Some(value) => repo = Some(value),
                None => {
                    return Err(fail(
                        "--repo requires an argument.",
                        "specify a repository URL, e.g. '--repo https://github.com/org/repo'",
                    ));
                }
            }
        } else if let Some(value) = arg.strip_prefix("--repo=") {
            // An empty value means no remote, as in the legacy dispatcher.
            repo = Some(value.to_owned()).filter(|value| !value.is_empty());
        } else {
            rest.push(arg);
        }
    }
    Ok((repo, rest))
}

fn wrap(text: &str, width: usize, hang: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
        } else if line.chars().count() + 1 + word.chars().count() <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            out.push_str(&line);
            out.push('\n');
            out.push_str(hang);
            line = word.to_owned();
        }
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// The legacy help menu, wrapped to the terminal width (80 columns when not a terminal).
pub fn help() -> String {
    let color = io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |term| term != "dumb");
    let (bold, cyan, reset) = if color {
        ("\x1b[1m", "\x1b[36m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let width = if io::stdout().is_terminal() {
        crossterm::terminal::size().map_or(80, |(cols, _)| cols as usize)
    } else {
        80
    };
    let longest = COMMANDS
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    let column = longest + 3;
    let desc_width = width.saturating_sub(2 + column).max(20);
    let hang = " ".repeat(2 + column);
    let mut out = format!("{bold}Usage:{reset} focal [global flags] <command> [args]\n\n");
    out.push_str(&format!("{bold}Global flags:{reset}\n"));
    out.push_str(&format!(
        "  {:<width$} {}\n",
        "--repo <url>",
        "Run the command on a remote GitHub repository (caches locally)",
        width = column - 1
    ));
    out.push_str(&format!("\n{bold}Available commands:{reset}\n"));
    for (name, desc) in COMMANDS {
        out.push_str(&format!("  {cyan}{bold}{name:<column$}{reset}"));
        out.push_str(&wrap(desc, desc_width, &hang));
    }
    out
}

/// Applies the dispatcher rules before clap parses the subcommand.
pub fn route(args: Vec<String>) -> io::Result<Route> {
    let (repo, rest) = split_repo(args)?;
    match rest.first().map(String::as_str) {
        Some("--commands") => {
            let mut names: Vec<&str> = COMMANDS.iter().map(|(name, _)| *name).collect();
            names.sort_unstable();
            return Ok(Route::Print(names.join("\n") + "\n"));
        }
        None | Some("help" | "-h" | "--help") => return Ok(Route::Print(help())),
        Some(name)
            if name != "--"
                && name != "completions"
                && !COMMANDS.iter().any(|(command, _)| *command == name) =>
        {
            return Err(fail(
                &format!("unknown command '{name}'"),
                "run 'focal help' for a list of available commands",
            ));
        }
        _ => {}
    }
    let mut argv = vec!["focal".to_owned()];
    argv.extend(rest.into_iter().skip_while(|arg| arg == "--"));
    let cli = Box::new(Cli::try_parse_from(argv).unwrap_or_else(|error| error.exit()));
    Ok(Route::Run { repo, cli })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn repo_flag_is_taken_from_either_side_of_the_command() {
        let (repo, rest) = split_repo(strings(&["--repo", "u1", "tree"])).unwrap();
        assert_eq!((repo.as_deref(), rest), (Some("u1"), strings(&["tree"])));
        let (repo, rest) = split_repo(strings(&["files", "--repo=u2", "-a"])).unwrap();
        assert_eq!(
            (repo.as_deref(), rest),
            (Some("u2"), strings(&["files", "-a"]))
        );
    }

    #[test]
    fn tokens_after_double_dash_pass_through() {
        let (repo, rest) = split_repo(strings(&["search", "--", "--repo", "x"])).unwrap();
        assert_eq!(repo, None);
        assert_eq!(rest, strings(&["search", "--", "--repo", "x"]));
    }

    #[test]
    fn empty_repo_value_runs_locally() {
        let (repo, rest) = split_repo(strings(&["--repo=", "tree"])).unwrap();
        assert_eq!((repo, rest), (None, strings(&["tree"])));
    }

    #[test]
    fn repo_flag_requires_a_value() {
        let error = split_repo(strings(&["tree", "--repo"])).err().unwrap();
        assert!(
            error
                .to_string()
                .starts_with("--repo requires an argument.")
        );
    }

    #[test]
    fn help_lists_every_command() {
        let text = help();
        assert!(text.starts_with("Usage: focal [global flags] <command> [args]\n"));
        for (name, _) in COMMANDS {
            assert!(text.contains(&format!("\n  {name} ")), "{name}");
        }
        // Command descriptions wrap at 80 columns; the --repo line is unwrapped, as in legacy.
        assert!(
            text.lines()
                .filter(|line| !line.contains("--repo"))
                .all(|line| line.chars().count() <= 80)
        );
    }

    #[test]
    fn every_public_command_parses() {
        for (name, _) in COMMANDS {
            let parsed = Cli::try_parse_from(["focal", name, "--help"]);
            let error = parsed.err().expect("--help short-circuits parsing");
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp, "{name}");
        }
    }
}
