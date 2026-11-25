/// Diagnostics never fail the command: unlike the std macros, these ignore write errors
/// such as a closed stderr pipe (`focal files --all 2>&1 | head -1`), which would otherwise
/// panic and lose the payload. Declared before the modules so it shadows the std macros.
macro_rules! eprintln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

macro_rules! eprint {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stderr(), $($arg)*);
    }};
}

mod ci_fail;
mod cli;
mod clipboard;
mod completions;
mod context;
mod diff;
mod discovery;
mod file;
mod git;
mod github;
mod issues;
mod local;
mod notebook;
mod output;
mod pdf;
mod picker;
mod pr_diff;
mod preview;
mod release;
mod remote;
mod search;
mod tree;
mod web;
mod wip;

use cli::{Action, Route};
use local::Selection;
use std::io::{self, Write};
use std::process::ExitCode;

fn dispatch(action: Action) -> io::Result<ExitCode> {
    let result = match action {
        Action::Files {
            all,
            staged,
            modified,
            paths,
        } => local::files(if all {
            Selection::All
        } else if staged {
            Selection::Staged
        } else if modified {
            Selection::Modified
        } else if paths.is_empty() {
            Selection::Pick
        } else {
            Selection::Paths(paths)
        }),
        Action::Tree => local::tree(),
        Action::Search { pattern } => local::search_command(pattern),
        Action::Symbol { pattern } => local::symbol(pattern),
        Action::Api => local::api(),
        Action::Diff {
            staged,
            all,
            thick,
            paths,
        } => local::diff_command(
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
        Action::WipContext { base } => local::wip_context(base),
        Action::Context { core, all, paths } => local::context(core, all, paths),
        Action::Web { url } => return local::web_command(url),
        Action::Issues(args) => issues::run(args),
        Action::PrDiff(args) => pr_diff::run(args),
        Action::CiFail(args) => ci_fail::run(args),
        Action::ReleaseContext(args) => release::run(*args),
        Action::Completions(args) => completions::run(args),
    };
    result.map(|()| ExitCode::SUCCESS)
}

fn run() -> io::Result<ExitCode> {
    match cli::route(
        // Lossy, so a non-UTF-8 argument is reported as a missing path instead of a panic.
        std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
    )? {
        Route::Print(text) => {
            io::stdout().lock().write_all(text.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
        Route::Run { repo, cli } => {
            if let Some(url) = repo {
                remote::enter(&url)?;
            }
            dispatch(cli.command)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
