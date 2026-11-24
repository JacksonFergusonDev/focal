mod discovery;
mod file;
mod notebook;
mod tree;

use clap::{Parser, Subcommand};
use file::{Noise, Outcome};
use git2::{ErrorCode, Repository};
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
    /// Extract named files, directory contents (dir/), or all discovered files.
    Files {
        #[arg(short, long, conflicts_with = "paths")]
        all: bool,
        #[arg(required_unless_present = "all", value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

fn append_git_header(text: &mut String, cwd: &Path) -> Result<(), git2::Error> {
    let repo = match Repository::open_from_env() {
        Ok(repo) => repo,
        Err(error) if error.code() == ErrorCode::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let Some(workdir) = repo.workdir() else {
        return Ok(());
    };
    if !cwd.starts_with(workdir) || cwd.starts_with(repo.path()) {
        return Ok(());
    }
    let reference = repo.find_reference("HEAD")?;
    let branch = reference
        .symbolic_target()
        .and_then(|name| name.strip_prefix("refs/heads/"))
        .unwrap_or("");
    let commit = match repo.head() {
        Ok(head) => head
            .peel_to_commit()?
            .as_object()
            .short_id()?
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        Err(error) if error.code() == ErrorCode::UnbornBranch => "none".into(),
        Err(error) => return Err(error),
    };
    writeln!(text, "Git Branch: {branch}\nGit Commit: {commit}").unwrap();
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

fn files(all: bool, paths: Vec<PathBuf>) -> io::Result<()> {
    let paths = if all {
        eprintln!("info: Discovering all valid files...");
        discovery::walk(Path::new("."))?
    } else {
        discovery::resolve(paths)?
    };
    if paths.is_empty() {
        eprintln!("info: No files selected.");
        return Ok(());
    }

    let noise = Noise::load();
    let mut payload = header()?;
    let (mut added, mut empty, mut skipped, mut truncated) = (0, 0, 0, 0);
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
        };
        if !matches!(section.outcome, Outcome::Skipped) {
            last = Some(path);
        }
        eprintln!("{label} {}", path.display());
    }
    let what = match last {
        Some(path) if added + truncated + empty == 1 => format!("{} to clipboard", path.display()),
        _ => "context".to_owned(),
    };
    deliver(payload, &format!("Copied {what}"))?;
    if empty + skipped + truncated > 0 {
        eprintln!("\n--- Extraction Summary ---\nAdded fully: {added} files");
        for (label, count) in [
            ("Added empty", empty),
            ("Truncated (size limit)", truncated),
            ("Skipped (noise/binary)", skipped),
        ] {
            if count > 0 {
                eprintln!("{label}: {count} files");
            }
        }
    }
    Ok(())
}

fn deliver(mut payload: String, message: &str) -> io::Result<()> {
    payload.push_str("\n</context>\n");
    if io::stdout().is_terminal() {
        eprintln!("warning: Rust preview writes to stdout; clipboard support is pending.");
    }
    io::stdout().lock().write_all(payload.as_bytes())?;
    eprintln!("done: {message} (~{} tokens)", payload.chars().count() / 4);
    Ok(())
}

fn tree() -> io::Result<()> {
    let mut payload = header()?;
    payload.push_str(&tree::render()?);
    deliver(payload, "Repository tree copied to clipboard")
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Action::Files { all, paths } => files(all, paths),
        Action::Tree => tree(),
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
