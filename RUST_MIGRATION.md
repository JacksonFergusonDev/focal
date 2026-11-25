# Focal migration to Rust

Replace the Bash dispatcher and Python backend with one installed `focal` executable for macOS and Linux. The primary performance target is extracting a repository of many small files. Today, file classification, metadata collection, fencing, and formatting can each launch processes per file. The Rust implementation should do that work in process and preserve the context people paste into LLMs or pipe into scripts.

The behavioral reference for this plan is commit `64b9641`. The milestones below describe proposed work. Keep the installed dispatcher usable until the Rust binary passes the full command suite.

## Current implementation

The Rust dispatcher reproduces the legacy help menu, `help`, `-h`, `--help`, `--commands`, and the unknown-command diagnostic. It takes `--repo <url>` and `--repo=<url>` from either side of the subcommand, passes tokens after `--` to the command untouched, and treats an empty `--repo=` as local. Each command owns its arguments in its own module. A shared GitHub client resolves the repository from the checkout's `origin` (HTTPS, SSH, and scp-style URLs, including GitHub Enterprise hosts) and credentials scoped to the host: `GH_TOKEN`/`GITHUB_TOKEN` only for github.com and `*.ghe.com`, `GH_ENTERPRISE_TOKEN`/`GITHUB_ENTERPRISE_TOKEN` for other hosts, then `gh auth token --hostname`. A github.com token is therefore never sent to a server named by a checkout's `origin` or by `--repo`. It follows `Link` pagination, never forwards credentials across redirects, and maps 401, 403, 404, rate limits, and transport failures to actionable diagnostics. `FOCAL_GITHUB_API_URL` overrides the API base, and the suite's `github` fixture serves canned API responses through it.

The Rust `tree` command renders hidden entries, empty directories, symlink targets, executable markers, and up to six levels of Git-aware directory structure in process. It sorts sibling entries by filename bytes, which can differ from locale-specific collation in the external `tree` utility.

The opt-in Cargo binary supports piped `files --all` and `files <path>...`, including recursive `dir/` selections. The `ignore` walker discovers hidden files, honors Git, fd, and generic ignore files, and visits sibling entries in path order. Explicit selections retain their first-seen order. The `git2` backend reads branch and commit metadata in process. The shared CLI tests exercise Rust extraction with an empty PATH. Binary detection currently samples for NUL bytes and omitted-file metadata reports the byte count and category. Notebook extraction runs in process and preserves cell order, nested Markdown fences, output MIME precedence, and the 4,000-character per-output limit. Malformed notebooks emit an inline diagnostic so extraction can continue. Redirected output is written to stdout without touching the clipboard. With a terminal on stdout, macOS copies in process through `arboard`, since the pasteboard keeps the contents after Focal exits (verified by pasting from a separate process). Linux hands the payload to `wl-copy` under Wayland or `xclip`/`xsel` under X11, because those clipboards are served by the owning process. When no clipboard is available, Focal names the reason and writes the payload to stdout. X11 and Wayland persistence still needs a check on a real desktop. The installed dispatcher remains the complete implementation.

PDF extraction uses `pdf-extract`, chosen over PDFium after a benchmark on 15 real PDFs (multi-column TeX cards, Cyrillic and Japanese text, a 147,000-character license, vector-only, encrypted, and malformed files). Word recall and reading order matched PDFium and legacy pdfplumber, and both Rust extractors kept two-column reading order that pdfplumber interleaves. Every file finished within 0.5 s. PDFium was 2–7 times faster on mid-size files, slower on the largest, and the only one to read RC4-encrypted files. It would add a 7.2 MB shared library (prebuilt PDFium is published only as shared libraries) or a self-maintained static Chromium build. The Rust extractor reads pages one at a time up to the 50-page cap, applies the 50,000-character cap, replaces typographic ligatures, and keeps extracting other files when one fails. A PDF that yields no text produces a specific reason inline and on stderr: not a PDF, damaged or truncated, encrypted (with a `qpdf --decrypt` hint), uses an unsupported feature, crashed the parser, or has no text layer (with an OCR hint). `files` reports these as `failed:` and counts them in the extraction summary.

`files --staged` and `files --modified` select paths through `git2`. Staged mode compares HEAD (or the empty tree on an unborn branch) to the index with rename detection, drops deletions, and renders working-tree contents. Modified mode compares the index to the working tree under the invocation directory.

`context --core` and `context --all` emit the tree, a Git Info section, the root core manifests, and either the named paths or every discovered file. The Git Info section reproduces `git status --porcelain` from `git2`, including conflict codes, index renames, type changes, collapsed untracked directories, Git's path quoting, and tracked-before-untracked ordering.

`files` without arguments and `context` without `--core` or `--all` open the interactive picker. It draws on the controlling terminal with `ratatui` and `crossterm`, so it works while stdout is redirected (`focal files > result.md`), and it filters with `nucleo-matcher` using fzf's query syntax and smart case. Items keep stable identities, so marks survive filtering, and selections return in discovery order. Keys follow fzf: Tab and Shift-Tab mark and move, Ctrl-A and Ctrl-D mark and unmark every match, Enter accepts the marks (or the item under the cursor when nothing is marked), and Esc, Ctrl-C, Ctrl-G, or Ctrl-Q cancel. Shift-Up and Shift-Down scroll the preview. The info line shows the match count, the number selected, and a size-based token estimate, so no candidate is formatted while the user types. Cancelling `files` reports that no files were selected. Cancelling `context` still emits the tree, Git info, core manifests, and pre-selected paths, which are listed in the picker header and not offered again. A drop guard restores the terminal on every exit path, including panics. Without a controlling terminal, the command explains that it needs paths or a flag.

Previews render on a worker thread that skips to the newest request, so a slow PDF never blocks typing, and results are keyed by item so a stale preview never replaces the current one. Previews reuse the notebook and PDF extractors, show numbered text for the first 1,000 lines or 256 KiB, and show type and size for noise and binary files. Text previews are not syntax-highlighted yet; `syntect` was left out to limit binary growth. `issues`, `pr-diff`, and `ci-fail` will use the same picker in milestone 5. Unit tests cover filtering, marking, cancellation, scrolling, rendering through `TestBackend`, and stale previews. Pseudo-terminal tests drive `files` and `context` end to end and check the restored terminal modes.

`search`, `symbol`, and `api` share one `grep-searcher` pass over ripgrep's default file set: ignore rules apply, hidden entries and binary files are skipped, and `.rgignore` is honored. Output keeps ripgrep's piped format (`./path:match`, `./path-context`, `--` between non-adjacent groups and between files). Files are visited in path order, where ripgrep's order varies between runs.

`web` converts pages with `html-to-markdown-rs`, the library behind the `html-to-markdown` CLI, using the same preprocessing and the nav, footer, header, aside, button, form, and SVG exclusions. Fetched and piped HTML share the converter. Output matched the legacy command byte for byte on five local HTML documents and three live pages, including the 50,000-character cap. Fetching uses synchronous `ureq` with redirects, gzip and Brotli decoding, the existing browser headers, and a 20-second timeout. TLS uses `rustls` with bundled Mozilla roots, so a corporate TLS-inspection root in the OS trust store is not honored yet. The diagnostics for HTTP 401, 403, 404, 999, other statuses, empty responses, connection failures, invalid schemes, and empty stdin match the legacy messages. Adding `web` grew the release binary from 6.4 MB to 11.8 MB, mostly from the HTML converter. A release profile with `strip`, LTO, and one codegen unit brings the full binary to 8.3 MB.

`diff` and `wip-context` render patches, stats, and topology through `git2`. `diff` keeps the default unstaged, `--staged/--cached`, `-a/--all` (with untracked patches and unborn HEAD), and `--thick` modes, the Status, Summary, and Diff sections, and the warnings about excluded staged, unstaged, and untracked changes. Without explicit paths it excludes noise files before rename detection, so a kept file never pairs with an excluded one. Explicit pathspecs resolve from the invocation directory and override the exclusions. `wip-context` resolves an explicit base, the remote default branch, or main/master/develop (local or `origin/`), and keeps the uncommitted status, topology, net diffstat, root and `.github/` patch priority, the 30,000-character patch budget with closed fences, omitted-file metadata, and the no-divergence diagnostic.

These two commands preserve Git's information rather than its exact bytes. Expected cosmetic differences: libgit2 pads `--stat` counts and scores rename similarity slightly differently, topology omits `git log`'s extra blank lines and rename scores (`R` instead of `R100`), and omitted-file types come from a built-in magic-byte check such as `PNG image data` instead of `file -b`. The shared tests assert on content, section order, and budgets for these commands.

`issues` and `pr-diff` run on the shared GitHub REST client and the Rust picker, with no `gh`, `fzf`, `jq`, or `column` on PATH. Both resolve the repository and credentials before any other work, so missing credentials, a missing `origin`, and HTTP failures exit 1 with the client's diagnostic and hint.

`issues` lists issues newest first from `/repos/{owner}/{repo}/issues` (`state=open`, or `state=all` with `--all`), skips the pull requests that endpoint also returns, and follows `Link` pages until it has 100 issues. The picker rows keep the legacy `column -t` layout (number, `OPEN`/`CLOSED`, title, `(@author)`), the `Select issues:` prompt, and the multi-select header. The preview shows `# title` and the body, or `*No description provided.*`. Each selected issue renders as `# Issue #N: title`, `URL:`, `## Description`, and a `## Discussion Thread` of `### Comment i (@author)` entries, with comments collected across every page. Issues are separated by `---`. When one issue's detail or comments request fails, Focal prints `error: could not fetch issue #N: …` with a hint, emits the legacy `[Could not fetch issue details or comments]` placeholder, and continues. The done message is `Copied context for Issue #N` or `Copied context for K issues`. Cancelling or selecting nothing exits 0 silently, as legacy does after fzf.

`pr-diff` accepts a number, `#number`, a pull request URL, or a head branch name. Without an argument it looks up the current branch (its upstream `branch.<name>.merge` name when configured, as `gh pr view` does) through `pulls?head=owner:branch&state=all`, preferring an open PR. A failed or empty lookup quietly falls back to a single-select picker of up to 30 open PRs (`Select PR:`, rows `number  [head] title`); cancelling exits 0 silently. The output keeps `# PR #N: title`, `URL:`, `## Intent / Description` (with the no-description placeholder), and `## Diff` with the patch requested as `application/vnd.github.diff` and fenced so nested backticks cannot close it. A failed patch request carries the legacy hint `verify that the PR exists and you have access`. The done message is `Copied context and diff for PR #N`.

`--repo <url>` prepares a cached checkout in process with `git2` and changes into it before dispatch, so it works with an empty PATH. The cache path is the legacy one, `${XDG_CACHE_HOME:-$HOME/.cache}/focal/repos/<basename>_<first 16 hex of sha256(url)>`, with `basename` following POSIX `basename "$url" .git`, so legacy entries are reused in place. A first use clones into a hidden scratch directory (`.tmp-<pid>-<entry>`), writes `.git/.focal_created` and `.git/.focal_accessed`, and renames it into place, so an entry only appears complete. Later uses fetch every remote, read origin's advertised default branch on the same connection, point `origin/HEAD` at it, and hard-reset the checkout to it. When the default branch was renamed, the checkout moves to a local branch of the new name tracking `origin/<name>`. When the fetch fails, Focal prints legacy's warning with the reason on an indented line and reuses the checkout. Status lines match legacy (`info: Cloning remote repository into cache...`, `info: Updating cached remote repository...`, `warning: Could not update the cached repository; using the cached copy.`). With a terminal on stderr, transfer progress is drawn on one line and cleared.

Concurrency uses `std::fs::File` locks in `.locks/`. `<entry>.lock` is held exclusively while an entry is cloned or updated, so two cold-cache invocations serialize: one clones, the other waits (saying so) and then updates the finished entry. `<entry>.use` is held shared from before the update until the process exits. Cleanup only removes an entry whose two locks it can take without waiting, and never the entry the current process uses. Leftover scratch directories of an entry are removed under its update lock, so an interrupted or killed clone leaves no entry and the next run clones cleanly. An entry that libgit2 cannot open (for example what a killed legacy `git clone` leaves) is replaced by a fresh clone, swapped in by rename only when no other process is using it.

Cleanup keeps legacy's semantics: at most once a day (`.last_cleanup` holds epoch seconds), `FOCAL_REPO_TTL_DAYS` (default 30) whole days counted like `find -mtime +N`, entries aged by `.git/.focal_accessed` or, without it, by the directory mtime, and the same `Pruning stale/legacy cached repository` lines. It only considers `<basename>_<16 hex>` directories that contain `.git`. Deletion renames the entry to `.trash-<pid>-<entry>` first. An invalid TTL value disables cleanup (legacy's `find` would fail and prune nothing). Legacy dispatchers take no locks, so a legacy process running beside Rust is not protected.

Credentials for private remotes are independent of the GitHub API client: SSH tries the agent, then `~/.ssh/id_ed25519`, `id_ecdsa`, and `id_rsa`. HTTPS asks the Git credential helpers configured for the URL (`Cred::credential_helper` with the default Git config), then offers `GH_TOKEN` or `GITHUB_TOKEN` as `x-access-token` to `https://github.com` only. Each source is offered once, so a rejected credential fails instead of looping. Credentials are never printed, and any `user:password@` from the URL is redacted from error messages.

**Clone strategy.**

libgit2 cannot make partial clones, so Rust makes full clones. Fixture: a generated bare repository with 2,000 commits over 4,000 text files (15 files changed per commit), a 142 MB pack, served on localhost by `git daemon` (git://) to model a smart server. Apple Silicon macOS, three samples, median:

| Strategy | First clone | `.git` size |
| --- | --- | --- |
| Git CLI full clone | 7.84 s | 142 MB |
| Git CLI `--filter=blob:none` (legacy) | 1.69 s | 23 MB |
| Git CLI `--depth=1` | 0.99 s | 17 MB |
| Rust `--repo … tree`, git2 full clone | 3.05 s | 142 MB |

A warm `--repo … tree` with nothing to fetch took 0.14 s. Over a real network the full clone's cost is dominated by transfer: about six times the bytes of legacy's blob-filtered clone on this history-heavy fixture (repositories with less churn differ less). Over `file://`, libgit2 builds the pack in process and the same clone took 25 s against 2.4 s for the Git CLI; plain local paths use libgit2's local copy and are fast. Decision: keep full clones with git2 so `--repo` has no Git CLI dependency and keeps full history for history-reading commands. Cold-clone cost is paid once per URL per TTL.

**Legacy partial clones.**

libgit2 1.9 refuses to open repositories with `extensions.partialclone` and cannot fetch missing blobs on demand. A legacy blob-filtered entry is therefore replaced with a full clone on its first online use (one full-clone cost, then normal updates). Offline, Focal registers the `partialclone` extension for the process and reuses the checkout, whose working tree and HEAD objects are present, so commands and the Git header work.

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

None of these can be `xfail(LEGACY, strict=True)` tests, because the legacy command needs `gh` and a live GitHub. Each one is covered by a Rust-only test.

- **Completions.** `focal completions bash|zsh` renders a script with `clap_complete` from `cli::Cli`, so subcommand names and their flags come from the parser. The generator drops hidden subcommands (`completions` itself) and adds the dispatcher's global `--repo <URL>`, which `cli::split_repo` handles outside clap. Any other shell name fails with `unsupported shell '<name>'` and a hint. The generated Bash script works under Bash 3.2. The static scripts in [completions](../completions) still complete command names through `focal --commands`, which the Rust dispatcher answers, so existing installs keep working against either executable; they stay in the repository for the legacy Homebrew formula until cutover.
- **Release profile.** `[profile.release]` sets `strip = true`, `lto = true`, and `codegen-units = 1`. It keeps `opt-level = 3` and the default `panic = "unwind"`, which the picker's `Drop` guard relies on to restore the terminal. Clean `cargo build --locked --release` on an Apple Silicon Mac (aarch64-apple-darwin, rustc 1.99.1), with other builds sometimes running on the machine, so times are approximate:

  | Profile | Size | Build time |
  | --- | --- | --- |
  | Default release | 14.0 MB | 67 s |
  | `strip` only | 11.2 MB | 93 s (contended) |
  | `strip`, thin LTO, 1 CGU | 10.2 MB | 85 s |
  | **`strip`, fat LTO, 1 CGU (chosen)** | **9.3 MB** | 95–131 s |
  | chosen + `opt-level = "s"` | 7.9 MB | 140 s |
  | chosen + `opt-level = "z"` | 6.5 MB | 98 s |
  | chosen + `panic = "abort"` | 8.3 MB | 100 s |

  With `scripts/benchmark_migration.py` (100-file fixture, warm cache), `opt-level = "z"` was within 3% of `opt-level = 3` for `tree`, `files --all`, and `context --all`, and no slower on a regex search across the Cargo registry. It is a candidate for a further 2.8 MB, but large PDF, HTML, and Brotli inputs have not been measured, so this track keeps `opt-level = 3`. `panic = "abort"` would save only 1 MB and remove the picker's terminal-restore guarantee.
- **Release workflow.** On a `v*.*.*` tag, [release.yml](../.github/workflows/release.yml) runs the unchanged legacy `sync-homebrew` job and, independently, builds the Rust binary on native runners for `aarch64-apple-darwin` (macos-15), `x86_64-apple-darwin` (macos-15-intel), `x86_64-unknown-linux-gnu` (ubuntu-22.04), and `aarch64-unknown-linux-gnu` (ubuntu-22.04-arm). Each job checks the tag against the `Cargo.toml` version, prints linked libraries and the highest glibc symbol version, packages `focal-<version>-<target>.tar.gz` (binary, generated `completions/focal.bash` and `completions/_focal`, README, LICENSE) with a `.sha256` file, and smoke-tests the extracted binary outside the checkout with an empty environment. A publish job attaches the archives and a combined `SHA256SUMS` to a **draft** GitHub release for a maintainer to review. `workflow_dispatch` builds and uploads workflow artifacts without publishing or touching the tap.
- **Native dependencies.** `git2` (through `libgit2-sys` and `libssh2-sys`) depends on `openssl-sys`. A default local build on macOS links Homebrew's `libssl`/`libcrypto` dylibs (`/opt/homebrew/opt/openssl@…`), which would break the archive on any Mac without that formula. The workflow sets `OPENSSL_STATIC=1` (with `OPENSSL_DIR` from `brew --prefix openssl@3` on macOS) and fails the build if the binary still links OpenSSL or anything under `/opt/homebrew` or `/usr/local`. Locally, the static build links only system libraries (`libobjc`, `libiconv`, `libz`, system frameworks), reports `minos 11.0`, is 12.4 MB instead of 9.3 MB, and `focal web https://example.com` works.
- **Supported platforms.** macOS 11 or later (`MACOSX_DEPLOYMENT_TARGET=11.0` on both architectures). Linux with glibc 2.35 or later (Ubuntu 22.04, Debian 12, RHEL 10), from building on ubuntu-22.04 runners; no OpenSSL runtime dependency.
- **CI.** A `rust-install` job in [ci.yml](../.github/workflows/ci.yml) checks that every file carries the same version, runs `cargo install --locked --path . --root <tmp>` from the clean checkout, and runs the installed binary (`--commands` with an empty environment, and both completion scripts).
- **Version bump.** [scripts/release.py](../scripts/release.py) replaces the external `release.py` that `just bump` fetched. It keeps the same pre-flight checks (tools, branch, clean tree, remote sync, tag collisions), bumps `pyproject.toml`, `uv.lock`, `Cargo.toml`, and `Cargo.lock` together, verifies both lockfiles (`uv lock --check`, `cargo metadata --locked --offline`), commits, tags, and pushes atomically, and resets the repository if any step fails. It uses only the standard library. `--check` prints the version and fails if the files disagree. Tested end to end in a scratch clone against a local bare remote, including rollback.
- **justfile.** `rust-build`, `rust-test` (fmt, clippy, unit tests, and the parity suite against both executables), `rust-install` / `rust-uninstall` (`~/.local/bin/focal-rs`; `just install` still installs the legacy `focal`), `rust-package` (a host archive in `target/dist/`, as the workflow builds it), `bump`, `bump-dry-run`, and `version`.

`release-context` resolves release membership in process with `git2`. Without `--base`, it selects the nearest reachable stable `vMAJOR.MINOR.PATCH` or `MAJOR.MINOR.PATCH` tag for the level (patch, minor x.y.0, or major x.0.0), excluding a tag on the target commit, and falls back to the repository beginning, which includes root commits through an empty-tree diff. The level comes from a positional `patch`/`minor`/`major`, `--patch`, `--minor`/`-m`, `--major`, or `--level`/`--since`, and the last form given wins, as in the legacy argument loop. `--version` targets an existing local tag unless `--head` is given; otherwise the version is inferred from a tag on the head or left as `vX.Y.Z`. Commits are listed in `git log --reverse --topo-order` order with first-parent paths per commit (merges list the paths they bring in), and the report keeps the non-ancestor and shallow-checkout notes.

GitHub enrichment shares one deadline (default 60 s) and a failure circuit across every request, and caps each request (default 10 s) by the remaining time. Collection runs in three steps: a repository lookup; then the release listing (or the explicit `--previous-release`) and 20-commit GraphQL association batches; then the chosen style reference and REST association listings for commits whose GraphQL connection overflowed or whose batch failed. Each step runs on up to four worker threads, and results and their diagnostics are assembled in job order, so the report is identical whatever order requests complete in. REST and release listings follow `Link` pages to exhaustion. A timeout, transport failure, 5xx, rate limit (HTTP 429, a 403 with an exhausted quota or `retry-after`, GraphQL `RATE_LIMITED`, or a zero GraphQL `rateLimit.remaining`), or authentication failure stops scheduling. Unsent requests are skipped, and the report says why. Without credentials, without a GitHub `origin`, or offline, the full local report is still built. Lookups that were never made count as unresolved and are reported separately from failed ones.

The report keeps the legacy drafting prompt, previous-release style reference (nearest published, non-draft, non-prerelease release reachable before the target), commit-pinned asset URLs, PR index, deterministic grouping that prefers the PR whose merge commit is the commit, ambiguity notes, unmatched commits, the three bot modes, Release Impact, `--diff` with repeatable cwd-relative `--path` filters, and completeness notes. The 24,000-token limiter is a character-for-character port and is reapplied after `--project-context`, which appends the current checkout's six-level tree, Git status, and 500-line manifest excerpts. Output omits the workspace header.

`src/github/track_c.rs` adds `Client::call` (GET or JSON POST with a per-request timeout, returning the status, body, next link, and rate-limit headers) and `Client::graphql_url` (`/graphql`, or `/api/graphql` beside an Enterprise `/api/v3` base). It is registered with one `pub(crate) mod track_c;` line in `src/github/mod.rs`.

Tests: `tests/migration/test_release_context.py` runs against both executables with GitHub unreachable (a failing `gh` stub for legacy, a closed local port for Rust). It covers level forms, historical selection from a newer checkout, a root release with pinned asset placeholders, merge-range order and first-parent paths, a divergent base, shallow history, offline notes, bot modes, focused diffs, project background, and output bounds with balanced fences. `tests/migration/test_release_github.py` is Rust-only (legacy needs `gh`). Its local fake answers GraphQL per commit from the request body and can delay routes. It covers enriched grouping, pinned asset URLs, the style reference across a paginated listing, an ambiguous association, pages followed to exhaustion, a failure on page two, deadline expiry midway (four listings complete, four time out against the remaining budget, two are never sent), a rate limit, GraphQL errors falling back to REST, missing credentials, and identical output when batches complete in a different order.

### Intentional differences

The shared suite marks each of these `xfail(strict=True)` against the legacy dispatcher. The GitHub commands cannot run under the legacy suite because they need an authenticated `gh` and `fzf`, so their rows name the Rust test that covers them instead.

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `focal` with no arguments under Bash 3.2 (macOS `/bin/bash`) | Exits 1 with `REMAINING_ARGS[@]: unbound variable` | Prints the help menu |
| `files --staged` from a subdirectory | Reads root-relative paths from `git diff --name-only` and silently skips files that don't resolve from the current directory | Selects every staged file and renders paths relative to the current directory, such as `../src/main.rs` |
| `files --modified` with a merge conflict | `git ls-files -m` lists the path once per conflict stage, so the file appears up to three times | Lists the file once |
| `context --all` with a nested `README.md` or other manifest name | `fd --exclude README.md` drops every file with that name at any depth | Excludes only the root manifests already emitted |
| `context --core README.md` | Emits the manifest twice | Emits it once |
| `search '('` | Prints ripgrep's parse error, then reports no matches and exits 0 | Reports `invalid regex` and exits 1 |
| `wip-context` on a branch that renamed a file | Diffs the new path alone, so the patch shows the whole file as added | Detects the rename and shows only the rename header and any edited hunks |
| `files locked.pdf` for an RC4-encrypted PDF that opens without a password | Extracts the text | Reports that the PDF is encrypted and suggests `qpdf --decrypt` (a known gap in the `lopdf` parser) |
| `files art.pdf` for a PDF without text, damaged, or not a PDF | `[PDF contains no extractable text]` or `[Error parsing PDF: …]`, sometimes with an empty message | States the specific reason inline and on stderr with a hint, and counts the file as failed |
| `issues` with no matching issues | Opens an empty fzf list, then exits 0 quietly | Prints `info: No open issues found in owner/repo.` (or `No issues` with `--all`) and exits 0 without a picker |
| `issues` without a controlling terminal | fzf fails and the command exits 0 with no output | Exits 1: `selecting issues needs an interactive terminal` (checked before the listing request) |
| `issues` per-issue failure | `error: Error executing gh issue view N --json title,body,comments,url: <gh stderr>` | `error: could not fetch issue #N: <API diagnostic>` plus a hint (`test_one_failing_issue_does_not_abort_the_others`) |
| `issues`, multi-select order | fzf prints selections in the order they were marked | Selections render in list order (newest first), the shared picker's contract (`test_lists_across_pages_without_pull_requests`) |
| `pr-diff https://github.com/o/r/pull/12` | Heading reads `# PR #https://github.com/o/r/pull/12: …` | Normalizes to the number: `# PR #12: …` (`test_explicit_url_and_empty_body`) |
| `pr-diff` with no branch PR and no terminal | fzf fails; exits 0 with no output | Exits 1: `no pull request found for the current branch, and choosing one needs an interactive terminal`, hint `focal pr-diff <number>` (`test_no_terminal_for_the_fallback`) |
| `pr-diff` with no open PRs to pick | Empty fzf list, exits 0 quietly | `info: No open pull requests found in owner/repo.` and exit 0 |
| `pr-diff <n>` HTTP failures | `Error executing gh pr view …: <gh stderr>` | The client's status-specific diagnostic and hint, exit 1 (`test_missing_pull_request`, `test_patch_failure`) |
| `--repo <url>` after the remote renamed its default branch | Resets the old local branch (for example `main`) to the new default, so the header still says `Git Branch: main` | Checks out a local branch named after the new default (`test_default_branch_rename_switches_the_local_branch`) |
| `--repo <url>` with a damaged cache entry (a killed `git clone`) | Fetch fails, warns, and runs in the broken checkout | Clones again and swaps the entry in (`test_damaged_entry_is_cloned_again`) |
| Cleanup with a directory such as `notes/` older than the TTL in the cache | Deletes it | Only considers `<basename>_<16 hex>` entries with `.git` (`test_cleanup_spares_directories_focal_does_not_own`) |
| Cleanup while another process works in a stale entry | Deletes it | Skips entries whose use lock is held (`test_cleanup_spares_an_entry_in_use`) |
| Two simultaneous cold-cache invocations | Both run `git clone` into the same path; one fails | One clones, the other waits and updates (`test_simultaneous_cold_invocations_share_one_clone`, skipped under legacy because the outcome is a race) |
| A killed first clone | Can leave a partial entry | Leaves only a scratch directory, removed by the next run (`test_killed_clone_is_retried_cleanly`, skipped under legacy for the same reason) |
| Legacy blob-filtered entry, online | Fetches into the partial clone | Replaces it with a full clone (asserted in `test_legacy_partial_entry_is_updated` under Rust only) |
| Offline update | Git's `fatal:` lines, then the warning | The warning, then `reason: <libgit2 message>` |
| First clone of a large repository | Git's progress output | A one-line progress counter on a terminal, nothing otherwise |
| `ci-fail` on a run whose step logs start with a UTF-8 byte order mark (GitHub's log files do) | The BOM sits between the step field and the timestamp, so the prefix regex misses the first line of each log. That line keeps `job<TAB>step<TAB>﻿<timestamp>`. | Drops the BOM, so the first line is stripped like the rest (`test_mixed_success_run_renders_only_failed_steps`) |
| `ci-fail` on a run with no failed jobs or no failed-step logs | `gh run view --log-failed` prints nothing, so the command copies an empty `[error logs]` block | Exits 1 with `run <id> has no failed steps with logs` and the run URL (`test_run_without_failed_steps`) |
| `ci-fail` on a run whose logs expired | `gh` reports `log not found`, and the command dies with that message | Explains that the logs expired or were deleted and suggests re-running the workflow (`test_expired_logs`) |
| `ci-fail` without credentials | `GitHub CLI is not authenticated.` | The shared client's `no GitHub credentials found for <host>` with the `GH_TOKEN`/`gh auth login` hint (`test_missing_credentials`) |
| `ci-fail` on logs that contain ANSI escapes, with a current `gh` (unverified) | Current GitHub CLI passes log text through `asciisanitizer`, which is believed to rewrite control characters as visible text, so the `\x1b[...m` regex may no longer match | Reads the archive directly, so the legacy ANSI regex removes the escapes (`test_mixed_success_run_renders_only_failed_steps`) |
| `focal completions bash` / `zsh` | Unknown command; scripts are static files | Prints a script generated from the parser, completing flags as well as commands (tests in `test_completions.py` are `xfail(LEGACY, strict=True)`) |
| `release-context` with GitHub unavailable (offline, no credentials, no GitHub `origin`) | Reports `0 unresolved association lookups` and treats every commit as a confirmed empty association | Counts each skipped lookup as unresolved, lists the commits in the incomplete-enrichment warning, and adds `Unresolved association lookups: N failed, M skipped …` (`test_offline_lookups_count_as_unresolved`) |
| `release-context --unknown-flag`, `release-context --base` (no value), `release-context --help` | Its own messages (`unknown argument: …`, `--base requires an argument`) with exit status 1; hand-written help | clap's messages (`unexpected argument '--unknown-flag' found`, `a value is required for '--base <REF>'`) with exit status 2; clap's help listing the same options. Positional, `--level`, `--bots`, `--max-tokens`, timeout, and `--path` errors keep the legacy wording and exit status 1 (`test_usage_errors_come_from_clap`, `test_argument_errors`) |
| `release-context --head does-not-exist` | `Git command failed: git rev-parse --verify … Error: fatal: Needed a single revision` | `unknown revision or not a commit: 'does-not-exist'` with a `git fetch --tags` hint (`test_unknown_revision`) |
| Request diagnostics in Completeness Notes and on stderr | Name `gh` operations (`gh api repos/…`, `gh api graphql`, `gh repo view`, `gh release view`) and quote `gh` stderr | Name HTTP operations (`GET repos/…`, `POST graphql`) and quote the HTTP status and API message. Untestable against legacy, which needs `gh`; covered by `test_association_pages_are_followed_and_page_two_failures_stay_unresolved` and `test_graphql_errors_fall_back_to_rest_listings` |
| Request scheduling | Serial `gh` processes; notes in chronological order; the release listing stops when a page has fewer than 100 items | Up to four requests in flight; per-job notes in job order followed by the stop reason; listings follow `Link` headers. The request count in the final note can differ by the extra short page legacy fetches. Untestable against legacy; covered by `test_output_order_is_independent_of_completion_order` and `test_deadline_expiry_midway_keeps_completed_results` |
| Git-rendered text (approved: same information, different bytes) | `git diff --stat` columns, `git log --name-only` paths quoted under `core.quotePath` (`"caf\303\251.txt"`), `git diff` patch headers | libgit2 diffstat padding, raw UTF-8 paths, libgit2 patch headers with 7-character index abbreviations. Shared tests assert on paths and hunk content only |
| Tag choice among equally near tags | `git describe --tags --abbrev=0 --match …` | Fewest commits between tag and head (libgit2 ahead/behind), ties to the tag found first in commit-date order; on one commit an annotated tag beats a lightweight one, then newer tagger date, then refname order. Matches `git describe` on the fixtures; exotic ties are not cross-checked |
| `release-context --max-tokens N` when the budget cuts the release impact | The impact is moved into a footer cut to a third of the budget, which can leave its `text` fence open and drop Completeness Notes; a PR description quoting `## Release Impact` is moved instead | The impact and the notes are clipped separately, every cut closes its fence, and the real section is moved (`limited_output_keeps_fences_balanced_and_notes_present`, `heading_inside_a_pr_description_is_not_moved`) |

### Open items

**`issues` and `pr-diff`:**

- Branch lookup assumes the PR head lives in the `origin` owner's repository. `gh` also consults the branch's push remote to find PRs opened from a fork; a fork-based branch here falls back to the picker.
- The PR picker has no preview, matching legacy. The list body is already fetched, so a body preview would be cheap to add.
- `issues` stops at 20 listing pages when a repository has very many pull requests interleaved with issues; with 100 per page that only matters past 2,000 consecutive PRs.
- GitHub returns HTTP 406 for patches over its diff size limit; `pr-diff` reports that as an error, as `gh pr diff` does, without a fallback to per-file patches.

**`--repo`:**

- Private HTTPS and SSH remotes were not exercised against a real server: there is no network fixture for authentication. The credential order is unit-tested only for its host gate and redaction.
- The `PWD:` header line shows the physical cache path (`std::env::current_dir`), while legacy subcommands print the shell's logical `$PWD`; they differ only when the cache path contains a symlink.
- Decision: optionally use the Git CLI for `--filter=blob:none` clones when it is on PATH (falling back to git2), or a git2 shallow clone (`depth=1`, about 17 MB here) if history is not needed by `--repo` users. Neither is implemented.
- Lock files in `.locks/` are never deleted (removing a lock file another process may have open is racy); they are empty.

**`ci-fail`:**

- Run attempts: like `gh run view` without `--attempt`, the command reads the latest attempt's jobs and logs. Earlier attempts cannot be selected.
- The picker list takes half the terminal width next to the preview, so long run labels are truncated in the list. The full label stays searchable, and the preview repeats the details.
- The ANSI difference depends on `asciisanitizer` behavior that was not checked against a real `gh` run.
- Large archives are read into memory, up to 1 GiB, as GitHub CLI also does.

**Cutover:**

- `tests/migration/test_ci_fail.py` computes expected output by importing the legacy `focal.gh_ci_fail.filter_logs`. Before removing the Python runtime, replace that import with recorded expected outputs.

**Packaging:**

- The workflows were linted with `actionlint` but not run. Runner labels `macos-15-intel` and `ubuntu-22.04-arm` and the pinned `upload-artifact` / `download-artifact` SHAs need a first `workflow_dispatch` run to confirm.
- The Homebrew formula still installs the legacy layout. Switching it to the Rust archives (and their generated completions) is the cutover step; the draft release gives a place to verify archives before that.
- `publish = false` stays. Enabling crates.io would need `description`, `license`, and `repository` metadata and an `include` list: `cargo package --list` currently ships the whole repository, including the legacy runtime and test fixtures.
- Static OpenSSL comes from the runner's Homebrew bottle (macOS) or `libssl-dev` (Linux). The Homebrew static libraries may be compiled for a newer macOS than 11, which the linker would report as a warning; the real floor needs checking on an old macOS. Enabling `openssl-sys`'s `vendored` feature (or disabling `git2`'s `ssh` feature if SSH remotes are not needed) would pin the OpenSSL build in `Cargo.lock` but changes dependencies, so it is left as a decision. An older glibc floor would need an older build image, a `zig`/`cross` toolchain, or a musl target.
- `rust-install` installs the binary only; the generated completion scripts are bound to the command name `focal`, so they are not installed for `focal-rs`.

**`release-context`:**

- Decision: should the dispatcher map clap usage errors to exit status 1 for every command? That would be a `src/cli.rs` change outside this track. `release-context` keeps legacy wording for the errors it validates itself.
- Decision: concurrency is four requests. GitHub's guidance prefers serial requests per token to avoid secondary rate limits; batching keeps a 500-commit release to 25 GraphQL requests, so a lower limit costs little.
- Integration: `track_c.rs` repeats part of `Client::request`'s response handling, because that function returns only mapped errors and has no per-request timeout. During integration, `Client::request` could delegate to `Client::call`. The registration line is `pub(crate) mod track_c;` so `release` can name `CallError`.
- Not verified against real GitHub: the GraphQL query is the legacy query verbatim, and REST shapes follow the documented API, but every test uses the local fake. The Enterprise GraphQL URL (`/api/graphql`) is unexercised.
- Offline asset rows keep the legacy `<ASSET_URL: …>` placeholders, although the Rust client knows the repository from `origin`. Deriving `https://<host>/<owner>/<name>` offline would be a further intentional difference.
- `--repo` depends on the remote-cache track.

Run the shared contract suite against both executables from the repository root:

```sh
cargo build --locked --release
python -m pytest tests/migration
FOCAL_TEST_EMPTY_PATH=1 FOCAL_TEST_BIN=target/release/focal python -m pytest tests/migration
python scripts/benchmark_migration.py bin/focal target/release/focal --files 64 --samples 3
```

The first warm-cache run on Apple Silicon macOS used 64 small Python modules in eight directories. Across three timed samples, median `files --all` time was 1.374 s for Bash and 0.025 s for Rust. Both emitted 3,997 bytes with equivalent file blocks. Median peak RSS reported by `wait4` was 12.8 MiB and 8.4 MiB respectively. Legacy baseline medians for `tree` and `context --all` on the same fixture shape were 0.113 s and 1.780 s. This fixture measures small-file overhead. Mixed document extraction and cold-cache timings remain open.

## Delivery contract

- The standard release and plain `cargo install` should support all 13 public commands. Local extraction and Git operations must run without Python, Git CLI, or the existing discovery, formatting, and picker executables on PATH. GitHub commands must work with a supplied token and without GitHub CLI.
- Keep `gh auth token` optional for users already signed in through GitHub CLI. Allow an optional desktop clipboard helper where needed for persistent clipboard ownership. Piped commands must work in a headless session without either integration.
- Preserve command names, argument forms, selection semantics, Markdown boundaries, meaningful diagnostics, and exit behavior. Keep one list of intentional differences in the migration PRs, with the affected invocation and expected output. Examples worth evaluating are stable file ordering, platform-specific file-type descriptions, and returning an actionable diagnostic for an invalid regex.
- The installed binary is the unit of delivery. Native code in a dependency is acceptable when it meets packaging requirements. A PDF build that requires shipping a sibling library changes the distribution contract and needs an explicit decision before adoption.

## Command contracts

Use the public dispatcher for comparisons. The wrappers in [libexec](libexec), the shared routines in [core.sh](lib/core.sh), and the [Python backend](focal) together define what a command does.

| Command | Behavior to preserve |
| --- | --- |
| `files` | Direct paths, recursive `dir/`, interactive multi-select, `-a/--all`, `-s/--staged`, and `-m/--modified`. A bare directory gets a trailing-slash hint. Direct paths deduplicate in first-seen order. Staged mode selects staged paths excluding deletions and renders their current working-tree contents. Modified mode selects tracked modifications as in `git ls-files -m`. Preserve per-file outcomes and the extraction summary. |
| `context` | Emit the tree, Git info when available, and existing core manifests. The default picker offers additional files and accounts for positional selections. `-c/--core` bypasses the picker and retains explicit paths. `-a/--all` discovers additional files. Preserve manifest selection and section order. |
| `tree` | Render from the invocation directory, include hidden entries, apply ignore rules, mark directory entries, and limit depth to six. Exclude `.git`, `node_modules`, `.venv`, `__pycache__`, `dist`, and `build` even without ignore rules. |
| `search` | Search a regex with five lines of context. Preserve path prefixes, overlapping-context grouping, and support for dash-prefixed patterns via `--`. No matches produce an informational diagnostic and success. |
| `symbol` | Find every file containing the supplied regex anywhere, including comments and call sites. Render the full matching files through the shared file formatter. |
| `api` | Emit matching source lines for top-level Python classes and sync or async functions, including private names. Preserve the Python file filter and column-zero match. |
| `diff` | Default to unstaged changes. Support `--staged/--cached`, `-a/--all`, `--thick`, and explicit pathspecs after `--`. Preserve status, summary, patch, and warnings about excluded change categories. All mode adds untracked patches and handles an unborn HEAD. Explicit paths override default noise exclusions. Thick mode appends the tree and affected working-tree files. |
| `wip-context` | Resolve an explicit base or infer the remote default, with main/master/develop fallbacks. Show working-tree status, divergent commit topology with paths, net diffstat, prioritized patches, and omitted-file metadata. The 30,000-character budget belongs to the WIP patch section. Preserve the no-divergence diagnostic. |
| `web` | Accept an HTTP(S) URL, a host without a scheme, or HTML from stdin. Keep the source heading, DOM preprocessing, selector exclusions, 50,000-character cap, and useful fetch errors. Piped browser HTML is a first-class offline input. |
| `issues` | List up to 100 open issues by default, include closed issues with `--all`, and allow multi-selection. Render each selected issue with its comment thread. Preserve per-issue failure reporting. |
| `pr-diff` | Accept an explicit PR identifier. With no argument, resolve the current branch’s PR first and offer a single-select list if lookup fails. Emit title, body, URL, and the PR patch. |
| `ci-fail` | Offer failed runs from the past 24 hours by default. `-a/--all` removes the time filter while retaining the list limit. Select one run, extract failed-step logs, and preserve error windows, collapsed successful groups, and ANSI/prefix stripping. |
| `release-context` | Resolve release membership from local Git ancestry and enrich it with GitHub data. Preserve historical endpoints, level aliases, prior-release style reference, commit-pinned assets, bot policy, focused diffs, optional checkout background, output budgets, and completeness notes. See milestone 6. |

The dispatcher also supports no-argument help, `help`, `--commands`, both forms of `--repo`, and an end-of-options delimiter. Capture placement of the global flag on either side of a subcommand and ensure tokens after `--` reach the command. Generate Bash and Zsh completions from the same command definitions. Keep the public command list distinct from internal PDF and notebook helpers.

## Implementation milestones

### 0. Establish the comparison harness and baseline

Build a harness that accepts a command path, creates disposable fixtures, invokes the public CLI, and records stdout, stderr, and exit status separately. Run the legacy [Bats](tests/bash) and [pytest](tests/python) suites as long as their implementation exists. Many call shell scripts directly or mock Python imports. Translate their user-visible assertions into tests that will survive removal of those entry points.

Create fixtures for a directory outside Git, a nested working directory, mixed tracked and ignored files, staged and unstaged edits to the same path, renames and deletions, an unborn branch, a merge history with tags, and a remote with a warm cache. Use fixed commit dates, locale, and terminal dimensions. Normalize temporary absolute paths and time-relative text narrowly. Compare file membership, selected revisions, section order, and truncation markers exactly. Where traversal order varies, compare whole file blocks so normalization cannot hide missing or duplicate files.

Benchmark an optimized Rust build against the legacy CLI on the same machine and fixture. Record startup for `tree`, wall time and peak memory for `files --all` over many small files, and `context --all` over a mixed repository. Record fixture size, OS, cache conditions, sample count, median time, and output size. Redirect output for both implementations and verify equivalent content before interpreting speedups.

**Exit criterion.** The legacy CLI passes the new representative cases, its known exceptions are recorded, and the benchmark can be rerun. Add Rust build, format, lint, and test jobs when the first Cargo code lands. Keep legacy CI alongside them.

### 1. Deliver one end-to-end local extraction path

Start with one Cargo package, a `clap` CLI, and a side-by-side binary exercised as `focal files --all`. Implement repo discovery, branch, abbreviated HEAD, and status early. The common output envelope and `context --core` already depend on them. Start with `git2` and use milestone 2 fixtures to confirm the choice before building more Git features on it.

Use `ignore` for discovery with explicit settings per caller. File extraction includes hidden files and excludes `.git`. Search and tree have separate policies. Keep paths as paths through discovery, selection, and opening. Limit text reads and reuse metadata. Preserve calls from subdirectories and outside a Git repository.

Embed [noise.json](lib/noise.json) as one source for exact filenames and case-insensitive suffixes, including compound suffixes. The formatter checks empty files first, dispatches notebooks and PDFs, emits metadata for noise/binary assets, and fences ordinary content with enough backticks. Preserve the 1,500-line and 100,000-byte text limits, including a single oversized line. Track added, empty, skipped, and truncated outcomes independently of rendered text.

Parse notebooks with `serde_json`. Preserve cell order, kernel metadata, execution counts, rich-output precedence, image omission, and the 4,000-character per-output default. Evaluate `pdf-extract` as the default on real PDFs with columns, tables, empty/image-only pages, and malformed input. Check reading order, the 50-page and 50,000-character caps, and the fallback messages against [the current extractor](focal/pdf.py). If quality is inadequate, evaluate PDFium immediately. Its [binding documentation](https://github.com/ajrcarey/pdfium-render#dynamic-linking) identifies the proposed prebuilt binaries as dynamic libraries. Prove an actual static artifact on the target matrix or document the impact of bundling a library. Test both the release and Cargo installation paths.

Centralize payload delivery. Preserve the outer context tags, workspace note, PWD, optional Git metadata, character-based token estimate, and stderr diagnostics. Web and release output omit the workspace header. With redirected stdout, write the entire payload and skip clipboard initialization. With a terminal, support clipboard copy and stdout fallback. Verify that a second process can paste after Focal exits on macOS, X11, and Wayland.

**Exit criterion.** Pipe the Rust binary’s output from a mixed fixture and compare the full payload and summary. Then extend the same implementation to direct `files`, `tree`, and non-interactive `context`. Exercise empty files, invalid input, nested backticks, cutoff boundaries, hidden files, and explicit directories. Re-run the extraction benchmark.

### 2. Port local Git and remote cache handling

Implement separate comparisons for index-to-worktree, HEAD-to-index, HEAD-to-worktree, and commit-to-commit. Preserve pathspec filtering, binary and untracked patches, rename handling, diffstat, and warnings. Use the library’s repository-aware diff support. Compare patch headers, paths, hunk contents, and command-specific limits against Git fixtures before accepting formatting differences.

Port staged/modified file selection, `diff`, and `wip-context`. Test staged content that is edited again, staged deletion, untracked-only changes, an explicit lockfile, an empty repository, and detached HEAD. Apply WIP’s root-file and `.github/` priority and keep fences closed when truncating. Confirm that a plain `diff` can exceed 30,000 characters. Exercise release-range ancestry on a merge fixture before committing to the backend for historical queries.

Treat `--repo` as a separate delivery within this milestone. Preserve the URL-keyed XDG cache, legacy entry discovery, access markers and TTL, default-branch refresh, and offline reuse. Verify filtered clone support and use of existing partially populated clones. If full clones are required, measure the first-clone cost and record the decision. Test the documented HTTPS workflow and existing local remote fixtures. Define credential handling for private remotes separately from GitHub API authentication.

Serialize competing updates to one cache entry. Publish a successful clone atomically, keep a usable checkout on refresh failure, and prevent expiry during active use. Restrict checkout resets and cleanup to Focal-owned cache entries.

**Exit criterion.** Run the Git fixtures through the public binary, including two simultaneous cold-cache invocations, an interrupted clone and retry, a remote default-branch rename, and an offline warm-cache invocation. Verify local extraction and Git behavior with the Git executable absent from PATH.

### 3. Port search and web extraction

Share `grep-searcher`, `grep-regex`, and file traversal across `search`, `symbol`, and `api`. Give each its own output formatter. Cover overlapping context windows, a call-site-only symbol, a private async Python function, an indented method, a hidden file, a leading dash, and zero matches.

For `web`, evaluate the Rust library behind the existing HTML-to-Markdown CLI first. Use the same converter for fetched and piped content. Preserve exclusions for navigation, footer, header, aside, button, form, and SVG elements. Use synchronous HTTP with redirects, compressed responses, and the existing 20-second fetch timeout. Exercise empty stdin, a login-wall response, an invalid scheme, and conversion at the output limit.

**Exit criterion.** Compare output on a fixed search corpus and an HTML page containing navigation, a table, and code. Serve the HTML over a local HTTP fixture and pipe it through stdin independently. Both paths must preserve the main content and their respective source headings.

### 4. Replace the interactive picker

Use one `ratatui`/`crossterm` picker with stable item identities, single- and multi-select modes, and `nucleo` filtering. Separate the display label, search text, selected value, and preview loader. Preserve Tab, Ctrl-A, Ctrl-D, Enter, Escape, selection across filtering, and each caller’s cancellation outcome. In particular, declining additional files in `context` still leaves its core context to emit.

Open the controlling terminal for interaction so the picker works with redirected stdout. Restore terminal state on success, cancellation, errors, and interruption. Load expensive previews outside the input/render loop and discard obsolete results when selection changes. Reuse the extraction code for notebooks, PDFs, and metadata previews. Apply `syntect` highlighting only to the visible preview. Keep estimates cheap enough to avoid formatting every candidate while a user types.

**Exit criterion.** Use `TestBackend` and state tests for selection, filtering, and scrolling. Use a pseudo-terminal to run `focal files > result.md`, send keys, and verify both the selected payload and restored terminal state. Repeat cancellation for `files` and `context`. A slow PDF preview must not block navigation or replace a newer selection.

### 5. Replace GitHub CLI for issues, PRs, and CI

Use `ureq` and `serde` for a synchronous client. Resolve repository owner, name, and host from the active checkout, including a cached checkout. Scope credentials to the intended host. Support `GH_TOKEN` and `GITHUB_TOKEN`, with optional host-aware `gh auth token` for existing logins. A plaintext entry in `hosts.yml` is insufficient as the primary integration because [GitHub CLI normally uses the OS credential store](https://cli.github.com/manual/gh_auth_login).

Implement list and detail operations separately. Include issue comments and their pagination, branch-to-PR lookup, the PR diff response, and run/job/step metadata. For CI, replace the behavior of `gh run view --log-failed`. The [run logs endpoint](https://docs.github.com/en/rest/actions/workflow-runs#download-workflow-run-logs) redirects to an archive. Associate entries with failed steps and test a run containing both successful and failed steps before applying the existing log window. Keep authorization off cross-host log downloads.

**Exit criterion.** Use fixture HTTP responses for pagination, unavailable issue details, PR lookup fallback, expired log downloads, mixed-success runs, and missing credentials. Exercise each command through its public entry point using a supplied token with no `gh` executable. Keep ordinary CI independent of live GitHub or personal credentials.

### 6. Port release context as its own milestone

Use [the release collector](focal/gh_release_context.py), [PR associations](focal/release_prs.py), [release notes](focal/release_notes.py), and [output limiter](focal/release_output.py) together as the reference.

1. Resolve the exclusive base and inclusive head. Preserve reachable stable tag selection, positional levels, aliases, explicit refs, and version/head precedence. Include root commits when the range starts at the empty tree. Preserve commit order, per-commit paths, non-ancestor diagnostics, and shallow-history warnings.
1. Build the local report even when GitHub enrichment is unavailable. Preserve the previous-release reference, commit-pinned asset URLs, merged PR associations, deterministic grouping, unmatched commits, ambiguous associations, and bot modes. Follow association pages to exhaustion within the request budget. Track failed or skipped lookups separately from confirmed empty results.
1. Use bounded request concurrency with a shared deadline. Defaults are 10 seconds per request and 60 seconds for GitHub collection. Cap each operation by the remaining time and stop scheduling on exhaustion or a terminal rate-limit response. Keep output order deterministic across completion orders.
1. Preserve net impact and repeatable `--path` filters for `--diff`. Keep the 24,000-token default and `--max-tokens 0` for full detail. Reapply the limiter after optional `--project-context`, which appends the *current checkout*, with its six-level tree and 500-line manifest excerpts. Preserve omission notices and balanced fences.

**Exit criterion.** Test historical release selection from a newer checkout, a root release, a non-linear range, an ambiguous association, failure on page two, deadline expiry midway through collection, and offline local output. Verify range membership, grouping, pinned asset revisions, and final output bounds independently.

### 7. Package and switch the installed entry point

Build release artifacts for Apple Silicon and Intel macOS, and x86-64 and ARM64 GNU/Linux. Pin and checksum any native build inputs. Inspect native dependencies, establish a supported deployment target per platform, and run the *installed* artifact on the supported architecture. Exercise PDF, local Git, HTTPS, completions, and headless output from outside the source checkout.

Update the [release workflow](.github/workflows/release.yml), external Homebrew tap integration, and [justfile](justfile) together. Publish and verify archives and checksums before updating the formula. Test plain `cargo install` from the packaged crate and a Homebrew installation with the generated Bash and Zsh completions. Replace the Python version-bump path and verify a clean install and an upgrade from the existing layout.

**Cutover criterion.** All 13 commands pass the public CLI suite. The installed artifact passes the target smoke tests with legacy tools absent, clipboard persistence has been checked on supported desktops, and before/after timings and peak memory are recorded. Review any accepted output differences before changing the default executable. Keep the previous release installable for rollback. Remove the Bash/Python runtime, superseded completion scripts, and obsolete build dependencies only after their replacement tests and installation paths are in place.
