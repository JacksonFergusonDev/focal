set shell := ["bash", "-euc", "-o", "pipefail"]
set unstable
set quiet

# --- ANSI Colors ---

blue := '\033[1;34m'
green := '\033[1;32m'
yellow := '\033[1;33m'
nc := '\033[0m'

# --- Paths ---

prefix := env("HOME") + "/.local/bin"
zsh_comp_dir := env("HOME") + "/.zsh/completions"

# Show available commands
default:
    @just --list

# Build a release binary, install it as ~/.local/bin/focal, and install Zsh completions
install: build
    @printf "\n{{ blue }}=== Installing Focal ==={{ nc }}\n"
    mkdir -p {{ prefix }} {{ zsh_comp_dir }}
    # Replace any symlink left by the old Bash dispatcher instead of writing through it.
    rm -f "{{ prefix }}/focal"
    install -m 755 target/release/focal "{{ prefix }}/focal"
    "{{ prefix }}/focal" completions zsh > "{{ zsh_comp_dir }}/_focal"
    rm -f ~/.zcompdump*
    @printf "{{ green }}✔ Installed {{ prefix }}/focal ($("{{ prefix }}/focal" --version)){{ nc }}\n"
    @printf "{{ yellow }}! If completions don't work immediately, restart your terminal.{{ nc }}\n"

# Remove the installed binary and completions
uninstall:
    @printf "\n{{ blue }}=== Uninstalling Focal ==={{ nc }}\n"
    rm -f "{{ prefix }}/focal" "{{ prefix }}/focal-rs" "{{ zsh_comp_dir }}/_focal"
    rm -f ~/.zcompdump*
    @printf "{{ green }}✔ Focal and completions removed.{{ nc }}\n"

# Auto-format Rust and Markdown
format:
    @printf "\n{{ blue }}=== Formatting Code ==={{ nc }}\n"
    cargo fmt --all
    rumdl fmt .
    @printf "{{ green }}✔ Formatting complete{{ nc }}\n"

# Run linters and check formatting (Clippy, rustfmt, and Markdown)
lint:
    @printf "\n{{ blue }}=== Running Linters ==={{ nc }}\n"
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    rumdl check .
    rumdl fmt --check .
    @printf "{{ green }}✔ Linting passed{{ nc }}\n"

# Run the unit and CLI contract tests
test:
    @printf "\n{{ blue }}=== Running Tests ==={{ nc }}\n"
    cargo test --all-targets
    @printf "{{ green }}✔ All tests passed{{ nc }}\n"

# Run tests with LLVM coverage (requires cargo-llvm-cov)
test-cov:
    @printf "\n{{ blue }}=== Running Tests with Coverage ==={{ nc }}\n"
    cargo llvm-cov --all-targets --fail-under-lines 80
    @printf "{{ green }}✔ Coverage run complete{{ nc }}\n"

# Build an optimized release binary
build:
    @printf "\n{{ blue }}=== Building Release Binary ==={{ nc }}\n"
    cargo build --release
    @printf "{{ green }}✔ Build complete{{ nc }}\n"

# Build the PyPI wheel locally into dist/ (requires uv)
wheel:
    @printf "\n{{ blue }}=== Building Wheel ==={{ nc }}\n"
    uvx maturin build --release --out dist
    @printf "{{ green }}✔ Wheel written to dist/{{ nc }}\n"

# Run the exact pipeline executed by GitHub Actions
ci: lint test
    @printf "\n{{ green }}✔ Local CI pipeline completed successfully. Clear to push!{{ nc }}\n"

# Preview the release notes for unreleased commits
changelog:
    uvx git-cliff@2.14.2 --unreleased --strip all

# Bump project version (part: major, minor, patch), sync lockfile, commit, tag, and atomic push
bump part:
    uv run --refresh https://raw.githubusercontent.com/JacksonFergusonDev/ci-cd-release-infrastructure/refs/heads/main/scripts/release.py {{ part }}

# Remove build artifacts and temporary files
clean:
    @printf "\n{{ blue }}=== Cleaning Workspace ==={{ nc }}\n"
    cargo clean
    rm -rf \
        dist \
        target \
        coverage.lcov \
        lcov.info \
        *.profraw
    @printf "{{ green }}✔ Workspace cleaned{{ nc }}\n"
