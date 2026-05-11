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

**CLI utilities for AI-assisted development context and workflow automation.**

[![Version](https://img.shields.io/github/v/release/JacksonFergusonDev/focal?style=flat-square&labelColor=0A0A0A&color=fb923c)](https://github.com/JacksonFergusonDev/focal/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/ci.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=CI)](https://github.com/JacksonFergusonDev/focal/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/actions/workflow/status/JacksonFergusonDev/focal/release.yml?style=flat-square&color=fb923c&labelColor=0A0A0A&label=release)](https://github.com/JacksonFergusonDev/focal/actions/workflows/release.yml)
[![Python](https://img.shields.io/badge/python-3.10+-fb923c?style=flat-square&labelColor=0A0A0A)](https://www.python.org/downloads/)
[![Ruff](https://img.shields.io/badge/style-ruff-fb923c?style=flat-square&labelColor=0A0A0A)](https://github.com/astral-sh/ruff)
[![Mypy](https://img.shields.io/badge/mypy-checked-fb923c?style=flat-square&labelColor=0A0A0A)](https://mypy-lang.org/)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-fb923c?style=flat-square&labelColor=0A0A0A)](https://github.com/pre-commit/pre-commit)
[![License](https://img.shields.io/badge/license-MIT-fb923c?style=flat-square&labelColor=0A0A0A)](LICENSE)

</div>

Gathering context for an LLM (whether ChatGPT, Claude, or Gemini) usually means copying and pasting multiple files, scraping git diffs, and manually formatting terminal outputs. Focal automates this boilerplate repository extraction so you can stay in flow and feed your AI assistant exactly what it needs to understand your codebase or external documentation.

## 🚀 Quick Start

The fastest way to install Focal on macOS or Linux is via Homebrew:

```bash
brew install jacksonfergusondev/tap/focal
```

*See [Detailed Installation & Autocompletion](#-detailed-installation--autocompletion) below for building from source and configuring Zsh/Bash.*

---

## 💡 Design Philosophy

Focal is built to maximize LLM attention window efficiency and stay out of your way. It adheres to a strict pipeline-native architecture to avoid generating bloated, noisy context blocks:

1. **High Signal, Low Noise:** When generating branch topologies or diffs, Focal aggressively filters out binary blobs, lockfiles, minified assets, and DOM noise using strict heuristic sets. It reserves token bandwidth for the source code and semantic text that actually matters.

1. **Clipboard-First Execution:** Outputs are automatically calculated for token length and piped directly to your system's native clipboard manager (`pbcopy`, `wl-copy`, `xclip`, or `xsel`). No intermediate files; just run the command and paste.

1. **Pipeline Native:** Core routing and file manipulation are handled by ultra-fast UNIX utilities (`rg`, `fd`, `fzf`, `bat`). Python is strictly reserved as an asynchronous backend to handle complex data transformations, like parsing Jupyter Notebook ASTs, resolving Git commit topologies, or stripping HTML structure.

1. **Clear Formatting:** LLMs need structural boundaries. Files, diffs, GitHub API responses, web pages, and CI logs are automatically wrapped in LLM-optimized Markdown blocks, ensuring the model understands file paths, language semantics, and context hierarchies.

1. **Fail Loud, Fail Early:** Pre-flight checks ensure all system dependencies are present before any context generation is attempted. If a required binary is missing, Focal aborts cleanly.

---

## ⚡️ Performance & Latency Isolation

Focal is built to be lightweight, utilizing bash wrappers to defer Python's startup overhead until strictly necessary.

1. **Fast-Path Execution:** Commands like `focal search`, `focal file`, or `focal tree` execute entirely via compiled binaries like `ripgrep` or `tree`/`fd`, meaning time-to-clipboard is measured in milliseconds.

1. **Heavy-Path Execution:** For complex extractions like `focal wip-context`, `focal web`, or `focal ci-fail`, the bash layer dynamically resolves a localized `uv` virtual environment to execute the Python backend, ensuring global namespace isolation without sacrificing execution speed.

Our CI pipeline enforces strict linting and type-checking across both the Python (`ruff`, `mypy`, `pytest`) and Shell (`shellcheck`, `shfmt`, `bats`) stacks to guarantee architectural stability.

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

## 🚀 Usage

Focal is designed to be run from anywhere inside a valid Git workspace, but also handles external URLs and clipboard streams.

### Basic Context Gathering

If you need to feed a specific file or set of files to an LLM, run the interactive selectors:

```bash
# Select a single file (uses fzf to search, bat to preview)
focal file

# Multi-select files (Tab to select, Enter to confirm)
focal files
```

### Git & Working Tree

To grab the exact state of your current feature branch (uncommitted changes, commit topology, microscopic diffs) compared to `main`:

```bash
focal wip-context
```

To quickly grab the diff of your currently staged or uncommitted files:

```bash
focal diff --staged
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

To aggregate a release context by compiling all pull request intents merged since the last Git tag:

```bash
focal release-context
```

---

## 🛠 Command Reference

### Local & Web Context Gathering

| Command | Description |
| --- | --- |
| `focal file` | Interactively selects a single file and copies its formatted contents. |
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
| `focal diff` | Copies a formatted git diff of uncommitted or staged (`--staged`) changes. |
| `focal pr-diff` | Fetches metadata, description intent, and the full code diff for a GitHub Pull Request. |
| `focal issue-graph` | Copies a GitHub issue description alongside its sequential comment thread. |
| `focal release-context` | Copies metadata and intent for all PRs merged since the last release tag. |
| `focal ci-fail` | Fetches and formats GitHub Actions CI failure logs for debugging. |

---

## ⚙️ Dependencies & Toolchain

Focal orchestrates several industry-standard CLI tools to achieve low-latency extraction. While Focal will fail gracefully if one is missing, installing the following is highly recommended:

- **`fzf`**: Required for interactive fuzzy-finding interfaces.
- **`fd`**: Required for high-speed file traversal (respects `.gitignore`).
- **`bat`**: Required for syntax-highlighted TUI previews.
- **`rg` (ripgrep)**: Required for the `search`, `api`, and `symbol` regex extractors.
- **`gh` (GitHub CLI)**: Required for `pr-diff`, `issue-graph`, `ci-fail`, and `release-context`.

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
