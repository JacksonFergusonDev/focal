//! `--repo`: cached checkouts of remote repositories.
//!
//! Local bare repositories stand in for remotes, and XDG_CACHE_HOME points at a temporary
//! cache, so nothing touches the network or the user's cache.

mod common;

use common::*;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;

/// A bare repository on `main` plus a working clone used to push commits.
struct Remote {
    tmp: tempfile::TempDir,
    bare: PathBuf,
    work: PathBuf,
    url: String,
}

impl Remote {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let bare = root.join("remote.git");
        git(
            root,
            &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
        );
        git(&bare, &["config", "uploadpack.allowFilter", "true"]);
        let work = root.join("upstream");
        git(
            root,
            &[
                "clone",
                "-q",
                bare.to_str().unwrap(),
                work.to_str().unwrap(),
            ],
        );
        git(&work, &["checkout", "-q", "-b", "main"]);
        let url = format!("file://{}", bare.display());
        let remote = Self {
            tmp,
            bare,
            work,
            url,
        };
        remote.commit("README.md", "remote v1\n");
        remote
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn commit(&self, name: &str, contents: &str) {
        self.commit_on(name, contents, "main");
    }

    fn commit_on(&self, name: &str, contents: &str, branch: &str) {
        write(&self.work, name, contents);
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-qm", &format!("update {name}")]);
        git(
            &self.work,
            &["push", "-q", "origin", &format!("HEAD:{branch}")],
        );
    }

    /// Adds `count` files of `bytes` random bytes (hex-encoded) to make clones slow.
    fn bulk(&self, count: usize, bytes: usize) {
        let mut state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            | 1;
        for i in 0..count {
            let mut hex = String::with_capacity(bytes * 2);
            for _ in 0..bytes / 8 {
                // xorshift64: incompressible enough to keep the clone busy.
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                hex.push_str(&format!("{state:016x}"));
            }
            write(&self.work, &format!("bulk{i}.txt"), hex);
        }
        self.commit("README.md", "remote v1\n");
    }
}

fn cache_root(tmp: &Path) -> PathBuf {
    tmp.join("cache").join("focal").join("repos")
}

fn entry_path(tmp: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap();
    let base = last.strip_suffix(".git").unwrap_or(last);
    cache_root(tmp).join(format!("{base}_{}", &hex[..16]))
}

fn focal_env(tmp: &Path, extra: &[(&str, &str)]) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    let cache = tmp.join("cache");
    let base = env(&[
        ("PATH", ""),
        ("HOME", home.to_str().unwrap()),
        ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ("LC_ALL", "C.UTF-8"),
        ("NO_COLOR", "1"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("FOCAL_OMIT_WORKSPACE", "1"),
    ]);
    with(&base, extra)
}

fn elsewhere(tmp: &Path) -> PathBuf {
    let cwd = tmp.join("elsewhere");
    fs::create_dir_all(&cwd).unwrap();
    cwd
}

/// Runs focal from `tmp/elsewhere`; returns (exit code, stdout, stderr).
fn run(tmp: &Path, args: &[&str], extra: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let out = focal(&elsewhere(tmp), &focal_env(tmp, extra), args, None);
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

/// `focal <before> --repo <url> files README.md`.
fn readme(
    tmp: &Path,
    url: &str,
    before: &[&str],
    extra: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut args = before.to_vec();
    args.extend(["--repo", url, "files", "README.md"]);
    run(tmp, &args, extra)
}

fn readme_ok(tmp: &Path, url: &str) {
    let (code, _, err) = readme(tmp, url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
}

/// Sets a path's access and modification times to `days` ago.
fn age(path: &Path, days: u64) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - days * DAY;
    let time = libc::timeval {
        tv_sec: stamp as libc::time_t,
        tv_usec: 0,
    };
    let times = [time, time];
    let c = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) },
        0,
        "utimes {}",
        path.display()
    );
}

fn mtime_age(path: &Path) -> Duration {
    SystemTime::now()
        .duration_since(fs::metadata(path).unwrap().modified().unwrap())
        .unwrap_or_default()
}

/// Scratch directories (`.tmp-*`, `.trash-*`) left in the cache root.
fn scratch(tmp: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(cache_root(tmp))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tmp-") || n.starts_with(".trash-"))
        .collect();
    names.sort();
    names
}

fn spawn_focal(tmp: &Path, args: &[&str], piped: bool) -> Child {
    let stdio = || {
        if piped { Stdio::piped() } else { Stdio::null() }
    };
    Command::new(bin())
        .args(args)
        .current_dir(elsewhere(tmp))
        .env_clear()
        .envs(focal_env(tmp, &[]))
        .stdin(Stdio::null())
        .stdout(stdio())
        .stderr(stdio())
        .spawn()
        .expect("spawn focal")
}

/// Waits up to `timeout` for a piped child, killing it on expiry; returns
/// (exit code, stdout, stderr).
fn wait_piped(mut child: Child, timeout: Duration) -> (Option<i32>, String, String) {
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            text(&buf)
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("focal did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    (status.code(), out.join().unwrap(), err.join().unwrap())
}

/// Opens (creating and truncating) a lock file and applies `flock(op)`; returns the file
/// and whether the lock was taken.
fn flock(path: &Path, op: libc::c_int) -> (fs::File, bool) {
    let file = fs::File::create(path).unwrap();
    let rc = unsafe { libc::flock(file.as_raw_fd(), op) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        assert_eq!(
            err.raw_os_error(),
            Some(libc::EWOULDBLOCK),
            "flock failed unexpectedly: {err}"
        );
    }
    (file, rc == 0)
}

#[test]
fn cold_clone_runs_the_command_in_the_checkout() {
    let remote = Remote::new();
    let tmp = remote.path();
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("info: Cloning remote repository into cache...\n"),
        "{err}"
    );
    assert!(out.contains("remote v1"));
    let entry = entry_path(tmp, &remote.url);
    assert!(entry.join(".git/.focal_created").is_file());
    assert!(entry.join(".git/.focal_accessed").is_file());
}

#[test]
fn warm_update_picks_up_new_commits() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    let marker = entry_path(tmp, &remote.url).join(".git/.focal_accessed");
    age(&marker, 2);
    remote.commit("README.md", "remote v2\n");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("info: Updating cached remote repository...\n"),
        "{err}"
    );
    assert!(!err.contains("Cloning"));
    assert!(out.contains("remote v2"));
    assert!(mtime_age(&marker) < Duration::from_secs(DAY));
}

#[test]
fn local_edits_in_the_cache_are_discarded() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    write(&entry_path(tmp, &remote.url), "README.md", "tampered\n");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote v1"));
    assert!(!out.contains("tampered"));
}

fn repo_flag_on_either_side_of_the_command(args: &[&str]) {
    let remote = Remote::new();
    let args: Vec<String> = args
        .iter()
        .map(|a| a.replace("{url}", &remote.url))
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, out, err) = run(remote.path(), &args, &[]);
    assert_eq!(code, Some(0), "{args:?}: {err}");
    assert!(out.contains("remote v1"), "{args:?}");
}

#[test]
fn repo_flag_on_either_side_of_the_command_before_separate() {
    repo_flag_on_either_side_of_the_command(&["--repo", "{url}", "files", "README.md"]);
}

#[test]
fn repo_flag_on_either_side_of_the_command_before_equals() {
    repo_flag_on_either_side_of_the_command(&["--repo={url}", "files", "README.md"]);
}

#[test]
fn repo_flag_on_either_side_of_the_command_after_separate() {
    repo_flag_on_either_side_of_the_command(&["files", "--repo", "{url}", "README.md"]);
}

#[test]
fn repo_flag_on_either_side_of_the_command_after_equals() {
    repo_flag_on_either_side_of_the_command(&["files", "README.md", "--repo={url}"]);
}

#[test]
fn plain_path_remote() {
    let remote = Remote::new();
    let tmp = remote.path();
    let bare = remote.bare.to_str().unwrap();
    let (code, out, err) = readme(tmp, bare, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote v1"));
    assert!(entry_path(tmp, bare).is_dir());
}

#[test]
fn simultaneous_cold_invocations_share_one_clone() {
    let remote = Remote::new();
    let tmp = remote.path();
    remote.bulk(200, 4096);
    let args = ["--repo", remote.url.as_str(), "files", "README.md"];
    let children: Vec<Child> = (0..2).map(|_| spawn_focal(tmp, &args, true)).collect();
    let results: Vec<_> = children
        .into_iter()
        .map(|child| wait_piped(child, Duration::from_secs(60)))
        .collect();
    for (code, out, err) in &results {
        assert_eq!(*code, Some(0), "{err}");
        assert!(out.contains("remote v1"));
    }
    let errs: String = results.iter().map(|(_, _, err)| err.as_str()).collect();
    assert_eq!(
        errs.matches("Cloning remote repository into cache").count(),
        1,
        "{errs}"
    );
    let entries: Vec<PathBuf> = fs::read_dir(cache_root(tmp))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .collect();
    assert_eq!(entries, [entry_path(tmp, &remote.url)]);
    assert!(scratch(tmp).is_empty(), "{:?}", scratch(tmp));
}

#[test]
fn leftover_scratch_clone_is_discarded() {
    let remote = Remote::new();
    let tmp = remote.path();
    let name = entry_path(tmp, &remote.url)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let leftover = cache_root(tmp).join(format!(".tmp-99999-{name}"));
    fs::create_dir_all(leftover.join(".git")).unwrap();
    write(&leftover, "half-written", "x");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("info: Cloning remote repository into cache...\n"),
        "{err}"
    );
    assert!(out.contains("remote v1"));
    assert!(scratch(tmp).is_empty(), "{:?}", scratch(tmp));
}

#[test]
fn killed_clone_is_retried_cleanly() {
    let remote = Remote::new();
    let tmp = remote.path();
    remote.bulk(400, 16384);
    let args = ["--repo", remote.url.as_str(), "files", "README.md"];
    let mut child = spawn_focal(tmp, &args, false);
    let root = cache_root(tmp);
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        let cloning = fs::read_dir(&root).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
        });
        if cloning {
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    let status = child.wait().unwrap();
    let entry = entry_path(tmp, &remote.url);
    // Either the clone finished first or the kill left no half-made entry.
    assert!(status.code() == Some(0) || !entry.exists(), "{status:?}");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote v1"));
    assert!(scratch(tmp).is_empty(), "{:?}", scratch(tmp));
}

#[test]
fn damaged_entry_is_cloned_again() {
    let remote = Remote::new();
    let tmp = remote.path();
    // What a killed `git clone` into the entry path leaves behind.
    fs::create_dir_all(entry_path(tmp, &remote.url).join(".git")).unwrap();
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote v1"));
    assert!(scratch(tmp).is_empty(), "{:?}", scratch(tmp));
}

#[test]
fn default_branch_rename_is_followed() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    git(&remote.work, &["branch", "-q", "-m", "main", "trunk"]);
    remote.commit_on("README.md", "remote on trunk\n", "trunk");
    git(&remote.bare, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
    git(&remote.bare, &["branch", "-q", "-D", "main"]);
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote on trunk"));
}

#[test]
fn default_branch_rename_switches_the_local_branch() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    git(&remote.work, &["branch", "-q", "-m", "main", "trunk"]);
    remote.commit_on("README.md", "remote on trunk\n", "trunk");
    git(&remote.bare, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
    let (code, out, err) = readme(tmp, &remote.url, &[], &[("FOCAL_OMIT_WORKSPACE", "")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("Git Branch: trunk\n"), "{out}");
}

#[test]
fn offline_warm_cache_is_reused() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    fs::rename(&remote.bare, tmp.join("moved.git")).unwrap();
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("warning: Could not update the cached repository; using the cached copy.\n"),
        "{err}"
    );
    assert!(out.contains("remote v1"));
}

#[test]
fn unreachable_cold_remote_fails_without_an_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp = tmp.path();
    let url = format!("file://{}/missing.git", tmp.display());
    let (code, out, err) = readme(tmp, &url, &[], &[]);
    assert_ne!(code, Some(0));
    assert_eq!(out, "");
    assert!(!entry_path(tmp, &url).exists());
    assert!(
        err.contains("error: could not clone the remote repository"),
        "{err}"
    );
    assert!(scratch(tmp).is_empty(), "{:?}", scratch(tmp));
}

/// A checkout made by the legacy implementation: a plain `git clone` into the entry path.
fn legacy_clone(remote: &Remote, flags: &[&str]) -> PathBuf {
    let tmp = remote.path();
    let entry = entry_path(tmp, &remote.url);
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    let mut args = vec!["clone", "-q"];
    args.extend_from_slice(flags);
    args.extend([remote.url.as_str(), entry.to_str().unwrap()]);
    git(tmp, &args);
    write(&entry, ".git/legacy-sentinel", "kept");
    entry
}

#[test]
fn legacy_full_entry_is_updated_in_place() {
    let remote = Remote::new();
    let tmp = remote.path();
    let entry = legacy_clone(&remote, &[]);
    remote.commit("README.md", "remote v2\n");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("info: Updating cached remote repository...\n"),
        "{err}"
    );
    assert!(out.contains("remote v2"));
    assert!(entry.join(".git/legacy-sentinel").is_file());
    assert!(entry.join(".git/.focal_accessed").is_file());
}

#[test]
fn legacy_partial_entry_is_updated() {
    let remote = Remote::new();
    let tmp = remote.path();
    let entry = legacy_clone(&remote, &["--filter=blob:none"]);
    remote.commit("README.md", "remote v2\n");
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("info: Updating cached remote repository...\n"),
        "{err}"
    );
    assert!(out.contains("remote v2"));
    // Replaced by a full clone, since libgit2 cannot fetch missing blobs.
    let config = fs::read_to_string(entry.join(".git/config")).unwrap();
    assert!(!config.contains("partialclone"), "{config}");
    assert!(!config.contains("promisor"), "{config}");
}

#[test]
fn legacy_partial_entry_is_reused_offline() {
    let remote = Remote::new();
    let tmp = remote.path();
    legacy_clone(&remote, &["--filter=blob:none"]);
    fs::rename(&remote.bare, tmp.join("moved.git")).unwrap();
    let (code, out, err) = readme(tmp, &remote.url, &[], &[("FOCAL_OMIT_WORKSPACE", "")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("warning: Could not update the cached repository; using the cached copy.\n"),
        "{err}"
    );
    assert!(out.contains("remote v1"));
    assert!(out.contains("Git Branch: main\n"), "{out}");
}

fn make_entry(root: &Path, name: &str, days: u64, marker: bool) -> PathBuf {
    let entry = root.join(name);
    fs::create_dir_all(entry.join(".git")).unwrap();
    if marker {
        write(&entry, ".git/.focal_created", "");
        write(&entry, ".git/.focal_accessed", "");
        age(&entry.join(".git/.focal_accessed"), days);
    }
    age(&entry, days);
    entry
}

#[test]
fn cleanup_prunes_stale_entries_but_not_the_active_one() {
    let remote = Remote::new();
    let tmp = remote.path();
    let root = cache_root(tmp);
    readme_ok(tmp, &remote.url);
    let active = entry_path(tmp, &remote.url);
    age(&active.join(".git/.focal_accessed"), 40);
    let stale = make_entry(&root, "old_0123456789abcdef", 40, true);
    let unmarked = make_entry(&root, "older_0123456789abcdef", 40, false);
    let fresh = make_entry(&root, "fresh_0123456789abcdef", 3, true);
    let fresh_unmarked = make_entry(&root, "cloning_0123456789abcdef", 0, false);
    fs::remove_file(root.join(".last_cleanup")).unwrap();
    let (code, out, err) = readme(tmp, &remote.url, &[], &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("remote v1"));
    assert!(!stale.exists());
    assert!(!unmarked.exists());
    assert!(active.is_dir());
    assert!(fresh.is_dir());
    assert!(fresh_unmarked.is_dir());
    assert!(
        err.contains("info: Pruning stale cached repository: old_0123456789abcdef\n"),
        "{err}"
    );
    assert!(
        err.contains("info: Pruning legacy cached repository: older_0123456789abcdef\n"),
        "{err}"
    );
}

#[test]
fn cleanup_honors_the_ttl_and_runs_once_a_day() {
    let remote = Remote::new();
    let tmp = remote.path();
    let root = cache_root(tmp);
    readme_ok(tmp, &remote.url);
    let entry = make_entry(&root, "old_0123456789abcdef", 10, true);
    let ttl = [("FOCAL_REPO_TTL_DAYS", "5")];
    let (code, _, err) = readme(tmp, &remote.url, &[], &ttl);
    assert_eq!(code, Some(0), "{err}");
    assert!(entry.is_dir(), "throttled by .last_cleanup");
    fs::remove_file(root.join(".last_cleanup")).unwrap();
    let (code, _, err) = readme(tmp, &remote.url, &[], &ttl);
    assert_eq!(code, Some(0), "{err}");
    assert!(!entry.exists());
}

#[test]
fn cleanup_spares_directories_focal_does_not_own() {
    let remote = Remote::new();
    let tmp = remote.path();
    let root = cache_root(tmp);
    readme_ok(tmp, &remote.url);
    let notes = root.join("notes");
    write(&notes, "keep.txt", "mine");
    age(&notes, 40);
    fs::remove_file(root.join(".last_cleanup")).unwrap();
    readme_ok(tmp, &remote.url);
    assert!(notes.join("keep.txt").is_file());
}

#[test]
fn cleanup_spares_an_entry_in_use() {
    let remote = Remote::new();
    let tmp = remote.path();
    let root = cache_root(tmp);
    readme_ok(tmp, &remote.url);
    let busy = make_entry(&root, "busy_0123456789abcdef", 40, true);
    fs::create_dir_all(root.join(".locks")).unwrap();
    fs::remove_file(root.join(".last_cleanup")).unwrap();
    {
        let (_lock, taken) = flock(
            &root.join(".locks/busy_0123456789abcdef.use"),
            libc::LOCK_SH,
        );
        assert!(taken);
        readme_ok(tmp, &remote.url);
    }
    assert!(busy.is_dir());
}

#[test]
fn entry_stays_locked_while_the_command_runs() {
    let remote = Remote::new();
    let tmp = remote.path();
    readme_ok(tmp, &remote.url);
    let name = entry_path(tmp, &remote.url)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let lock_path = cache_root(tmp).join(".locks").join(format!("{name}.use"));
    let cwd = elsewhere(tmp);
    let cache = tmp.join("cache");
    let session = Session::start(
        &cwd,
        &["--repo", remote.url.as_str(), "files"],
        &[
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ],
    );
    session.wait_for("Files: ");
    {
        let (_lock, taken) = flock(&lock_path, libc::LOCK_EX | libc::LOCK_NB);
        assert!(!taken, "the entry lock was free while the picker ran");
    }
    session.send(&[b"\x1b"]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    let (_lock, taken) = flock(&lock_path, libc::LOCK_EX | libc::LOCK_NB);
    assert!(taken, "the entry lock was still held after exit");
}
