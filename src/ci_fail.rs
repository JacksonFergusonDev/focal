//! `ci-fail`: pick a failed GitHub Actions run and copy its failed-step logs.
//!
//! Owned by migration track B. Reference: libexec/ci-fail and focal/gh_ci_fail.py.
//!
//! The legacy command read `gh run view --log-failed`. This port lists runs, jobs, and
//! steps through the REST API, downloads the run logs archive (following the redirect to
//! the archive host without credentials), rebuilds GitHub CLI's `job<TAB>step<TAB>line`
//! output for failed steps, and filters it exactly as the Python module did.

mod logs;

use crate::github::{ApiError, Client};
use crate::{file, output, picker};
use logs::{Job, Source};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Cursor, Read};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Runs offered in the picker, as `gh run list --limit 30`.
const LIST_LIMIT: usize = 30;
/// Upper bound on a downloaded log archive or job log.
const DOWNLOAD_LIMIT: u64 = 1024 * 1024 * 1024;
/// Upper bound on decompressed failed-step logs, so a highly compressible archive cannot
/// exhaust memory.
const MAX_LOG_BYTES: u64 = 256 * 1024 * 1024;
/// GitHub CLI's cap on per-job log requests when the archive lacks a job's log.
const MAX_JOB_LOG_REQUESTS: usize = 25;

#[derive(clap::Args)]
pub struct Args {
    /// Show all historical failed runs (bypasses the 24-hour filter).
    #[arg(short, long)]
    pub all: bool,
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
struct Run {
    id: u64,
    name: Option<String>,
    display_title: Option<String>,
    head_branch: Option<String>,
    head_sha: Option<String>,
    event: Option<String>,
    created_at: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: Option<String>,
}

#[derive(Deserialize)]
struct RunList {
    #[serde(default)]
    workflow_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct JobList {
    #[serde(default)]
    jobs: Vec<Job>,
}

type JobCache = Arc<Mutex<HashMap<u64, Vec<Job>>>>;

pub fn run(args: Args) -> io::Result<()> {
    let client = Arc::new(Client::for_current_repo()?);
    let runs = list_runs(&client, args.all)?;
    if runs.is_empty() {
        if args.all {
            eprintln!("info: No failed CI runs found.");
        } else {
            eprintln!(
                "info: No failed CI runs found in the last 24 hours. Run with -a or --all to view historical runs."
            );
        }
        return Ok(());
    }
    if std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .is_err()
    {
        return Err(output::fail(
            "selecting a CI run needs an interactive terminal",
            "run 'focal ci-fail' from a terminal",
        ));
    }
    let cache: JobCache = Arc::default();
    let Some(run) = choose(&client, runs, args.all, &cache)? else {
        return Ok(());
    };
    let jobs = match cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&run.id).cloned())
    {
        Some(jobs) => jobs,
        None => fetch_jobs(&client, run.id)?,
    };
    let content = failure_context(&client, &run, &jobs)?;
    let mut payload = output::header()?;
    payload.push_str(&content);
    output::deliver(
        payload,
        &format!("Copied CI failure logs for run {}", run.id),
    )
}

fn list_runs(client: &Client, all: bool) -> io::Result<Vec<Run>> {
    let mut path = format!(
        "/repos/{}/actions/runs?per_page={LIST_LIMIT}&status=failure",
        client.repo.slug()
    );
    if !all {
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
            .saturating_sub(24 * 60 * 60);
        write!(path, "&created=%3E%3D{}", iso_timestamp(cutoff)).unwrap();
    }
    let value = client.get_json(&path)?;
    let list: RunList = serde_json::from_value(value).map_err(|error| {
        output::fail(
            format!("GitHub returned an unexpected workflow run list: {error}"),
            "retry the command",
        )
    })?;
    let mut runs = list.workflow_runs;
    runs.truncate(LIST_LIMIT);
    Ok(runs)
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or_default()
}

/// The legacy fzf line: `[status/conclusion] created | workflow (branch) - title`.
fn label(run: &Run) -> String {
    let conclusion = match run.conclusion.as_deref() {
        Some(conclusion) if !conclusion.is_empty() => format!("/{conclusion}"),
        _ => String::new(),
    };
    format!(
        "{}  [{}{conclusion}] {} | {} ({}) - {}",
        run.id,
        text(&run.status),
        text(&run.created_at),
        text(&run.name),
        text(&run.head_branch),
        text(&run.display_title),
    )
}

fn choose(
    client: &Arc<Client>,
    runs: Vec<Run>,
    all: bool,
    cache: &JobCache,
) -> io::Result<Option<Run>> {
    let items = runs
        .iter()
        .map(|run| {
            let label = label(run);
            picker::Item {
                search: label.clone(),
                label,
                weight: 0,
            }
        })
        .collect();
    let header = if all {
        "Showing all historical failed runs"
    } else {
        "Showing failed runs from the last 24 hours (use -a / --all for full history)"
    };
    let options = picker::Options {
        prompt: "Select failed CI run: ".into(),
        header: vec![header.into()],
        multi: false,
    };
    let runs = Arc::new(runs);
    let (shared, client, cache) = (Arc::clone(&runs), Arc::clone(client), Arc::clone(cache));
    let loader: Arc<dyn Fn(usize) -> String + Send + Sync> = Arc::new(move |id| {
        let run = &shared[id];
        let jobs = fetch_jobs(&client, run.id).map_err(|error| error.to_string());
        if let (Ok(jobs), Ok(mut cache)) = (&jobs, cache.lock()) {
            cache.insert(run.id, jobs.clone());
        }
        preview(run, jobs)
    });
    Ok(match picker::pick(items, &options, Some(loader))? {
        picker::Outcome::Selected(ids) => ids.first().map(|&id| runs[id].clone()),
        picker::Outcome::Cancelled => None,
    })
}

fn preview(run: &Run, jobs: Result<Vec<Job>, String>) -> String {
    let mut text = format!(
        "Run {} · {} · {}/{}\nTitle: {}\nBranch: {}\nEvent: {}\nCommit: {}\nCreated: {}\nURL: {}\n\n",
        run.id,
        self::text(&run.name),
        self::text(&run.status),
        self::text(&run.conclusion),
        self::text(&run.display_title),
        self::text(&run.head_branch),
        self::text(&run.event),
        self::text(&run.head_sha)
            .chars()
            .take(12)
            .collect::<String>(),
        self::text(&run.created_at),
        self::text(&run.html_url),
    );
    match jobs {
        Err(error) => writeln!(text, "Could not load jobs: {error}").unwrap(),
        Ok(jobs) => {
            let failed: Vec<&Job> = jobs
                .iter()
                .filter(|job| logs::is_failure(job.conclusion.as_deref()))
                .collect();
            if failed.is_empty() {
                text.push_str("No failed jobs.\n");
            } else {
                text.push_str("Failed jobs:\n");
            }
            for job in failed {
                writeln!(text, "  {} ({})", job.name, self::text(&job.conclusion)).unwrap();
                for step in job
                    .steps
                    .iter()
                    .filter(|step| logs::is_failure(step.conclusion.as_deref()))
                {
                    writeln!(text, "    ✗ {}. {}", step.number, step.name).unwrap();
                }
            }
        }
    }
    text
}

fn fetch_jobs(client: &Client, run_id: u64) -> io::Result<Vec<Job>> {
    let mut jobs = Vec::new();
    let mut next = Some(format!(
        "/repos/{}/actions/runs/{run_id}/jobs?per_page=100",
        client.repo.slug()
    ));
    for _ in 0..100 {
        let Some(url) = next.take() else { break };
        let page = client.request(&url, "application/vnd.github+json")?;
        let list: JobList = serde_json::from_str(&page.body).map_err(|error| {
            output::fail(
                format!("GitHub returned an unexpected job list for run {run_id}: {error}"),
                "retry the command",
            )
        })?;
        jobs.extend(list.jobs);
        next = page.next;
    }
    Ok(jobs)
}

fn expired(run_id: u64, error: ApiError) -> io::Error {
    match error.status {
        Some(status @ (404 | 410)) => output::fail(
            format!("the logs for run {run_id} have expired or were deleted (HTTP {status})"),
            "GitHub removes workflow logs after the retention period; re-run the workflow to produce new logs",
        ),
        _ => error.into(),
    }
}

/// The rendered `# CI Failure Context` section for a run's failed steps.
fn failure_context(client: &Client, run: &Run, jobs: &[Job]) -> io::Result<String> {
    let no_failures = || {
        output::fail(
            format!("run {} has no failed steps with logs", run.id),
            &match run.html_url.as_deref() {
                Some(url) => format!("it may have failed before any job ran; open {url}"),
                None => "it may have failed before any job ran".to_owned(),
            },
        )
    };
    if !jobs
        .iter()
        .any(|job| logs::is_failure(job.conclusion.as_deref()))
    {
        return Err(no_failures());
    }
    let archive = client
        .download(
            &format!("/repos/{}/actions/runs/{}/logs", client.repo.slug(), run.id),
            DOWNLOAD_LIMIT,
        )
        .map_err(|error| expired(run.id, error))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(archive)).map_err(|error| {
        output::fail(
            format!(
                "the logs for run {} are not a readable archive: {error}",
                run.id
            ),
            "retry the command",
        )
    })?;
    let names: Vec<String> = (0..archive.len())
        .map(|index| {
            archive
                .name_for_index(index)
                .and_then(Result::ok)
                .map(|name| name.into_owned())
                .unwrap_or_default()
        })
        .collect();
    let segments = logs::associate(&names, jobs);
    if segments.is_empty() {
        return Err(no_failures());
    }
    let requests = segments
        .iter()
        .filter(|segment| matches!(segment.source, Source::Job(_)))
        .count();
    if requests > MAX_JOB_LOG_REQUESTS {
        return Err(output::fail(
            format!(
                "the logs archive for run {} is missing too many job logs",
                run.id
            ),
            "open the run on GitHub to read its logs",
        ));
    }
    let mut raw = String::new();
    let mut budget = MAX_LOG_BYTES;
    for segment in segments {
        if budget == 0 {
            eprintln!(
                "warning: failed-step logs exceed {} MiB; later steps were omitted",
                MAX_LOG_BYTES / (1024 * 1024)
            );
            break;
        }
        let log = match segment.source {
            Source::Entry(index) => {
                let mut bytes = Vec::new();
                archive
                    .by_index(index)
                    .and_then(|entry| {
                        entry
                            .take(budget)
                            .read_to_end(&mut bytes)
                            .map_err(Into::into)
                    })
                    .map_err(|error| {
                        output::fail(
                            format!(
                                "could not read '{}' from the run logs: {error}",
                                names[index]
                            ),
                            "retry the command",
                        )
                    })?;
                bytes
            }
            Source::Job(job) => client
                .download(
                    &format!("/repos/{}/actions/jobs/{job}/logs", client.repo.slug()),
                    DOWNLOAD_LIMIT,
                )
                .map_err(|error| expired(run.id, error))?,
        };
        budget = budget.saturating_sub(log.len() as u64);
        logs::gh_lines(&mut raw, &segment.job, &segment.step, &log);
    }
    let filtered = logs::filter_logs(raw.trim());
    let title = format!(
        "{} - {}",
        run.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("CI"),
        text(&run.display_title)
    );
    let body = file::fenced(&format!("[error logs]\n{}", filtered.trim()), "text");
    Ok(format!("# CI Failure Context: {title}\n\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_timestamp(1_795_978_861), "2026-11-29T19:01:01Z");
    }

    #[test]
    fn labels_runs_like_legacy() {
        let run = Run {
            id: 9,
            name: Some("CI".into()),
            display_title: Some("Fix".into()),
            head_branch: Some("main".into()),
            created_at: Some("2026-11-24T10:00:00Z".into()),
            status: Some("completed".into()),
            conclusion: Some("failure".into()),
            ..Run::default()
        };
        assert_eq!(
            label(&run),
            "9  [completed/failure] 2026-11-24T10:00:00Z | CI (main) - Fix"
        );
    }
}
