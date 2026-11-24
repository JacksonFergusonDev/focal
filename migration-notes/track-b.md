# Track B: `ci-fail`

## Implemented

`ci-fail` lists failed workflow runs through the REST API with the same query `gh run list --limit 30 --status failure` sends (`per_page=30&status=failure`), adding `created=>=<now minus 24 hours>` unless `-a/--all` is given. With no runs it prints the legacy `info:` message for the active mode and exits 0. A single-select picker offers each run with the legacy label (`<id>  [status/conclusion] created | workflow (branch) - title`), the legacy prompt, and the legacy 24-hour or all-history header. The preview shows the run's title, branch, event, commit, creation time, and URL, followed by its failed jobs and their failed steps. The preview loads jobs on the picker's worker thread and caches them for the selected run. Cancelling exits 0 without output, as legacy does after an empty fzf selection. Without a controlling terminal, the command explains that choosing a run needs one.

After selection the command fetches the run's jobs and steps (following `Link` pagination) and downloads the run logs archive from `/actions/runs/{id}/logs`. The endpoint's redirect to the archive host is followed by the shared agent, which never forwards `Authorization`. The new `Client::download` in `src/github/track_b.rs` reads the response as bytes. Archive entries are associated with jobs and steps as GitHub CLI does. Job names lose `/` and `:` and are cut to 90 UTF-16 units. Failed jobs (`failure`, `timed_out`, `startup_failure`, or `action_required`) contribute the `<job>/<step number>_*.txt` entries of their failed steps, in step order. A job whose steps have no entries contributes its top-level `<n>_<job>.txt` log, or the legacy `-<n>_<job>.txt` log. Without either, the job comes from `/actions/jobs/{id}/logs`, also without credentials after the redirect. The command allows at most 25 such job requests, matching GitHub CLI. The command rebuilds `gh run view --log-failed` output (`job<TAB>step<TAB>line`, with line endings and a trailing carriage return trimmed) and runs it through a line-for-line port of `focal/gh_ci_fail.py`. The port covers Python `splitlines` boundaries, job, step, and timestamp prefix stripping, ANSI stripping, boilerplate removal, collapsed successful `##[group]` blocks, and the 15-before and 10-after error windows joined by `...`. The section heading, the `[error logs]` fence, and the `Copied CI failure logs for run <id>` done message match legacy.

Diagnostics:

- Missing credentials use the shared client's message.
- An HTTP 404 or 410 from the logs endpoint, or from a job logs fallback, reports that the run's logs expired or were deleted, with a hint to re-run the workflow.
- A run without failed jobs, or whose failed steps have no logs, reports `run <id> has no failed steps with logs` with the run URL.

`tests/migration/test_ci_fail.py` runs only against the Rust binary, since legacy needs `gh`. It computes expected log text with the legacy `filter_logs` applied to the `gh`-format text, so the rendered output is checked against legacy processing. The tests cover:

- the 24-hour query versus `-a`/`--all`
- missing credentials
- picker cancellation
- a mixed-success run in which only the failed step's entry appears
- ANSI and timestamp stripping, error windows, and collapsed groups
- the absence of `Authorization` on the cross-host archive and job-log redirects (the redirect targets `localhost` while the API is `127.0.0.1`)
- the job-log and job-endpoint fallbacks
- expired logs (404 and 410)
- a run without failed steps

## Intentional differences

None of these can be `xfail(LEGACY, strict=True)` tests, because the legacy command needs `gh` and a live GitHub. Each one is covered by a Rust-only test.

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `ci-fail` on a run whose step logs start with a UTF-8 byte order mark (GitHub's log files do) | The BOM sits between the step field and the timestamp, so the prefix regex misses the first line of each log. That line keeps `job<TAB>step<TAB>﻿<timestamp>`. | Drops the BOM, so the first line is stripped like the rest (`test_mixed_success_run_renders_only_failed_steps`) |
| `ci-fail` on a run with no failed jobs or no failed-step logs | `gh run view --log-failed` prints nothing, so the command copies an empty `[error logs]` block | Exits 1 with `run <id> has no failed steps with logs` and the run URL (`test_run_without_failed_steps`) |
| `ci-fail` on a run whose logs expired | `gh` reports `log not found`, and the command dies with that message | Explains that the logs expired or were deleted and suggests re-running the workflow (`test_expired_logs`) |
| `ci-fail` without credentials | `GitHub CLI is not authenticated.` | The shared client's `no GitHub credentials found for <host>` with the `GH_TOKEN`/`gh auth login` hint (`test_missing_credentials`) |
| `ci-fail` on logs that contain ANSI escapes, with a current `gh` (unverified) | Current GitHub CLI passes log text through `asciisanitizer`, which is believed to rewrite control characters as visible text, so the `\x1b[...m` regex may no longer match | Reads the archive directly, so the legacy ANSI regex removes the escapes (`test_mixed_success_run_renders_only_failed_steps`) |

## Not done or needing a decision

- Run attempts: like `gh run view` without `--attempt`, the command reads the latest attempt's jobs and logs. Earlier attempts cannot be selected.
- The picker list takes half the terminal width next to the preview, so long run labels are truncated in the list. The full label stays searchable, and the preview repeats the details.
- The ANSI difference depends on `asciisanitizer` behavior that was not checked against a real `gh` run.
- Large archives are read into memory, up to 1 GiB, as GitHub CLI also does.
