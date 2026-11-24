use crate::file::{self, Noise};
use crate::{discovery, git};
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// Manifests that `context` always includes when present at the invocation directory.
const CORE_MANIFESTS: [&str; 6] = [
    "README.md",
    "pyproject.toml",
    "package.json",
    "requirements.txt",
    "CONTRIBUTING.md",
    "Cargo.toml",
];

pub enum Mode {
    Core,
    All,
}

fn git_info(cwd: &Path) -> Result<String, git2::Error> {
    let Some(repo) = git::open(cwd)? else {
        return Ok(String::new());
    };
    let (branch, commit) = git::head(&repo)?;
    Ok(format!(
        "# Git Info\n```text\nBranch: {branch}\nCommit: {commit}\n\nModified files:\n{}```\n\n",
        git::porcelain(&repo)?
    ))
}

fn append_files(payload: &mut String, paths: &[PathBuf], noise: &Noise) -> io::Result<()> {
    for path in paths {
        let section = file::format(path, noise)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        payload.push_str(&section.markdown);
    }
    Ok(())
}

/// Renders the tree, Git info, core manifests, and requested files in legacy section order.
pub fn render(mode: Mode, paths: Vec<PathBuf>) -> io::Result<String> {
    let core: Vec<PathBuf> = CORE_MANIFESTS
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect();
    let selected = discovery::resolve(paths)?;
    let extra: Vec<PathBuf> = match mode {
        Mode::Core => selected,
        Mode::All => {
            eprintln!("info: Discovering all valid files...");
            discovery::walk(Path::new("."))?
        }
    }
    .into_iter()
    .filter(|path| !core.contains(path))
    .collect();

    let cwd = std::env::current_dir()?;
    let mut payload = String::new();
    write!(
        payload,
        "# Repository Tree\n```text\n{}\n```\n\n",
        crate::tree::render()?
    )
    .unwrap();
    payload.push_str(&git_info(&cwd).map_err(io::Error::other)?);
    let noise = Noise::load();
    append_files(&mut payload, &core, &noise)?;
    append_files(&mut payload, &extra, &noise)?;
    Ok(payload)
}
