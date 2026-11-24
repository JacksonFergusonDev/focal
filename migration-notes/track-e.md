# Track E: completions, release profile, and packaging

## Implemented

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

## Intentional differences

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `focal completions bash` / `zsh` | Unknown command; scripts are static files | Prints a script generated from the parser, completing flags as well as commands (tests in `test_completions.py` are `xfail(LEGACY, strict=True)`) |

## Not done, not verified, or needing a decision

- The workflows were linted with `actionlint` but not run. Runner labels `macos-15-intel` and `ubuntu-22.04-arm` and the pinned `upload-artifact` / `download-artifact` SHAs need a first `workflow_dispatch` run to confirm.
- The Homebrew formula still installs the legacy layout. Switching it to the Rust archives (and their generated completions) is the cutover step; the draft release gives a place to verify archives before that.
- `publish = false` stays. Enabling crates.io would need `description`, `license`, and `repository` metadata and an `include` list: `cargo package --list` currently ships the whole repository, including the legacy runtime and test fixtures.
- Static OpenSSL comes from the runner's Homebrew bottle (macOS) or `libssl-dev` (Linux). The Homebrew static libraries may be compiled for a newer macOS than 11, which the linker would report as a warning; the real floor needs checking on an old macOS. Enabling `openssl-sys`'s `vendored` feature (or disabling `git2`'s `ssh` feature if SSH remotes are not needed) would pin the OpenSSL build in `Cargo.lock` but changes dependencies, so it is left as a decision. An older glibc floor would need an older build image, a `zig`/`cross` toolchain, or a musl target.
- `rust-install` installs the binary only; the generated completion scripts are bound to the command name `focal`, so they are not installed for `focal-rs`.
