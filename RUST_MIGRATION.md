# Focal migration to Rust

Replace the Bash dispatcher and Python backend with one installed `focal` executable for macOS and Linux. The primary performance target is extracting a repository of many small files. Today, file classification, metadata collection, fencing, and formatting can each launch processes per file. The Rust implementation should do that work in process and preserve the context people paste into LLMs or pipe into scripts.

The behavioral reference for this plan is commit `64b9641`. The milestones below describe proposed work. Keep the installed dispatcher usable until the Rust binary passes the full command suite.

## Current implementation

The Rust `tree` command renders hidden entries, empty directories, symlink targets, executable markers, and up to six levels of Git-aware directory structure in process. It sorts sibling entries by filename bytes, which can differ from locale-specific collation in the external `tree` utility.

The opt-in Cargo binary supports piped `files --all` and `files <path>...`, including recursive `dir/` selections. The `ignore` walker discovers hidden files, honors Git, fd, and generic ignore files, and visits sibling entries in path order. Explicit selections retain their first-seen order. The `git2` backend reads branch and commit metadata in process. The shared CLI tests exercise Rust extraction with an empty PATH. Binary detection currently samples for NUL bytes and omitted-file metadata reports the byte count and category. Notebook extraction runs in process and preserves cell order, nested Markdown fences, output MIME precedence, and the 4,000-character per-output limit. Malformed notebooks emit an inline diagnostic so extraction can continue. Terminal output goes to stdout until clipboard support lands. The installed dispatcher remains the complete implementation.

PDF extraction uses `pdf-extract`, chosen over PDFium after a benchmark on 15 real PDFs (multi-column TeX cards, Cyrillic and Japanese text, a 147,000-character license, vector-only, encrypted, and malformed files). Word recall and reading order matched PDFium and legacy pdfplumber, and both Rust extractors kept two-column reading order that pdfplumber interleaves. Every file finished within 0.5 s. PDFium was 2–7 times faster on mid-size files, slower on the largest, and the only one to read RC4-encrypted files. It would add a 7.2 MB shared library (prebuilt PDFium is published only as shared libraries) or a self-maintained static Chromium build. The Rust extractor reads pages one at a time up to the 50-page cap, applies the 50,000-character cap, replaces typographic ligatures, and keeps extracting other files when one fails. A PDF that yields no text produces a specific reason inline and on stderr: not a PDF, damaged or truncated, encrypted (with a `qpdf --decrypt` hint), uses an unsupported feature, crashed the parser, or has no text layer (with an OCR hint). `files` reports these as `failed:` and counts them in the extraction summary.

`files --staged` and `files --modified` select paths through `git2`. Staged mode compares HEAD (or the empty tree on an unborn branch) to the index with rename detection, drops deletions, and renders working-tree contents. Modified mode compares the index to the working tree under the invocation directory.

`context --core` and `context --all` emit the tree, a Git Info section, the root core manifests, and either the named paths or every discovered file. The Git Info section reproduces `git status --porcelain` from `git2`, including conflict codes, index renames, type changes, collapsed untracked directories, Git's path quoting, and tracked-before-untracked ordering. The interactive picker is pending, so `context` without `--core` or `--all` exits with a diagnostic.

`search`, `symbol`, and `api` share one `grep-searcher` pass over ripgrep's default file set: ignore rules apply, hidden entries and binary files are skipped, and `.rgignore` is honored. Output keeps ripgrep's piped format (`./path:match`, `./path-context`, `--` between non-adjacent groups and between files). Files are visited in path order, where ripgrep's order varies between runs.

`diff` and `wip-context` render patches, stats, and topology through `git2`. `diff` keeps the default unstaged, `--staged/--cached`, `-a/--all` (with untracked patches and unborn HEAD), and `--thick` modes, the Status, Summary, and Diff sections, and the warnings about excluded staged, unstaged, and untracked changes. Without explicit paths it excludes noise files before rename detection, so a kept file never pairs with an excluded one. Explicit pathspecs resolve from the invocation directory and override the exclusions. `wip-context` resolves an explicit base, the remote default branch, or main/master/develop (local or `origin/`), and keeps the uncommitted status, topology, net diffstat, root and `.github/` patch priority, the 30,000-character patch budget with closed fences, omitted-file metadata, and the no-divergence diagnostic.

These two commands preserve Git's information rather than its exact bytes. Expected cosmetic differences: libgit2 pads `--stat` counts and scores rename similarity slightly differently, topology omits `git log`'s extra blank lines and rename scores (`R` instead of `R100`), and omitted-file types come from a built-in magic-byte check such as `PNG image data` instead of `file -b`. The shared tests assert on content, section order, and budgets for these commands.

### Intentional differences

The shared suite marks each of these `xfail(strict=True)` against the legacy dispatcher.

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `files --staged` from a subdirectory | Reads root-relative paths from `git diff --name-only` and silently skips files that don't resolve from the current directory | Selects every staged file and renders paths relative to the current directory, such as `../src/main.rs` |
| `files --modified` with a merge conflict | `git ls-files -m` lists the path once per conflict stage, so the file appears up to three times | Lists the file once |
| `context --all` with a nested `README.md` or other manifest name | `fd --exclude README.md` drops every file with that name at any depth | Excludes only the root manifests already emitted |
| `context --core README.md` | Emits the manifest twice | Emits it once |
| `search '('` | Prints ripgrep's parse error, then reports no matches and exits 0 | Reports `invalid regex` and exits 1 |
| `wip-context` on a branch that renamed a file | Diffs the new path alone, so the patch shows the whole file as added | Detects the rename and shows only the rename header and any edited hunks |
| `files locked.pdf` for an RC4-encrypted PDF that opens without a password | Extracts the text | Reports that the PDF is encrypted and suggests `qpdf --decrypt` (a known gap in the `lopdf` parser) |
| `files art.pdf` for a PDF without text, damaged, or not a PDF | `[PDF contains no extractable text]` or `[Error parsing PDF: …]`, sometimes with an empty message | States the specific reason inline and on stderr with a hint, and counts the file as failed |

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
