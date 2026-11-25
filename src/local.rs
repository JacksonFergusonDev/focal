//! Handlers for the commands that run entirely on local files and Git data.

use crate::file::{self, Noise, Outcome};
use crate::output::{deliver, header};
use crate::{context, diff, discovery, git, output, search, tree, web, wip};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub enum Selection {
    All,
    Staged,
    Modified,
    Paths(Vec<PathBuf>),
    Pick,
}

pub fn files(selection: Selection) -> io::Result<()> {
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
        Selection::Pick => crate::picker::pick_paths(
            discovery::walk(Path::new("."))?,
            "Files: ",
            vec!["TAB: multi-select | CTRL-A: select all | ENTER: confirm".into()],
        )?
        .unwrap_or_default(),
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
        let section = file::section(path, &noise);
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
        eprintln!("{label} {}", crate::file::shown(path));
    }
    let what = match last {
        Some(path) if added + truncated + empty == 1 => {
            format!("{} to clipboard", crate::file::shown(path))
        }
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

pub fn tree() -> io::Result<()> {
    let mut payload = header()?;
    payload.push_str(&tree::render()?);
    deliver(payload, "Repository tree copied to clipboard")
}

pub fn context(core: bool, all: bool, paths: Vec<PathBuf>) -> io::Result<()> {
    let mode = match (core, all) {
        (_, true) => context::Mode::All,
        (true, _) => context::Mode::Core,
        _ => context::Mode::Pick,
    };
    let mut payload = header()?;
    payload.push_str(&context::render(mode, paths)?);
    deliver(payload, "Context and files copied to clipboard")
}

fn usage(missing: &str, usage: &str) -> io::Error {
    io::Error::other(format!("{missing}\n  hint: usage: {usage}"))
}

pub fn search_command(pattern: Option<String>) -> io::Result<()> {
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

pub fn symbol(pattern: Option<String>) -> io::Result<()> {
    let pattern = pattern.ok_or_else(|| usage("missing symbol name", "focal symbol <symbol>"))?;
    let paths = search::files_matching(&search::matcher(&pattern)?)?;
    if paths.is_empty() {
        eprintln!("info: No files found containing symbol '{pattern}'.");
        return Ok(());
    }
    let noise = Noise::load();
    let mut payload = header()?;
    for path in &paths {
        let section = file::section(path, &noise);
        payload.push_str(section.markdown.trim_end_matches('\n'));
        payload.push_str("\n\n");
    }
    payload.truncate(payload.trim_end_matches('\n').len());
    deliver(
        payload,
        &format!("Symbol context for '{pattern}' copied to clipboard"),
    )
}

pub fn api() -> io::Result<()> {
    let found = search::api()?;
    if found.is_empty() {
        eprintln!("info: No Python classes or functions found.");
        return Ok(());
    }
    let mut payload = header()?;
    payload.push_str(found.trim_end_matches('\n'));
    deliver(payload, "API overview copied to clipboard")
}

pub fn diff_command(mode: diff::Mode, thick: bool, paths: Vec<String>) -> io::Result<()> {
    let (body, target) = diff::render(mode, thick, paths)?;
    let Some(body) = body else {
        eprintln!("info: No changes detected in {target}.");
        return Ok(());
    };
    let mut payload = header()?;
    payload.push_str(&body);
    deliver(payload, &format!("Copied diff of {target}"))
}

pub fn wip_context(base: Option<String>) -> io::Result<()> {
    let mut payload = header()?;
    payload.push_str(&wip::render(base.as_deref())?);
    deliver(payload, "Copied WIP branch context")
}

pub fn web_command(url: Option<String>) -> io::Result<ExitCode> {
    let Some((body, message)) = web::render(url.as_deref())? else {
        println!("{}", web::HELP);
        return Ok(ExitCode::FAILURE);
    };
    // Web output never carries the workspace header.
    deliver(format!("{}{body}", output::bare()), &message)?;
    Ok(ExitCode::SUCCESS)
}
