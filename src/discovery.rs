use ignore::WalkBuilder;
use std::collections::HashSet;
use std::env;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub fn walk(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .add_custom_ignore_filename(".fdignore")
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != ".git")
        .sort_by_file_path(Path::cmp);
    let config_dir = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    if let Some(config) = config_dir {
        let global = config.join("fd/ignore");
        if global.is_file()
            && let Some(error) = builder.add_ignore(global)
        {
            eprintln!("warning: {error}");
        }
    }
    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(io::Error::other)?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            let path = entry.into_path();
            files.push(path.strip_prefix(".").unwrap_or(&path).to_owned());
        }
    }
    Ok(files)
}

pub fn resolve(paths: Vec<PathBuf>) -> io::Result<Vec<PathBuf>> {
    let mut selected = Vec::new();
    for path in paths {
        if path.as_os_str().as_bytes().ends_with(b"/") {
            if !path.is_dir() {
                return Err(io::Error::other(format!(
                    "directory not found: {}",
                    path.display()
                )));
            }
            selected.extend(walk(&path)?);
        } else if path.is_file() {
            selected.push(path);
        } else if path.is_dir() {
            return Err(io::Error::other(format!(
                "'{}' is a directory; append a trailing slash to select its contents",
                path.display()
            )));
        } else {
            return Err(io::Error::other(format!(
                "file not found: {}",
                path.display()
            )));
        }
    }
    let mut seen = HashSet::new();
    selected.retain(|path| seen.insert(path.clone()));
    Ok(selected)
}
