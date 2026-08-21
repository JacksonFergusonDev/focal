<!-- markdownlint-disable-file MD041 -->
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

[![Version](https://img.shields.io/github/v/release/JacksonFergusonDev/focal?style=flat-square&labelColor=0A0A0A&color=fb923c)](https://github.com/JacksonFergusonDev/focal/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/ci.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=CI)](https://github.com/JacksonFergusonDev/focal/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/release.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=release)](https://github.com/JacksonFergusonDev/focal/actions/workflows/release.yml)
[![Python](https://img.shields.io/badge/python-3.10+-fb923c?style=flat-square&labelColor=0A0A0A)](https://www.python.org/downloads/)
[![Ruff](https://img.shields.io/badge/style-ruff-fb923c?style=flat-square&labelColor=0A0A0A)](https://github.com/astral-sh/ruff)
[![Mypy](https://img.shields.io/badge/mypy-checked-fb923c?style=flat-square&labelColor=0A0A0A)](https://mypy-lang.org/)
[![prek](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/j178/prek/master/docs/assets/badge-v0.json&style=flat-square&labelColor=0A0A0A&color=fb923c)](https://github.com/j178/prek)
[![License](https://img.shields.io/badge/license-MIT-fb923c?style=flat-square&labelColor=0A0A0A)](LICENSE)

</div>

You know the drill: you want a second opinion on your plan for a refactor, so you open ChatGPT, alt-tab back to your editor, select a few files, copy them in one at a time, scroll back up to grab the git diff, and hope you didn't forget any necessary files. By the time the context is pasted in, you've spent more time assembling it than the LLM spends answering.

Focal replaces that whole ritual with a single command. Point it at a diff, a branch, a GitHub issue, a CI failure, or a web page, and it hands back clean, LLM-formatted markdown — already on your clipboard, ready to paste into `chatgpt.com`, `claude.ai`, `gemini.google.com`, or wherever you're working.

---

## 🚀 Quick Start

The fastest way to install Focal on macOS or Linux is via Homebrew:

```bash
brew install jacksonfergusondev/tap/focal
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

If you need to feed a specific file or set of files to an LLM, run the interactive selector:

```bash
# Interactively select files (Tab to multi-select, Enter to confirm)
focal files
```

### Git & Working Tree

To grab the exact state of your current feature branch (uncommitted changes, commit topology, microscopic diffs) compared to `main`:

```bash
focal wip-context
```

To quickly grab the diff of your currently staged or uncommitted files (or use `--all` to include untracked files):

```bash
focal diff          # Unstaged changes
focal diff --staged # Staged changes
focal diff --all    # All changes (staged, unstaged, and untracked)
```

### Web & Documentation Context

You can pipe a library's documentation page directly to your clipboard, stripped of all DOM noise (navbars, scripts, footers):

```bash
# Fetch from a public URL
focal web https://docs.python.org/3/

# Parse an authenticated dashboard copied to your clipboard
pbpaste | focal web
```

### GitHub API Context

Need to debug why your GitHub Actions pipeline crashed? Grab the error logs and metadata instantly:

```bash
focal ci-fail
```

To aggregate a release context by compiling all pull request intents and raw commit history since the last Git tag (or specify `minor` / `major` to synthesize across versions):

```bash
# Context since the most recent release tag (default)
focal release-context

# Synthesize changes across patch bumps since the last minor release
focal release-context minor
```

---

## 🛠 Command Reference

### Local & Web Context Gathering

| Command | Description |
| --- | --- |
| `focal files` | Interactively (or via glob) selects multiple files and copies contents + metadata. |
| `focal context` | Generates a high-level project summary (tree, git status, dependency manifests). |
| `focal tree` | Generates and copies the repository directory tree, ignoring `.git` and build caches. |
| `focal api` | Extracts an overview of Python classes and functions using `ripgrep`. |
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
| `focal release-context` | Copies metadata, PR intent, and raw commit history since the last release tag (supports `minor`, `major`, `patch`). |
| `focal ci-fail` | Fetches and formats GitHub Actions CI failure logs for debugging. |

---

## 💡 How It Works

Focal is a thin bash dispatcher (`bin/focal`) that routes each subcommand to a script in `libexec/`. The split keeps things fast:

- **Fast-path commands** — `focal search`, `focal files`, `focal tree` — run entirely through compiled binaries (`ripgrep`, `fd`, `fzf`), so time-to-clipboard is measured in milliseconds.
- **Heavy-path commands** — `focal wip-context`, `focal web`, `focal ci-fail` — hand off to a Python backend for tasks like parsing notebook ASTs, resolving git commit topologies, or stripping HTML DOM noise.

A few principles shape the output itself:

- **High signal, low noise.** Binary blobs, lockfiles, minified assets, and DOM cruft are aggressively filtered out, so your LLM's attention budget goes to code and text that actually matters.
- **Clipboard-first.** Every command writes straight to your system clipboard (`pbcopy`, `wl-copy`, `xclip`, or `xsel`) — no intermediate files, no extra steps.
- **Clear formatting.** Files, diffs, GitHub API responses, and web pages are wrapped in consistent, LLM-optimized markdown blocks, so the model isn't guessing at file paths or context boundaries.
- **Fail loud, fail early.** Missing dependencies are caught in a pre-flight check before any context generation starts.

CI enforces strict linting and type-checking across both stacks — `ruff`, `mypy`, `pytest` for Python, and `shellcheck`, `shfmt`, `bats` for shell — to keep the tool itself dependable.

---

## 📦 Detailed Installation & Autocompletion

### Building from Source

If you prefer to compile and install locally instead of using Homebrew, ensure you have the [`uv`](https://github.com/astral-sh/uv) package manager and the [`just`](https://github.com/casey/just) command runner installed on your system.

```bash
git clone https://github.com/JacksonFergusonDev/focal.git
cd focal
just install
```

*Note: The `just install` pipeline resolves a localized Python environment and symlinks the entrypoint binary into `~/.local/bin`. Ensure this directory is prioritized in your system `$PATH`.*

---

### Shell Autocompletion

Focal supports native shell autocompletion for fast subcommand routing.

#### 1. Zsh

**If installed via Homebrew:**

Homebrew automatically links the completion scripts to its internal `site-functions` directory during installation. You only need to ensure `compinit` is initialized in your `~/.zshrc`:

```zsh
autoload -Uz compinit
compinit
```

**If installed from source (`just install`):**

The `just install` command automatically symlinks the completion script to `~/.zsh/completions/_focal` and clears your active `zcompdump` cache. To activate it, ensure your `~/.zshrc` appends that directory to your `fpath` **before** loading `compinit`:

```zsh
# Add this above your compinit calls in ~/.zshrc
fpath+=~/.zsh/completions

autoload -Uz compinit
compinit
```

#### 2. Bash

To enable Bash completions, you need to source the included `focal.bash` script. Point directly to the completion file in your `~/.bashrc` or `~/.bash_profile`:

```bash
source ~/.local/share/focal/completions/focal.bash # Adjust path if cloned elsewhere
```

---

## ⚙️ Dependencies & Toolchain

Focal orchestrates several industry-standard CLI tools to achieve low-latency extraction. Homebrew will ensure these are installed on your system. While Focal will fail gracefully if one is missing, installing the following is highly recommended:

- **`fzf`**: Required for interactive fuzzy-finding interfaces.
- **`fd`**: Required for high-speed file traversal (respects `.gitignore`).
- **`bat`**: Required for syntax-highlighted TUI previews.
- **`rg` (ripgrep)**: Required for the `search`, `api`, and `symbol` regex extractors.
- **`gh` (GitHub CLI)**: Required for `pr-diff`, `issues`, `ci-fail`, and `release-context`.

---

## 🤝 Collaboration

This repository utilizes a dual-language testing and linting architecture.

- **Python:** 100% type-hinted via `mypy`, formatted with `ruff`, and tested with `pytest`. Parsing pipelines utilize `beautifulsoup4` and `markdownify` for AST and DOM manipulation.
- **Bash:** Strictly linted via `shellcheck`, formatted with `shfmt`, and behaviorally tested using the `bats` framework.

To run the complete local CI pipeline before submitting a pull request:

```bash
just ci
```

Please feel free to open an issue or PR if you'd like to see a specific context extractor added to the suite.

---

## 📧 Contact

[![GitHub](https://img.shields.io/badge/GitHub-100000?style=for-the-badge&logo=github&logoColor=white)](https://github.com/JacksonFergusonDev)
[![LinkedIn](https://img.shields.io/badge/LinkedIn-0077B5?style=for-the-badge&logo=linkedin&logoColor=white)](https://www.linkedin.com/in/jackson--ferguson/)
[![Email](https://img.shields.io/badge/Email-D14836?style=for-the-badge&logo=gmail&logoColor=white)](mailto:jackson.ferguson0@gmail.com)

---

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
