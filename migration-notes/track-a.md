# Track A: `issues` and `pr-diff`

## Implemented

`issues` and `pr-diff` run on the shared GitHub REST client and the Rust picker, with no `gh`, `fzf`, `jq`, or `column` on PATH. Both resolve the repository and credentials before any other work, so missing credentials, a missing `origin`, and HTTP failures exit 1 with the client's diagnostic and hint.

`issues` lists issues newest first from `/repos/{owner}/{repo}/issues` (`state=open`, or `state=all` with `--all`), skips the pull requests that endpoint also returns, and follows `Link` pages until it has 100 issues. The picker rows keep the legacy `column -t` layout (number, `OPEN`/`CLOSED`, title, `(@author)`), the `Select issues:` prompt, and the multi-select header. The preview shows `# title` and the body, or `*No description provided.*`. Each selected issue renders as `# Issue #N: title`, `URL:`, `## Description`, and a `## Discussion Thread` of `### Comment i (@author)` entries, with comments collected across every page. Issues are separated by `---`. When one issue's detail or comments request fails, Focal prints `error: could not fetch issue #N: …` with a hint, emits the legacy `[Could not fetch issue details or comments]` placeholder, and continues. The done message is `Copied context for Issue #N` or `Copied context for K issues`. Cancelling or selecting nothing exits 0 silently, as legacy does after fzf.

`pr-diff` accepts a number, `#number`, a pull request URL, or a head branch name. Without an argument it looks up the current branch (its upstream `branch.<name>.merge` name when configured, as `gh pr view` does) through `pulls?head=owner:branch&state=all`, preferring an open PR. A failed or empty lookup quietly falls back to a single-select picker of up to 30 open PRs (`Select PR:`, rows `number  [head] title`); cancelling exits 0 silently. The output keeps `# PR #N: title`, `URL:`, `## Intent / Description` (with the no-description placeholder), and `## Diff` with the patch requested as `application/vnd.github.diff` and fenced so nested backticks cannot close it. A failed patch request carries the legacy hint `verify that the PR exists and you have access`. The done message is `Copied context and diff for PR #N`.

Tests: `tests/migration/test_issues.py` and `tests/migration/test_pr_diff.py` (Rust only, since legacy needs `gh`) cover list pagination, the 100-issue cap, PR exclusion, `--all`, comment pagination, a failing issue among successful ones, preview, multi-select, cancel, a 401 on the listing, missing credentials with an empty PATH, a missing origin, the Bearer header, explicit number/URL/branch identifiers, current-branch and upstream lookup, lookup failure and empty lookup falling back to the picker, picker cancel, no terminal for the fallback, a 404 PR, and a failed patch request.

## Intentional differences

These cannot carry `xfail(LEGACY, strict=True)` tests because the legacy commands require an authenticated `gh` and `fzf`, which the suite does not provide. Each is covered by the named Rust test.

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `issues` with no matching issues | Opens an empty fzf list, then exits 0 quietly | Prints `info: No open issues found in owner/repo.` (or `No issues` with `--all`) and exits 0 without a picker |
| `issues` without a controlling terminal | fzf fails and the command exits 0 with no output | Exits 1: `selecting issues needs an interactive terminal` (checked before the listing request) |
| `issues` per-issue failure | `error: Error executing gh issue view N --json title,body,comments,url: <gh stderr>` | `error: could not fetch issue #N: <API diagnostic>` plus a hint (`test_one_failing_issue_does_not_abort_the_others`) |
| `issues`, multi-select order | fzf prints selections in the order they were marked | Selections render in list order (newest first), the shared picker's contract (`test_lists_across_pages_without_pull_requests`) |
| `pr-diff https://github.com/o/r/pull/12` | Heading reads `# PR #https://github.com/o/r/pull/12: …` | Normalizes to the number: `# PR #12: …` (`test_explicit_url_and_empty_body`) |
| `pr-diff` with no branch PR and no terminal | fzf fails; exits 0 with no output | Exits 1: `no pull request found for the current branch, and choosing one needs an interactive terminal`, hint `focal pr-diff <number>` (`test_no_terminal_for_the_fallback`) |
| `pr-diff` with no open PRs to pick | Empty fzf list, exits 0 quietly | `info: No open pull requests found in owner/repo.` and exit 0 |
| `pr-diff <n>` HTTP failures | `Error executing gh pr view …: <gh stderr>` | The client's status-specific diagnostic and hint, exit 1 (`test_missing_pull_request`, `test_patch_failure`) |

## Not done or needing a decision

- Branch lookup assumes the PR head lives in the `origin` owner's repository. `gh` also consults the branch's push remote to find PRs opened from a fork; a fork-based branch here falls back to the picker.
- The PR picker has no preview, matching legacy. The list body is already fetched, so a body preview would be cheap to add.
- `issues` stops at 20 listing pages when a repository has very many pull requests interleaved with issues; with 100 per page that only matters past 2,000 consecutive PRs.
- GitHub returns HTTP 406 for patches over its diff size limit; `pr-diff` reports that as an error, as `gh pr diff` does, without a fallback to per-file patches.
