//! Shared regex search over ripgrep's default file set, with per-command formatters.

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch,
};
use ignore::WalkBuilder;
use ignore::types::TypesBuilder;
use std::io;
use std::path::{Path, PathBuf};

const CONTEXT_LINES: usize = 5;

/// Compiles a pattern with ripgrep's default regex settings.
pub fn matcher(pattern: &str) -> io::Result<RegexMatcher> {
    RegexMatcherBuilder::new()
        .line_terminator(Some(b'\n'))
        .build(pattern)
        .map_err(|error| io::Error::other(format!("invalid regex '{pattern}': {error}")))
}

/// Files ripgrep would search from `.`: ignore rules apply and hidden entries are skipped.
fn candidates(python_only: bool) -> io::Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(".");
    builder
        .add_custom_ignore_filename(".rgignore")
        .sort_by_file_path(Path::cmp);
    if python_only {
        let mut types = TypesBuilder::new();
        types.add_defaults();
        types.select("py");
        builder.types(types.build().map_err(io::Error::other)?);
    }
    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("warning: {error}");
                continue;
            }
        };
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

fn searcher(context: usize) -> Searcher {
    SearcherBuilder::new()
        .line_number(false)
        .binary_detection(BinaryDetection::quit(0))
        .before_context(context)
        .after_context(context)
        .build()
}

/// Writes ripgrep-style `path:line` and `path-line` records with `--` group separators.
struct Lines<'a> {
    path: String,
    out: &'a mut String,
    started: bool,
    separate_files: bool,
}

impl Lines<'_> {
    fn push(&mut self, separator: char, bytes: &[u8]) {
        if !self.started {
            if self.separate_files && !self.out.is_empty() {
                self.out.push_str("--\n");
            }
            self.started = true;
        }
        let text = String::from_utf8_lossy(bytes);
        for line in text.strip_suffix('\n').unwrap_or(&text).split('\n') {
            self.out.push_str(&self.path);
            self.out.push(separator);
            self.out.push_str(line);
            self.out.push('\n');
        }
    }
}

impl Sink for Lines<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> io::Result<bool> {
        self.push(':', found.bytes());
        Ok(true)
    }

    fn context(&mut self, _: &Searcher, context: &SinkContext<'_>) -> io::Result<bool> {
        if matches!(
            context.kind(),
            SinkContextKind::Before | SinkContextKind::After
        ) {
            self.push('-', context.bytes());
        }
        Ok(true)
    }

    fn context_break(&mut self, _: &Searcher) -> io::Result<bool> {
        self.out.push_str("--\n");
        Ok(true)
    }
}

/// Stops at the first match, recording that the file matched.
struct Any(bool);

impl Sink for Any {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, _: &SinkMatch<'_>) -> io::Result<bool> {
        self.0 = true;
        Ok(false)
    }
}

fn lines(matcher: &RegexMatcher, python_only: bool, context: usize) -> io::Result<String> {
    let mut searcher = searcher(context);
    let mut out = String::new();
    for path in candidates(python_only)? {
        let mut sink = Lines {
            path: path.display().to_string(),
            out: &mut out,
            started: false,
            separate_files: context > 0,
        };
        if let Err(error) = searcher.search_path(matcher, &path, &mut sink) {
            eprintln!("warning: {}: {error}", path.display());
        }
    }
    Ok(out)
}

/// Matching lines with five lines of surrounding context, as `rg --context 5`.
pub fn search(matcher: &RegexMatcher) -> io::Result<String> {
    lines(matcher, false, CONTEXT_LINES)
}

/// Column-zero top-level Python class and function definitions.
pub fn api() -> io::Result<String> {
    lines(&matcher("^(class|(async )?def) ")?, true, 0)
}

/// Every searched file containing a match anywhere, as `rg -l`.
pub fn files_matching(matcher: &RegexMatcher) -> io::Result<Vec<PathBuf>> {
    let mut searcher = searcher(0);
    let mut found = Vec::new();
    for path in candidates(false)? {
        let mut sink = Any(false);
        if let Err(error) = searcher.search_path(matcher, &path, &mut sink) {
            eprintln!("warning: {}: {error}", path.display());
        }
        if sink.0 {
            found.push(path);
        }
    }
    Ok(found)
}
