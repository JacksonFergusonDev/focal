mod clipboard;
mod context;
mod diff;
mod discovery;
mod file;
mod git;
mod notebook;
mod pdf;
mod search;
mod tree;
mod web;
mod wip;

use clap::{Parser, Subcommand};
use file::{Noise, Outcome};
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(about = "Focal Rust migration preview")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
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
        #[arg(
            required_unless_present = "mode",
            conflicts_with = "mode",
            value_name = "PATH"
        )]
        paths: Vec<PathBuf>,
    },
}

fn append_git_header(text: &mut String, cwd: &Path) -> Result<(), git2::Error> {
    if let Some(repo) = git::open(cwd)? {
        let (branch, commit) = git::head(&repo)?;
        writeln!(text, "Git Branch: {branch}\nGit Commit: {commit}").unwrap();
    }
    Ok(())
}

fn header() -> io::Result<String> {
    let mut text = String::from("<context>\n");
    if std::env::var_os("FOCAL_OMIT_WORKSPACE").is_none_or(|value| value.is_empty()) {
        text.push_str("> [!NOTE]\n> The following is an automated extraction of the user's local codebase environment.\n> Use this metadata, directory structure, and file contents to inform your response to the user's subsequent prompt.\n\n# Workspace Context\n");
        let cwd = std::env::current_dir()?;
        writeln!(text, "PWD: {}", cwd.display()).unwrap();
        append_git_header(&mut text, &cwd).map_err(io::Error::other)?;
        text.push_str("\n---\n\n");
    }
    Ok(text)
}

enum Selection {
    All,
    Staged,
    Modified,
    Paths(Vec<PathBuf>),
}

fn files(selection: Selection) -> io::Result<()> {
    let git_paths = |kind: &str, select: fn(&Path) -> io::Result<Vec<PathBuf>>| {
        eprintln!("info: Discovering {kind} files...");
        let mut paths = select(&std::env::current_dir()?)?;
        paths.retain(|path| path.is_file());
        io::Result::Ok(paths)
    };
    let paths = match selection {
        Selection::All => {
            eprintln!("info: Discovering all valid files...");
            discovery::walk(Path::new("."))?
        }
        Selection::Staged => git_paths("staged", git::staged)?,
        Selection::Modified => git_paths("modified", git::modified)?,
        Selection::Paths(paths) => discovery::resolve(paths)?,
    };
    if paths.is_empty() {
        eprintln!("info: No files selected.");
        return Ok(());
    }

    let noise = Noise::load();
    let mut payload = header()?;
    let (mut added, mut empty, mut skipped, mut truncated, mut failed) = (0, 0, 0, 0, 0);
    let mut last = None;
    for path in &paths {
        let section = file::format(path, &noise)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        payload.push_str(&section.markdown);
        let label = match section.outcome {
            Outcome::Empty => {
                empty += 1;
                "added: (empty):"
            }
            Outcome::Text => {
                added += 1;
                "added:"
            }
            Outcome::Truncated => {
                truncated += 1;
                "added: (truncated):"
            }
            Outcome::Skipped => {
                skipped += 1;
                "skipped: noise/binary:"
            }
            Outcome::Failed => {
                failed += 1;
                "failed: (no text extracted):"
            }
        };
        if !matches!(section.outcome, Outcome::Skipped | Outcome::Failed) {
            last = Some(path);
        }
        eprintln!("{label} {}", path.display());
    }
    let what = match last {
        Some(path) if added + truncated + empty == 1 => format!("{} to clipboard", path.display()),
        _ => "context".to_owned(),
    };
    deliver(payload, &format!("Copied {what}"))?;
    if empty + skipped + truncated + failed > 0 {
        eprintln!("\n--- Extraction Summary ---\nAdded fully: {added} files");
        for (label, count) in [
            ("Added empty", empty),
            ("Truncated (size limit)", truncated),
            ("Skipped (noise/binary)", skipped),
            ("Failed (no text extracted)", failed),
        ] {
            if count > 0 {
                eprintln!("{label}: {count} files");
            }
        }
    }
    Ok(())
}

/// Writes the payload to stdout when it is redirected; otherwise copies it to the
/// clipboard, falling back to stdout with the reason when the clipboard is unavailable.
fn deliver(mut payload: String, message: &str) -> io::Result<()> {
    payload.push_str("\n</context>\n");
    let done = format!("done: {message} (~{} tokens)", payload.chars().count() / 4);
    if io::stdout().is_terminal() {
        match clipboard::copy(&payload) {
            Ok(()) => {
                eprintln!("{done}");
                return Ok(());
            }
            Err(reason) => {
                eprintln!(
                    "warning: Could not copy to the clipboard: {reason}. Dumping to stdout..."
                );
            }
        }
        return io::stdout().lock().write_all(payload.as_bytes());
    }
    io::stdout().lock().write_all(payload.as_bytes())?;
    eprintln!("{done}");
    Ok(())
}

fn tree() -> io::Result<()> {
    let mut payload = header()?;
    payload.push_str(&tree::render()?);
    deliver(payload, "Repository tree copied to clipboard")
}

fn context(core: bool, all: bool, paths: Vec<PathBuf>) -> io::Result<()> {
    let mode = match (core, all) {
        (_, true) => context::Mode::All,
        (true, _) => context::Mode::Core,
        _ => {
            return Err(io::Error::other(
                "the interactive picker is pending in the Rust preview; pass --core or --all",
            ));
        }
    };
    let mut payload = header()?;
    payload.push_str(&context::render(mode, paths)?);
    deliver(payload, "Context and files copied to clipboard")
}

fn usage(missing: &str, usage: &str) -> io::Error {
    io::Error::other(format!("{missing}\n  hint: usage: {usage}"))
}

fn search_command(pattern: Option<String>) -> io::Result<()> {
    let pattern =
        pattern.ok_or_else(|| usage("missing search pattern", "focal search <pattern>"))?;
    let found = search::search(&search::matcher(&pattern)?)?;
    if found.is_empty() {
        eprintln!("info: No matches found for '{pattern}'.");
        return Ok(());
    }
    let mut payload = header()?;
    payload.push_str(found.trim_end_matches('\n'));
    deliver(payload, "Search results copied to clipboard")
}

fn symbol(pattern: Option<String>) -> io::Result<()> {
    let pattern = pattern.ok_or_else(|| usage("missing symbol name", "focal symbol <symbol>"))?;
    let paths = search::files_matching(&search::matcher(&pattern)?)?;
    if paths.is_empty() {
        eprintln!("info: No files found containing symbol '{pattern}'.");
        return Ok(());
    }
    let noise = Noise::load();
    let mut payload = header()?;
    for path in &paths {
        let section = file::format(path, &noise)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        payload.push_str(section.markdown.trim_end_matches('\n'));
        payload.push_str("\n\n");
    }
    payload.truncate(payload.trim_end_matches('\n').len());
    deliver(
        payload,
        &format!("Symbol context for '{pattern}' copied to clipboard"),
    )
}

fn api() -> io::Result<()> {
    let found = search::api()?;
    if found.is_empty() {
        eprintln!("info: No Python classes or functions found.");
        return Ok(());
    }
    let mut payload = header()?;
    payload.push_str(found.trim_end_matches('\n'));
    deliver(payload, "API overview copied to clipboard")
}

fn diff_command(mode: diff::Mode, thick: bool, paths: Vec<String>) -> io::Result<()> {
    let (body, target) = diff::render(mode, thick, paths)?;
    let Some(body) = body else {
        eprintln!("info: No changes detected in {target}.");
        return Ok(());
    };
    let mut payload = header()?;
    payload.push_str(&body);
    deliver(payload, &format!("Copied diff of {target}"))
}

fn wip_context(base: Option<String>) -> io::Result<()> {
    let mut payload = header()?;
    payload.push_str(&wip::render(base.as_deref())?);
    deliver(payload, "Copied WIP branch context")
}

fn web_command(url: Option<String>) -> io::Result<ExitCode> {
    let Some((body, message)) = web::render(url.as_deref())? else {
        println!("{}", web::HELP);
        return Ok(ExitCode::FAILURE);
    };
    // Web output never carries the workspace header.
    deliver(format!("<context>\n{body}"), &message)?;
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Action::Files {
            all,
            staged,
            modified,
            paths,
        } => files(if all {
            Selection::All
        } else if staged {
            Selection::Staged
        } else if modified {
            Selection::Modified
        } else {
            Selection::Paths(paths)
        }),
        Action::Tree => tree(),
        Action::Search { pattern } => search_command(pattern),
        Action::Symbol { pattern } => symbol(pattern),
        Action::Api => api(),
        Action::Diff {
            staged,
            all,
            thick,
            paths,
        } => diff_command(
            if all {
                diff::Mode::All
            } else if staged {
                diff::Mode::Staged
            } else {
                diff::Mode::Unstaged
            },
            thick,
            paths,
        ),
        Action::WipContext { base } => wip_context(base),
        Action::Context { core, all, paths } => context(core, all, paths),
        Action::Web { url } => match web_command(url) {
            Ok(code) => return code,
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
