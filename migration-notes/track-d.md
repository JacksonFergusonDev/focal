# Track D: `--repo` cached checkouts

## Implemented

`--repo <url>` prepares a cached checkout in process with `git2` and changes into it before dispatch, so it works with an empty PATH. The cache path is the legacy one, `${XDG_CACHE_HOME:-$HOME/.cache}/focal/repos/<basename>_<first 16 hex of sha256(url)>`, with `basename` following POSIX `basename "$url" .git`, so legacy entries are reused in place. A first use clones into a hidden scratch directory (`.tmp-<pid>-<entry>`), writes `.git/.focal_created` and `.git/.focal_accessed`, and renames it into place, so an entry only appears complete. Later uses fetch every remote, read origin's advertised default branch on the same connection, point `origin/HEAD` at it, and hard-reset the checkout to it. When the default branch was renamed, the checkout moves to a local branch of the new name tracking `origin/<name>`. When the fetch fails, Focal prints legacy's warning with the reason on an indented line and reuses the checkout. Status lines match legacy (`info: Cloning remote repository into cache...`, `info: Updating cached remote repository...`, `warning: Could not update the cached repository; using the cached copy.`). With a terminal on stderr, transfer progress is drawn on one line and cleared.

Concurrency uses `std::fs::File` locks in `.locks/`. `<entry>.lock` is held exclusively while an entry is cloned or updated, so two cold-cache invocations serialize: one clones, the other waits (saying so) and then updates the finished entry. `<entry>.use` is held shared from before the update until the process exits. Cleanup only removes an entry whose two locks it can take without waiting, and never the entry the current process uses. Leftover scratch directories of an entry are removed under its update lock, so an interrupted or killed clone leaves no entry and the next run clones cleanly. An entry that libgit2 cannot open (for example what a killed legacy `git clone` leaves) is replaced by a fresh clone, swapped in by rename only when no other process is using it.

Cleanup keeps legacy's semantics: at most once a day (`.last_cleanup` holds epoch seconds), `FOCAL_REPO_TTL_DAYS` (default 30) whole days counted like `find -mtime +N`, entries aged by `.git/.focal_accessed` or, without it, by the directory mtime, and the same `Pruning stale/legacy cached repository` lines. It only considers `<basename>_<16 hex>` directories that contain `.git`. Deletion renames the entry to `.trash-<pid>-<entry>` first. An invalid TTL value disables cleanup (legacy's `find` would fail and prune nothing). Legacy dispatchers take no locks, so a legacy process running beside Rust is not protected.

Credentials for private remotes are independent of the GitHub API client: SSH tries the agent, then `~/.ssh/id_ed25519`, `id_ecdsa`, and `id_rsa`. HTTPS asks the Git credential helpers configured for the URL (`Cred::credential_helper` with the default Git config), then offers `GH_TOKEN` or `GITHUB_TOKEN` as `x-access-token` to `https://github.com` only. Each source is offered once, so a rejected credential fails instead of looping. Credentials are never printed, and any `user:password@` from the URL is redacted from error messages.

### Clone strategy

libgit2 cannot make partial clones, so Rust makes full clones. Fixture: a generated bare repository with 2,000 commits over 4,000 text files (15 files changed per commit), a 142 MB pack, served on localhost by `git daemon` (git://) to model a smart server. Apple Silicon macOS, three samples, median:

| Strategy | First clone | `.git` size |
| --- | --- | --- |
| Git CLI full clone | 7.84 s | 142 MB |
| Git CLI `--filter=blob:none` (legacy) | 1.69 s | 23 MB |
| Git CLI `--depth=1` | 0.99 s | 17 MB |
| Rust `--repo … tree`, git2 full clone | 3.05 s | 142 MB |

A warm `--repo … tree` with nothing to fetch took 0.14 s. Over a real network the full clone's cost is dominated by transfer: about six times the bytes of legacy's blob-filtered clone on this history-heavy fixture (repositories with less churn differ less). Over `file://`, libgit2 builds the pack in process and the same clone took 25 s against 2.4 s for the Git CLI; plain local paths use libgit2's local copy and are fast. Decision: keep full clones with git2 so `--repo` has no Git CLI dependency and keeps full history for history-reading commands. Cold-clone cost is paid once per URL per TTL.

### Legacy partial clones

libgit2 1.9 refuses to open repositories with `extensions.partialclone` and cannot fetch missing blobs on demand. A legacy blob-filtered entry is therefore replaced with a full clone on its first online use (one full-clone cost, then normal updates). Offline, Focal registers the `partialclone` extension for the process and reuses the checkout, whose working tree and HEAD objects are present, so commands and the Git header work.

## Intentional differences

| Invocation | Legacy | Rust |
| --- | --- | --- |
| `--repo <url>` after the remote renamed its default branch | Resets the old local branch (for example `main`) to the new default, so the header still says `Git Branch: main` | Checks out a local branch named after the new default (`test_default_branch_rename_switches_the_local_branch`) |
| `--repo <url>` with a damaged cache entry (a killed `git clone`) | Fetch fails, warns, and runs in the broken checkout | Clones again and swaps the entry in (`test_damaged_entry_is_cloned_again`) |
| Cleanup with a directory such as `notes/` older than the TTL in the cache | Deletes it | Only considers `<basename>_<16 hex>` entries with `.git` (`test_cleanup_spares_directories_focal_does_not_own`) |
| Cleanup while another process works in a stale entry | Deletes it | Skips entries whose use lock is held (`test_cleanup_spares_an_entry_in_use`) |
| Two simultaneous cold-cache invocations | Both run `git clone` into the same path; one fails | One clones, the other waits and updates (`test_simultaneous_cold_invocations_share_one_clone`, skipped under legacy because the outcome is a race) |
| A killed first clone | Can leave a partial entry | Leaves only a scratch directory, removed by the next run (`test_killed_clone_is_retried_cleanly`, skipped under legacy for the same reason) |
| Legacy blob-filtered entry, online | Fetches into the partial clone | Replaces it with a full clone (asserted in `test_legacy_partial_entry_is_updated` under Rust only) |
| Offline update | Git's `fatal:` lines, then the warning | The warning, then `reason: <libgit2 message>` |
| First clone of a large repository | Git's progress output | A one-line progress counter on a terminal, nothing otherwise |

## Not done or not verified

- Private HTTPS and SSH remotes were not exercised against a real server: there is no network fixture for authentication. The credential order is unit-tested only for its host gate and redaction.
- The `PWD:` header line shows the physical cache path (`std::env::current_dir`), while legacy subcommands print the shell's logical `$PWD`; they differ only when the cache path contains a symlink.
- Decision: optionally use the Git CLI for `--filter=blob:none` clones when it is on PATH (falling back to git2), or a git2 shallow clone (`depth=1`, about 17 MB here) if history is not needed by `--repo` users. Neither is implemented.
- Lock files in `.locks/` are never deleted (removing a lock file another process may have open is racy); they are empty.
