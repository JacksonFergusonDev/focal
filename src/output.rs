//! The shared payload envelope and delivery: `<context>` tags, the workspace header,
//! the token estimate, and clipboard or stdout output.

use crate::{clipboard, git};
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::path::Path;

fn append_git_header(text: &mut String, cwd: &Path) -> Result<(), git2::Error> {
    if let Some(repo) = git::open(cwd)? {
        let (branch, commit) = git::head(&repo)?;
        writeln!(text, "Git Branch: {branch}\nGit Commit: {commit}").unwrap();
    }
    Ok(())
}

/// Opens a payload: `<context>` plus the workspace note, PWD, and Git metadata unless
/// `FOCAL_OMIT_WORKSPACE` is set.
pub fn header() -> io::Result<String> {
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

/// Opens a payload without the workspace header (web and release output).
pub fn bare() -> String {
    String::from("<context>\n")
}

/// Closes the payload, then writes it to stdout when redirected; otherwise copies it to
/// the clipboard, falling back to stdout with the reason when the clipboard is unavailable.
pub fn deliver(mut payload: String, message: &str) -> io::Result<()> {
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

/// An error with an actionable hint line, rendered as `error: …\n  hint: …`.
pub fn fail(message: impl Into<String>, hint: &str) -> io::Error {
    io::Error::other(format!("{}\n  hint: {hint}", message.into()))
}
