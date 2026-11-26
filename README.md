<!-- rumdl-disable-file first-line-heading -->
<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/readme-light.svg">
  <img alt="Focal Logo"
       src="assets/readme-light.svg"
       width="320"
       style="max-width:100%; height:auto;">
</picture>

<br>

**Stop copy-pasting your codebase into ChatGPT. One command, formatted context, straight to your clipboard.**

[![PyPI Version](https://img.shields.io/pypi/v/focal-rs?style=flat-square&color=fb923c&labelColor=0A0A0A&logo=pypi&logoColor=fb923c)](https://pypi.org/project/focal-rs/)
[![CI](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/ci.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=CI)](https://github.com/JacksonFergusonDev/focal/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/release.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=release)](https://github.com/JacksonFergusonDev/focal/actions/workflows/release.yml)
[![Rust](https://img.shields.io/badge/rust-2024-fb923c?style=flat-square&labelColor=0A0A0A&logo=rust&logoColor=fb923c)](https://www.rust-lang.org/)
[![prek](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/j178/prek/master/docs/assets/badge-v0.json&style=flat-square&labelColor=0A0A0A&color=fb923c)](https://github.com/j178/prek)
[![License](https://img.shields.io/badge/license-MIT-fb923c?style=flat-square&labelColor=0A0A0A)](LICENSE)

</div>

You know the drill: you want a second opinion on your plan for a refactor, so you open ChatGPT, alt-tab back to your editor, select a few files, copy them in one at a time, scroll back up to grab the git diff, and hope you didn't forget any necessary files. By the time the context is pasted in, you've spent more time assembling it than the LLM spends answering.

Focal replaces that whole ritual with a single command. Point it at a diff, a branch, a GitHub issue, a CI failure, or a web page, and it hands back clean, LLM-formatted markdown — already on your clipboard, ready to paste into `chatgpt.com`, `claude.ai`, `gemini.google.com`, or wherever you're working.

---

## 🚀 Quick Start

Focal is a single self-contained binary for macOS and Linux. Install it with Homebrew:

```bash
brew install jacksonfergusondev/tap/focal
```

or from PyPI (the package is `focal-rs`; the command is `focal`):

```bash
uv tool install focal-rs   # or: pipx install focal-rs
```

*See [Detailed Installation & Autocompletion](#-detailed-installation--autocompletion) below for building from source and configuring Zsh/Bash.*

---

## 🎯 Isn't This What Claude Code / Cursor / Codex Already Do?

No — different job. Those are autonomous coding agents: point one at your repo and it traverses the file tree, runs commands, and writes code on its own, in a dedicated agentic loop.

Focal isn't an agent, and it doesn't try to be one. It exists for the moments you *don't* want to spin up an agentic session — when you just want to drop a precise, curated slice of your repo into a standard web chat to brainstorm an architecture decision, explain a concept, or debug a CI failure. If you're already living inside Claude Code, Codex, or Cursor, you probably don't need Focal for that session. Focal is for everything that still happens in a browser tab.

---

## 🚀 Usage

Focal is designed to be run from anywhere inside a valid Git workspace, but also handles external URLs and clipboard streams.

### Remote Repository Support

You can run any focal command against a remote GitHub repository without having to manually clone it first. Focal will automatically cache a clone locally to make subsequent runs instantaneous:

```bash
# Extract the project context of a remote repository
focal context --repo https://github.com/JacksonFergusonDev/focal

# List and extract specific files from a remote repository
focal files --repo https://github.com/JacksonFergusonDev/focal
```

### Basic Context Gathering

If you need to feed specific files or directories to an LLM, run the interactive selector or pass paths directly:

```bash
# Interactively select files (Tab to multi-select, Enter to confirm)
focal files

# Pass specific files or directories (directories require a trailing slash)
focal files src/ justfile

# Generate high-level project summary with pre-selected files or directories
focal context src/
```

### Git & Working Tree

To grab the exact state of your current feature branch (uncommitted changes, commit topology, microscopic diffs) compared to your repository's default branch (or specify a custom base branch):

```bash
# Automatically detects base branch (main, master, develop, or tracked remote)
focal wip-context

# Compare against a specific target branch
focal wip-context release/v1.0
```

To quickly grab the diff of your currently staged or uncommitted files (or use `--all` to include untracked files):

```bash
focal diff          # Unstaged changes
focal diff --staged # Staged changes
focal diff --all    # All changes (staged, unstaged, and untracked)
```

### Web & Documentation Context

You can pipe a library's documentation page directly to your clipboard, stripped of all DOM noise (navbars, scripts, footers) and bounded to 50,000 characters to protect LLM context windows:

```bash
# Fetch from a public URL (validates http:// and https://)
focal web https://docs.python.org/3/

# Parse an authenticated dashboard copied to your clipboard
pbpaste | focal web
```

### GitHub API Context

Need to debug why your GitHub Actions pipeline crashed? Grab the error logs and metadata instantly (preserving test runner failures across groups):

```bash
focal ci-fail
```

To collect release evidence from an exact Git range, with associated PR descriptions,
changed paths, unmatched commits, and a net diffstat:

```bash
# Draft the next release using the built-in prompt and previous release style
focal release-context --version v1.2.0

# Infer the release version when the target is already tagged
focal release-context --head v1.1.0

# Compare since the previous minor release (x.y.0)
focal release-context minor

# Inspect an explicit historical release
focal release-context --base v1.0.0 --head v1.1.0

# Draft a tagged release even after HEAD has advanced
focal release-context --version v1.1.0 --previous-release v1.0.0

# Include a focused net diff (repeat --path for multiple pathspecs)
focal release-context --diff --path src/ --path tests/

# Select a particular previous GitHub release as the style reference
focal release-context --version v1.2.0 --previous-release v1.1.0

# Expand bot updates and append current checkout background
focal release-context --bots include --project-context

# Save the payload instead of copying it to the clipboard
focal release-context > release-context.md
```

The payload starts with a request to write a GitHub release title and description.
Use `--version` to name a release. If that exact tag exists locally, it becomes
the target commit automatically, even when `HEAD` has moved past it. Without a
local version tag, the target defaults to `HEAD` for an upcoming release.
An explicit `--head` always overrides this selection, including `--head HEAD`.
Without `--version`, the version is inferred from a tag at the target commit when
possible, or left as `vX.Y.Z` for confirmation.
The prompt asks for release notes matching the previous release's structure and
tone, supported by the current range's evidence.

The previous GitHub release's title, description, tag, and URL are fetched through
`gh`. Automatic selection uses the nearest locally reachable published stable
release before the target commit, independent of the comparison base. Releases at
the target commit, drafts, and prereleases are excluded. `--previous-release <tag>`
selects a style reference explicitly, including a prerelease or a tag not available
locally, without changing release membership. Release listings are fully paginated;
missing references and lookup failures are reported without discarding Git evidence.

Tracked image and video assets at the target commit are included in an inventory
with commit-pinned file and raw URLs. Future, untracked, and symlink assets are not
included. URLs use the repository host returned by `gh`; unavailable repository
URLs produce placeholders. The inventory proves local path existence, not remote
availability or suitability. The prompt asks the writer to inspect relevant assets,
keep every repository asset link pinned to the full target SHA, and use labeled
placeholders when a link cannot be resolved. A separate verification checklist
lists proposed assets and unresolved links for review before publishing. Links from
the previous release must also be pinned to the new release commit before reuse.

Git ancestry determines which commits belong to the release. GitHub supplies merged
PR associations for those commits, across all target branches, without merge-date
filters or a 100-PR ceiling. Commits appear once, grouped under a PR or listed as
unmatched. Merge commits are included. PR descriptions have hidden HTML comments
removed; changed paths reflect only the commits included in the range.

The default base is the nearest reachable stable `vMAJOR.MINOR.PATCH` or
`MAJOR.MINOR.PATCH` tag. `minor` selects `x.y.0`; `major` selects `x.0.0`.
Prerelease and non-version tags are ignored. When the target is itself tagged,
the previous matching tag is used. Without a matching tag, all history reachable
from the selected head is included, including root commits. An explicit `--base`
is exclusive and the target head is inclusive. Fetch full history and tags before collecting
release context; shallow checkouts are flagged in the payload.

Bot updates are summarized by default. Use `--bots include` for full details or
`--bots exclude` to omit their detail. The net diffstat and optional diff always
include bot changes. `--diff` includes the full net diff unless repeatable `--path`
Git pathspecs focus it; path filters do not change release membership or the diffstat.
Output defaults to an approximate 24,000-token budget (four characters per token).
A PR index covers collected PRs before the overall limit is applied. Long PR
bodies use 800-character excerpts; detailed commit and path lists show up to five
entries per PR with omission counts. The release-impact summary and completeness
notes are retained when the overall payload is clipped, and a prominent warning
marks incomplete output. Use `--max-tokens 0` for full detail or increase the budget.
Optional project background is also subject to the overall budget.

The payload records resolved endpoint SHAs, counts, exclusions, ambiguous PR
associations, and failed lookups. GitHub failures preserve local commit evidence
with a warning. Cherry-picks without a GitHub association remain unmatched;
commit subjects are not used to guess PR attribution. When several PRs are
associated with one commit, an exact merge SHA match is preferred, then the lowest
PR number, and all alternatives are noted. PR descriptions reflect current GitHub
metadata, which may have changed since the release. PR associations are batched in groups of 20 commits per GraphQL request.
Overflow connections use fully paginated REST lookups, without a PR count ceiling.
Changed paths for the entire range are collected with one local Git log invocation
(using first-parent merge diffs; requires Git 2.31 or later).

GitHub progress is printed to stderr, leaving piped Markdown clean. Requests are
noninteractive, with a 10-second process timeout and a shared 60-second GitHub
collection budget. Override these with `--request-timeout` and `--github-timeout`.
Timeouts terminate the `gh` process group, including credential helpers. Rate limits,
authentication failures, and connection failures stop further remote requests;
there are no automatic retries or waits for quota resets. Local Git evidence is
still returned, with incomplete enrichment and skipped associations reported.
The GitHub budget does not limit local Git processing or optional project background.

`--project-context` appends the current checkout's directory tree (six levels),
Git status, and core manifests (first 500 lines each). It is labeled separately
from the historical release snapshot. This background is omitted by default.

---

## 🛠 Command Reference

### Local & Web Context Gathering

| Command | Description |
| --- | --- |
| `focal files [paths...]` | Interactively or directly extracts file and directory contents (`dir/` trailing slash syntax). |
| `focal context [paths...]` | Generates a high-level project summary (tree, git status, manifests) with optional pre-selected files/dirs. |
| `focal tree` | Generates and copies the repository directory tree, ignoring `.git` and build caches. |
| `focal api` | Extracts an overview of Python classes and functions. |
| `focal search` | Searches the codebase for a regex pattern and copies the results with surrounding context. |
| `focal symbol` | Locates files containing a specific symbol and copies their full contents. |
| `focal web` | Extracts semantic markdown from public URLs or piped HTML, stripping DOM noise. |

### Git & GitHub Extraction

| Command | Description |
| --- | --- |
| `focal wip-context` | Extracts a comprehensive diff, uncommitted status, and topological context for your active branch. |
| `focal diff` | Copies a formatted git diff of uncommitted, staged (`--staged`), or all (`--all`) changes, including status and summary context. |
| `focal pr-diff` | Fetches metadata, description intent, and the full code diff for a GitHub Pull Request. |
| `focal issues` | Interactively selects and copies GitHub issue descriptions alongside their comment threads. |
| `focal release-context` | Copies exact release membership, PR intent, changed paths, and net impact; supports focused diffs and configurable bot detail. |
| `focal ci-fail` | Fetches and formats GitHub Actions CI failure logs for debugging. |

---

## 💡 How It Works

Focal is one Rust executable with no runtime dependencies. File discovery, ignore rules, search, Git, the fuzzy picker, HTML conversion, PDF and notebook extraction, and the GitHub API client all run in process, so commands start in milliseconds and behave the same on every machine.

- **Discovery and search** use the same engines as `fd` and `ripgrep` (the `ignore` and `grep` crates), honoring `.gitignore`, `.ignore`, `.fdignore`, and `.rgignore`.
- **Git** goes through `libgit2`, so `diff`, `wip-context`, `release-context`, and `--repo` need no `git` executable.
- **The picker** is a built-in terminal UI with fuzzy filtering, multi-select, and a live preview that never blocks typing.
- **GitHub commands** talk to the REST and GraphQL APIs directly. They read a token from `GH_TOKEN` or `GITHUB_TOKEN` (`GH_ENTERPRISE_TOKEN` for GitHub Enterprise hosts), falling back to `gh auth token` if you are signed in to GitHub CLI.

A few principles shape the output itself:

- **High signal, low noise.** Binary blobs, lockfiles, minified assets, and DOM cruft are filtered out, and token ceilings prevent prompt overflow, so your LLM's attention budget goes to code and text that matter.
- **Clipboard-first.** With a terminal on stdout, every command copies its payload to the clipboard (natively on macOS; through `wl-copy`, `xclip`, or `xsel` on Linux). Redirect or pipe stdout and the payload goes there instead.
- **Clear formatting.** Files, Jupyter notebooks (`.ipynb`), PDF documents, diffs, GitHub API responses, and web pages are wrapped in consistent Markdown blocks whose fences never collide with their content.
- **Actionable diagnostics.** Errors say what failed and why (for example, that a PDF is encrypted or has no text layer) and print a hint for fixing it. Output honors `NO_COLOR` and `TERM=dumb`.

---

## 📦 Detailed Installation & Autocompletion

### Building from Source

With a [Rust toolchain](https://rustup.rs) and the [`just`](https://github.com/casey/just) command runner:

```bash
git clone https://github.com/JacksonFergusonDev/focal.git
cd focal
just install
```

`just install` builds a release binary, installs it as `~/.local/bin/focal`, and writes Zsh completions to `~/.zsh/completions/_focal`. Make sure `~/.local/bin` is on your `PATH`. `cargo install --path .` also works and installs into `~/.cargo/bin`.

### Shell Autocompletion

`focal completions bash` and `focal completions zsh` print completion scripts for every command and flag.

**Zsh.** `just install` writes `~/.zsh/completions/_focal`. Add that directory to your `fpath` **before** `compinit` in `~/.zshrc`:

```zsh
fpath+=~/.zsh/completions

autoload -Uz compinit
compinit
```

For any other installation, generate the file yourself: `focal completions zsh > ~/.zsh/completions/_focal`.

**Bash.** Add this to `~/.bashrc`:

```bash
eval "$(focal completions bash)"
```

### Optional Integrations

- **Clipboard on Linux:** `wl-copy` (Wayland) or `xclip`/`xsel` (X11). Without one, Focal explains why and prints the payload to stdout.
- **GitHub CLI:** only used as a credential source (`gh auth token`) when no token is set in the environment.

---

## 🤝 Collaboration

Focal is written in Rust (edition 2024), formatted with `rustfmt`, and linted with Clippy (`-D warnings`). The test suite combines unit tests with CLI contract tests that run the real binary against temporary Git repositories, a local fake of the GitHub API, and a pseudo-terminal for the interactive pickers. CI runs it natively on macOS and Linux for both arm64 and x64.

To run the complete local CI pipeline before submitting a pull request:

```bash
just ci
```

Please feel free to open an issue or PR if you'd like to see a specific context extractor added to the suite.

---

## Contact

[![GitHub](https://img.shields.io/badge/GitHub-100000?style=for-the-badge&logo=github&logoColor=white)](https://github.com/JacksonFergusonDev)
[![LinkedIn](https://img.shields.io/badge/LinkedIn-0077B5?style=for-the-badge&logo=linkedin&logoColor=white)](https://www.linkedin.com/in/jackson--ferguson/)
[![Email](https://img.shields.io/badge/Email-D14836?style=for-the-badge&logo=gmail&logoColor=white)](mailto:jackson.ferguson0@gmail.com)

[![Website](https://raw.githubusercontent.com/JacksonFergusonDev/JacksonFergusonDev.github.io/refs/heads/main/.github/assets/badge.svg)](https://jacksonferguson.me)

---

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
