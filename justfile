set shell := ["bash", "-euc", "-o", "pipefail"]
set unstable
set quiet

# --- ANSI Colors ---

blue := '\033[1;34m'
green := '\033[1;32m'
yellow := '\033[1;33m'
cyan := '\033[1;36m'
nc := '\033[0m'

# --- Paths ---

prefix := env("HOME") + "/.local/bin"
focal_bin := justfile_directory() + "/bin/focal"
zsh_comp_dir := env("HOME") + "/.zsh/completions"
focal_comp := justfile_directory() + "/completions/_focal"

# Show available commands
default:
    @just --list

# Install the package to libexec, symlink the dispatcher, and setup completions
install:
    @printf "\n{{ blue }}=== Installing Focal ==={{ nc }}\n"
    @printf "{{ cyan }}Syncing uv virtual environment...{{ nc }}\n"
    uv sync
    @printf "{{ cyan }}Asserting executable permissions...{{ nc }}\n"
    chmod +x bin/focal libexec/* lib/*.sh 2>/dev/null || true
    @printf "{{ cyan }}Linking dispatcher to {{ prefix }}...{{ nc }}\n"
    mkdir -p {{ prefix }}
    ln -sf "{{ focal_bin }}" "{{ prefix }}/focal"
    @printf "{{ cyan }}Installing Zsh completions...{{ nc }}\n"
    mkdir -p {{ zsh_comp_dir }}
    ln -sf "{{ focal_comp }}" "{{ zsh_comp_dir }}/_focal"
    rm -f ~/.zcompdump*
    @printf "{{ green }}✔ Install complete. Ensure {{ prefix }} is in your PATH.{{ nc }}\n"
    @printf "{{ yellow }}! If completions don't work immediately, restart your terminal.{{ nc }}\n"

# Remove the dispatcher symlink and autocompletions
uninstall:
    @printf "\n{{ blue }}=== Uninstalling Focal ==={{ nc }}\n"
    rm -f "{{ prefix }}/focal"
    rm -f "{{ zsh_comp_dir }}/_focal"
    rm -f ~/.zcompdump*
    @printf "{{ green }}✔ Focal and completions removed.{{ nc }}\n"

# Completely remove and reinstall focal
reset: uninstall install

# Auto-format Python code and Shell scripts
format:
    @printf "\n{{ blue }}=== Formatting Code ==={{ nc }}\n"
    uv run ruff check --fix .
    uv run ruff format .
    if command -v shfmt >/dev/null 2>&1; then \
        shfmt -w -s -i 2 -ci bin/* libexec/* lib/* 2>/dev/null || true; \
    else \
        printf "{{ yellow }}⚠ shfmt not found. Skipping shell formatting.{{ nc }}\n"; \
    fi
    @printf "{{ green }}✔ Formatting complete{{ nc }}\n"

# Run all linters (Ruff, Markdown, Shellcheck)
lint:
    @printf "\n{{ blue }}=== Running Linters ==={{ nc }}\n"
    uv run ruff check .
    uv run ruff format --check .
    if command -v markdownlint-cli2 >/dev/null 2>&1; then \
        markdownlint-cli2 "**/*.md"; \
    else \
        printf "{{ yellow }}⚠ markdownlint-cli2 not found. Skipping.{{ nc }}\n"; \
    fi
    if command -v shellcheck >/dev/null 2>&1; then \
        find bin libexec lib -maxdepth 1 -type f ! -name "*.json" -exec shellcheck --severity=warning {} +; \
    else \
        printf "{{ yellow }}⚠ shellcheck not found. Skipping shell linting.{{ nc }}\n"; \
    fi
    @printf "{{ green }}✔ Linting passed{{ nc }}\n"

# Run static type checking with Mypy
typecheck:
    @printf "\n{{ blue }}=== Running Type Checks ==={{ nc }}\n"
    uv run mypy .
    @printf "{{ green }}✔ Type checking passed{{ nc }}\n"

# Run python and bash testing
test:
    @printf "\n{{ blue }}=== Running Tests ==={{ nc }}\n"
    uv run pytest
    bats tests/bash/
    @printf "{{ green }}✔ All tests passed{{ nc }}\n"

# Run pytest with coverage
test-cov:
    @printf "\n{{ blue }}=== Running Tests with Coverage ==={{ nc }}\n"
    uv run pytest --cov
    @printf "{{ green }}✔ Coverage run complete{{ nc }}\n"

# Generate detailed coverage reports
test-cov-report:
    @printf "\n{{ blue }}=== Generating Coverage Reports ==={{ nc }}\n"
    uv run pytest --cov --cov-report=term-missing --cov-report=annotate:coverage_annotations/ | tee coverage_report.txt
    @printf "{{ green }}✔ Coverage reports generated{{ nc }}\n"

# Run the exact pipeline executed by CI
ci: lint typecheck test
    @printf "\n{{ green }}✔ Local CI pipeline completed successfully. Clear to push!{{ nc }}\n"

# Remove caches, artifacts, and temp files
clean:
    @printf "\n{{ blue }}=== Cleaning Workspace ==={{ nc }}\n"
    rm -rf \
        .pytest_cache \
        .mypy_cache \
        .ruff_cache \
        htmlcov \
        .coverage \
        coverage.xml \
        coverage_annotations \
        .cache
    rm -f coverage_report.txt
    find . -type d -name "__pycache__" -exec rm -rf {} +
    @printf "{{ green }}✔ Workspace cleaned{{ nc }}\n"

# --- Rust binary (preview until cutover; `just install` stays the legacy layout) ---

rust_bin := justfile_directory() + "/target/release/focal"

# Build the optimized Rust binary
rust-build:
    cargo build --locked --release

# Check formatting, lint, unit tests, and the parity suite against both executables
rust-test: rust-build
    @printf "\n{{ blue }}=== Testing the Rust binary ==={{ nc }}\n"
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    @printf "{{ cyan }}Parity suite against the legacy dispatcher...{{ nc }}\n"
    uv run pytest tests/migration -q
    @printf "{{ cyan }}Parity suite against the Rust binary, with nothing on PATH...{{ nc }}\n"
    FOCAL_TEST_EMPTY_PATH=1 FOCAL_TEST_BIN="{{ rust_bin }}" uv run pytest tests/migration -q
    @printf "{{ green }}✔ Rust checks passed{{ nc }}\n"

# Install the Rust binary as focal-rs alongside the legacy focal
rust-install: rust-build
    mkdir -p {{ prefix }}
    install -m 755 "{{ rust_bin }}" "{{ prefix }}/focal-rs"
    @printf "{{ green }}✔ Installed {{ prefix }}/focal-rs{{ nc }}\n"

# Remove the focal-rs binary
rust-uninstall:
    rm -f "{{ prefix }}/focal-rs"
    @printf "{{ green }}✔ Removed {{ prefix }}/focal-rs{{ nc }}\n"

# Build a release archive for this machine, as the release workflow does, into target/dist/
rust-package: rust-build
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(uv run --no-project scripts/release.py --check)"
    target="$(rustc -vV | sed -n 's/^host: //p')"
    name="focal-${version}-${target}"
    rm -rf "target/dist/${name}" "target/dist/${name}.tar.gz"
    mkdir -p "target/dist/${name}/completions"
    cp "{{ rust_bin }}" README.md LICENSE "target/dist/${name}/"
    "{{ rust_bin }}" completions bash > "target/dist/${name}/completions/focal.bash"
    "{{ rust_bin }}" completions zsh > "target/dist/${name}/completions/_focal"
    tar -C target/dist -czf "target/dist/${name}.tar.gz" "${name}"
    (cd target/dist && shasum -a 256 "${name}.tar.gz" | tee "${name}.tar.gz.sha256")

# --- Release ---

# Bump the version in pyproject.toml, uv.lock, Cargo.toml, and Cargo.lock, then commit, tag, and atomic push
bump part:
    uv run --no-project scripts/release.py {{ part }}

# Validate a version bump without changing anything
bump-dry-run part:
    uv run --no-project scripts/release.py {{ part }} --dry-run

# Print the version, failing if the version-carrying files disagree
version:
    uv run --no-project scripts/release.py --check
