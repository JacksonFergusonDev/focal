//! Git transport for cached checkouts: clone, fetch, default-branch refresh, and reset,
//! with credentials from the user's Git configuration.

use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{
    Config, Cred, CredentialType, Direction, Error, FetchOptions, RemoteCallbacks, Repository,
    ResetType,
};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

const REMOTE_HEAD: &str = "refs/remotes/origin/HEAD";

/// Clones `url` into `dest`, which must not exist yet.
pub fn clone(url: &str, dest: &Path) -> Result<(), Error> {
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(url, true));
    let repo = RepoBuilder::new()
        .fetch_options(fetch)
        .clone(&clone_source(url), dest)?;
    finish_progress();
    // Record the remote's default branch as origin/HEAD, as `git clone` does.
    if repo.find_reference(REMOTE_HEAD).is_err()
        && let Ok(head) = repo.find_reference("HEAD")
        && let Some(branch) = head
            .symbolic_target()
            .and_then(|name| name.strip_prefix("refs/heads/"))
    {
        let tracking = format!("refs/remotes/origin/{branch}");
        if repo.find_reference(&tracking).is_ok() {
            repo.reference_symbolic(REMOTE_HEAD, &tracking, true, "clone: origin/HEAD")?;
        }
    }
    Ok(())
}

/// An existing local path becomes absolute so the stored `origin` does not depend on
/// the directory Focal was started from.
fn clone_source(url: &str) -> String {
    if !url.contains("://")
        && let Ok(path) = Path::new(url).canonicalize()
    {
        return path.to_string_lossy().into_owned();
    }
    url.to_owned()
}

pub fn open(entry: &Path) -> Result<Repository, Error> {
    Repository::open(entry)
}

/// Whether the checkout is a Git CLI partial clone (`--filter=blob:none`), which
/// libgit2 refuses to open and cannot fill in on demand.
pub fn is_partial(entry: &Path) -> bool {
    Config::open(&entry.join(".git").join("config")).is_ok_and(|config| {
        config.get_string("extensions.partialclone").is_ok()
            || config
                .entries(Some(r"remote\..*\.promisor"))
                .is_ok_and(|mut entries| {
                    let mut promisor = false;
                    while let Some(Ok(entry)) = entries.next() {
                        promisor |= entry.value() == Some("true");
                    }
                    promisor
                })
    })
}

/// Lets this process open partial clones, which is enough to read their checkout and
/// the objects they already hold.
pub fn allow_partial_clones() {
    // SAFETY: called on the main thread before any other thread uses libgit2.
    let _ = unsafe { git2::opts::set_extensions(&["partialclone"]) };
}

/// Fetches every remote, like `git fetch --all`, and returns origin's current default
/// branch (`refs/heads/<name>`) when the remote advertises one.
pub fn fetch(repo: &Repository) -> Result<Option<String>, Error> {
    let mut default = None;
    let names = repo.remotes()?;
    for name in names.iter().flatten() {
        let mut remote = repo.find_remote(name)?;
        let url = remote.url().unwrap_or_default().to_owned();
        let mut options = FetchOptions::new();
        options.remote_callbacks(callbacks(&url, true));
        if name == "origin" {
            let mut connection =
                remote.connect_auth(Direction::Fetch, Some(callbacks(&url, false)), None)?;
            default = connection
                .default_branch()
                .ok()
                .and_then(|branch| branch.as_str().map(str::to_owned));
            connection
                .remote()
                .fetch(&[] as &[&str], Some(&mut options), None)?;
        } else {
            remote.fetch(&[] as &[&str], Some(&mut options), None)?;
        }
        finish_progress();
    }
    Ok(default)
}

/// Points origin/HEAD at `default` (when known), then checks out the branch it names at
/// the remote commit, discarding local changes like `git reset --hard origin/HEAD`.
pub fn reset_to_default(repo: &Repository, default: Option<&str>) -> Result<(), Error> {
    if let Some(branch) = default
        && let Some(tracking) = tracking_ref(repo, branch)
        && repo.find_reference(&tracking).is_ok()
    {
        repo.reference_symbolic(REMOTE_HEAD, &tracking, true, "focal: refresh origin/HEAD")?;
    }
    let remote_head = repo.find_reference(REMOTE_HEAD)?;
    let commit = remote_head.resolve()?.peel_to_commit()?;
    let name = remote_head
        .symbolic_target()
        .and_then(|target| target.strip_prefix("refs/remotes/origin/"))
        .map(str::to_owned);

    let mut force = CheckoutBuilder::new();
    force.force();
    let local = name.as_ref().map(|name| format!("refs/heads/{name}"));
    let current = repo
        .find_reference("HEAD")?
        .symbolic_target()
        .map(str::to_owned);
    if let (Some(name), Some(local)) = (&name, &local)
        && current.as_deref() != Some(local)
    {
        // The default branch changed: switch to a local branch of the new name.
        repo.checkout_tree(commit.as_object(), Some(&mut force))?;
        repo.set_head_detached(commit.id())?;
        let mut branch = repo.branch(name, &commit, true)?;
        let _ = branch.set_upstream(Some(&format!("origin/{name}")));
        repo.set_head(local)?;
    }
    repo.reset(commit.as_object(), ResetType::Hard, Some(&mut force))
}

/// The remote-tracking ref origin's fetch refspecs map `branch` to.
fn tracking_ref(repo: &Repository, branch: &str) -> Option<String> {
    let remote = repo.find_remote("origin").ok()?;
    remote
        .refspecs()
        .filter(|spec| spec.direction() == Direction::Fetch && spec.src_matches(branch))
        .find_map(|spec| spec.transform(branch).ok()?.as_str().map(str::to_owned))
}

/// Transport callbacks: credentials from the user's Git configuration and, when
/// stderr is a terminal, transfer progress.
fn callbacks<'a>(url: &str, progress: bool) -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    let mut credentials = Credentials::new(url);
    callbacks.credentials(move |url, username, allowed| credentials.next(url, username, allowed));
    if progress && std::io::stderr().is_terminal() {
        callbacks.transfer_progress(|stats| {
            if stats.total_objects() > 0 {
                let done = stats.received_objects();
                let total = stats.total_objects();
                eprint!(
                    "\rReceiving objects: {:3}% ({done}/{total})",
                    done * 100 / total
                );
                let _ = std::io::stderr().flush();
            }
            true
        });
    }
    callbacks
}

fn finish_progress() {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K");
    }
}

/// Offers each credential source once, so a rejected credential ends the attempt
/// instead of looping:
///
/// - SSH: the agent, then the default identity files in `~/.ssh`.
/// - HTTPS: the Git credential helpers configured for the URL, then `GH_TOKEN` or
///   `GITHUB_TOKEN` for github.com only.
/// - Kerberos/NTLM: the platform default.
///
/// Credentials are handed to libgit2 and never printed.
struct Credentials {
    config: Option<Config>,
    token: Option<String>,
    ssh: Vec<SshSource>,
    helper_tried: bool,
    token_tried: bool,
    default_tried: bool,
    username_tried: bool,
}

enum SshSource {
    Agent,
    Key(PathBuf),
}

impl Credentials {
    fn new(url: &str) -> Self {
        let mut ssh = vec![SshSource::Agent];
        if let Some(home) = std::env::var_os("HOME") {
            for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
                let key = PathBuf::from(&home).join(".ssh").join(name);
                if key.is_file() {
                    ssh.push(SshSource::Key(key));
                }
            }
        }
        ssh.reverse();
        Self {
            config: Config::open_default().ok(),
            token: github_token(url),
            ssh,
            helper_tried: false,
            token_tried: false,
            default_tried: false,
            username_tried: false,
        }
    }

    fn next(
        &mut self,
        url: &str,
        username: Option<&str>,
        allowed: CredentialType,
    ) -> Result<Cred, Error> {
        if allowed.contains(CredentialType::USERNAME) && !self.username_tried {
            self.username_tried = true;
            return Cred::username(username.unwrap_or("git"));
        }
        if allowed.contains(CredentialType::SSH_KEY) {
            let user = username.unwrap_or("git");
            while let Some(source) = self.ssh.pop() {
                let cred = match source {
                    SshSource::Agent => Cred::ssh_key_from_agent(user),
                    SshSource::Key(path) => Cred::ssh_key(user, None, &path, None),
                };
                if cred.is_ok() {
                    return cred;
                }
            }
        }
        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
            if !self.helper_tried {
                self.helper_tried = true;
                if let Some(config) = &self.config
                    && let Ok(cred) = Cred::credential_helper(config, url, username)
                {
                    return Ok(cred);
                }
            }
            if !self.token_tried {
                self.token_tried = true;
                if let Some(token) = &self.token {
                    return Cred::userpass_plaintext("x-access-token", token);
                }
            }
        }
        if allowed.contains(CredentialType::DEFAULT) && !self.default_tried {
            self.default_tried = true;
            return Cred::default();
        }
        Err(Error::from_str(
            "the remote rejected or did not receive credentials",
        ))
    }
}

/// `GH_TOKEN` or `GITHUB_TOKEN`, offered only to github.com over HTTPS.
fn github_token(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split('/').next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

/// Removes any `user:password@` part of `url` from a message.
pub fn redact(message: &str, url: &str) -> String {
    let userinfo = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .and_then(|authority| authority.rsplit_once('@'))
        .map(|(userinfo, _)| userinfo)
        .filter(|userinfo| !userinfo.is_empty());
    match userinfo {
        Some(userinfo) => message.replace(userinfo, "***"),
        None => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_offered_only_to_github_over_https() {
        // The variables may be unset in the test environment; check the host gate.
        assert_eq!(github_token("https://gitlab.com/o/r.git"), None);
        assert_eq!(github_token("git@github.com:o/r.git"), None);
        assert_eq!(github_token("http://github.com/o/r.git"), None);
        assert_eq!(github_token("https://github.com.evil.example/o/r"), None);
    }

    #[test]
    fn credentials_in_urls_are_redacted() {
        assert_eq!(
            redact(
                "failed for https://me:s3cret@host/r",
                "https://me:s3cret@host/r"
            ),
            "failed for https://***@host/r"
        );
        assert_eq!(redact("plain", "https://host/r"), "plain");
    }
}
