//! Failed-step log assembly and filtering.
//!
//! [`associate`] reproduces how `gh run view --log-failed` maps entries of the run logs
//! archive onto failed jobs and steps, [`gh_lines`] reproduces its `job<TAB>step<TAB>line`
//! output, and [`filter_logs`] is a line-for-line port of `focal/gh_ci_fail.py`, which
//! consumed that output.

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize, Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Step {
    pub number: u64,
    pub name: String,
    #[serde(default)]
    pub conclusion: Option<String>,
}

/// GitHub CLI's `IsFailureState`: cancelled and skipped do not count.
pub fn is_failure(conclusion: Option<&str>) -> bool {
    matches!(
        conclusion,
        Some("action_required" | "failure" | "startup_failure" | "timed_out")
    )
}

/// Where a segment's log text comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// An archive entry, by index.
    Entry(usize),
    /// The job logs endpoint, used when the archive has no log for the job at all.
    Job(u64),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    pub job: String,
    pub step: String,
    pub source: Source,
}

/// The archive's spelling of a job name: `/` and `:` removed, cut to 90 UTF-16 units,
/// then trimmed.
fn archive_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !matches!(c, '/' | ':')).collect();
    let units: Vec<u16> = cleaned.encode_utf16().collect();
    let cut = if units.len() > 90 {
        String::from_utf16_lossy(&units[..90])
    } else {
        cleaned
    };
    cut.trim().to_owned()
}

fn first_match(names: &[String], pattern: &str) -> Option<usize> {
    let re = Regex::new(pattern).ok()?;
    names.iter().position(|name| re.is_match(name))
}

/// Maps failed jobs and steps onto archive entries the way GitHub CLI does. A job whose
/// steps have per-step entries contributes its failed steps' entries (failed steps without
/// an entry are dropped). A job without any per-step entry contributes its whole job log,
/// from the archive when present and otherwise from the job logs endpoint.
pub fn associate(names: &[String], jobs: &[Job]) -> Vec<Segment> {
    let mut segments = Vec::new();
    for job in jobs {
        if !is_failure(job.conclusion.as_deref()) {
            continue;
        }
        let name = regex::escape(&archive_name(&job.name));
        let step_entry =
            |step: &Step| first_match(names, &format!(r"^{name}/{}_.*\.txt$", step.number));
        if job.steps.iter().any(|step| step_entry(step).is_some()) {
            let mut steps = job.steps.clone();
            steps.sort_by_key(|step| step.number);
            for step in steps {
                if !is_failure(step.conclusion.as_deref()) {
                    continue;
                }
                if let Some(index) = step_entry(&step) {
                    segments.push(Segment {
                        job: job.name.clone(),
                        step: step.name.clone(),
                        source: Source::Entry(index),
                    });
                }
            }
            continue;
        }
        let entry = first_match(names, &format!(r"^\d+_{name}\.txt$"))
            .or_else(|| first_match(names, &format!(r"^-\d+_{name}\.txt$")));
        segments.push(Segment {
            job: job.name.clone(),
            step: "UNKNOWN STEP".into(),
            source: entry.map_or(Source::Job(job.id), Source::Entry),
        });
    }
    segments
}

/// Appends a log as GitHub CLI prints it: one `job<TAB>step<TAB>line` per line, with the
/// line ending (and a carriage return before it) removed. The UTF-8 byte order mark that
/// opens GitHub's log files is dropped so the first line's timestamp is recognized.
pub fn gh_lines(out: &mut String, job: &str, step: &str, log: &[u8]) {
    let text = String::from_utf8_lossy(log);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        out.push_str(job);
        out.push('\t');
        out.push_str(step);
        out.push('\t');
        out.push_str(line);
        out.push('\n');
    }
}

const ERROR_PATTERNS: [&str; 13] = [
    "##[error]",
    "FAILED ",
    "FAILED:",
    "FAIL ",
    "FAIL:",
    "Traceback (most recent call last):",
    "Error: ",
    "error: ",
    "Exception: ",
    "ERR! ",
    "exit status 1",
    "exit code 1",
    "not ok ",
];

const IGNORED: [&str; 5] = [
    "Download action repository",
    "Secret source: Actions",
    "Prepare workflow directory",
    "Getting action download info",
    "Complete job name:",
];

fn has_error(line: &str) -> bool {
    ERROR_PATTERNS.iter().any(|marker| line.contains(marker))
}

/// Python's `str.splitlines()`, which also breaks on vertical tab, form feed, the
/// information separators, NEL, and the Unicode line and paragraph separators.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let breaks = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !breaks {
            continue;
        }
        lines.push(&text[start..index]);
        start = index + c.len_utf8();
        if c == '\r'
            && let Some(&(next, '\n')) = chars.peek()
        {
            chars.next();
            start = next + 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[^\t\n]*\t){0,2}\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\s?").unwrap()
});
static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap());

/// Strips the job, step, and timestamp prefix and ANSI escapes, collapses successful
/// `##[group]` blocks, and keeps 15 lines before and 10 after each error line, joining
/// gaps with `...`. Without any error line, every processed line is kept.
pub fn filter_logs(raw: &str) -> String {
    let mut processed: Vec<String> = Vec::new();
    let mut in_group = false;
    let mut group_name = String::new();
    let mut group: Vec<String> = Vec::new();
    let mut group_has_error = false;

    for line in split_lines(raw) {
        let clean = PREFIX.replace(line, "");
        let clean = ANSI.replace_all(&clean, "").into_owned();
        if IGNORED.iter().any(|ignored| clean.contains(ignored)) {
            continue;
        }
        if let Some(name) = clean.strip_prefix("##[group]") {
            in_group = true;
            group_name = name.to_owned();
            group = vec![clean];
            group_has_error = false;
        } else if clean.starts_with("##[endgroup]") {
            if in_group {
                group.push(clean);
                if group_has_error || group.iter().any(|entry| has_error(entry)) {
                    processed.append(&mut group);
                } else {
                    processed.push(format!("> [Group completed successfully: {group_name}]"));
                }
                in_group = false;
                group.clear();
            } else {
                processed.push(clean);
            }
        } else if in_group {
            if has_error(&clean) {
                group_has_error = true;
            }
            group.push(clean);
        } else {
            processed.push(clean);
        }
    }
    if in_group {
        processed.append(&mut group);
    }

    let errors: Vec<usize> = (0..processed.len())
        .filter(|&i| has_error(&processed[i]))
        .collect();
    if errors.is_empty() {
        return processed.join("\n");
    }
    let mut keep = vec![false; processed.len()];
    for index in errors {
        let end = (index + 11).min(processed.len());
        for slot in &mut keep[index.saturating_sub(15)..end] {
            *slot = true;
        }
    }
    let mut lines: Vec<&str> = Vec::new();
    let mut last: Option<usize> = None;
    for (index, line) in processed.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        if last.is_some_and(|last| index > last + 1) {
            lines.push("...");
        }
        lines.push(line);
        last = Some(index);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_like_python() {
        assert_eq!(
            split_lines("a\r\nb\rc\x0cd\n\ne\n"),
            ["a", "b", "c", "d", "", "e"]
        );
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("x\u{2028}y"), ["x", "y"]);
    }

    #[test]
    fn strips_prefixes_and_ansi() {
        let raw = "build\tRun tests\t2026-08-10T19:09:40.6560982Z \x1b[31mboom\x1b[0m\n\
                   build\tRun tests\tno timestamp here\n\
                   2026-08-10T19:09:40Z bare 2026-08-10T19:09:40Z kept";
        assert_eq!(
            filter_logs(raw),
            "boom\nbuild\tRun tests\tno timestamp here\nbare 2026-08-10T19:09:40Z kept"
        );
    }

    #[test]
    fn collapses_successful_groups_and_keeps_failing_ones() {
        let raw = "##[group]Install\nok\n##[endgroup]\n##[group]Test\nError: bad\n##[endgroup]\n##[group]Open\nleft";
        assert_eq!(
            filter_logs(raw),
            "> [Group completed successfully: Install]\n##[group]Test\nError: bad\n##[endgroup]\n##[group]Open\nleft"
        );
    }

    #[test]
    fn windows_errors_with_gaps() {
        let mut lines: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
        lines[20] = "error: first".into();
        lines[50] = "##[error]second".into();
        let out = filter_logs(&lines.join("\n"));
        let kept: Vec<&str> = out.lines().collect();
        assert_eq!(kept[0], "line 5");
        assert_eq!(kept[15], "error: first");
        assert_eq!(kept[25], "line 30");
        assert_eq!(kept[26], "...");
        assert_eq!(kept[27], "line 35");
        assert_eq!(*kept.last().unwrap(), "line 59");
    }

    fn job(name: &str, conclusion: &str, steps: &[(u64, &str, &str)]) -> Job {
        Job {
            id: 7,
            name: name.into(),
            conclusion: Some(conclusion.into()),
            steps: steps
                .iter()
                .map(|&(number, name, conclusion)| Step {
                    number,
                    name: name.into(),
                    conclusion: Some(conclusion.into()),
                })
                .collect(),
        }
    }

    #[test]
    fn associates_failed_steps_and_falls_back_to_job_logs() {
        let names: Vec<String> = [
            "0_lint.txt",
            "1_test 3.12.txt",
            "test 3.12/1_Set up job.txt",
            "test 3.12/11_Other.txt",
            "test 3.12/2_Run tests.txt",
            "lint/1_Set up job.txt",
            "2_deploy prod.txt",
        ]
        .map(String::from)
        .to_vec();
        let jobs = [
            job("lint", "success", &[(1, "Set up job", "success")]),
            job(
                "test 3.12",
                "failure",
                &[
                    (2, "Run tests", "failure"),
                    (1, "Set up job", "success"),
                    (3, "Gone", "failure"),
                ],
            ),
            job("deploy: prod", "failure", &[(1, "Ship", "failure")]),
            job("docs", "timed_out", &[]),
            job("cancelled", "cancelled", &[]),
        ];
        assert_eq!(
            associate(&names, &jobs),
            [
                Segment {
                    job: "test 3.12".into(),
                    step: "Run tests".into(),
                    source: Source::Entry(4)
                },
                Segment {
                    job: "deploy: prod".into(),
                    step: "UNKNOWN STEP".into(),
                    source: Source::Entry(6)
                },
                Segment {
                    job: "docs".into(),
                    step: "UNKNOWN STEP".into(),
                    source: Source::Job(7)
                },
            ]
        );
    }

    #[test]
    fn archive_names_match_github() {
        assert_eq!(archive_name("build / test: unit "), "build  test unit");
        assert_eq!(archive_name(&"x".repeat(95)).len(), 90);
    }

    #[test]
    fn formats_like_gh() {
        let mut out = String::new();
        gh_lines(&mut out, "j", "s", "\u{feff}one\r\n\ntwo".as_bytes());
        assert_eq!(out, "j\ts\tone\nj\ts\t\nj\ts\ttwo\n");
    }
}
