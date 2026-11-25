//! Interactive picker contracts, driven through a pseudo-terminal.
//!
//! Each run gives Focal a controlling terminal while stdout goes to a file, the way
//! `focal files > result.md` works.

mod common;

use common::*;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const ESC: &[u8] = b"\x1b";
const TAB: &[u8] = b"\t";
const ENTER: &[u8] = b"\r";
const CTRL_A: &[u8] = b"\x01";
const CTRL_C: &[u8] = b"\x03";
const CTRL_U: &[u8] = b"\x15";

fn repo(tmp: &Path) -> PathBuf {
    let root = tmp.join("repo");
    write(&root, "src/alpha.py", "alpha = 1\n");
    write(&root, "src/beta.py", "beta = 2\n");
    write(&root, "notes.md", "notes\n");
    write(&root, "README.md", "readme\n");
    root
}

fn headings(markdown: &str) -> Vec<&str> {
    markdown.lines().filter(|l| l.starts_with("# ")).collect()
}

/// Waits for the process to exit (without reaping it) and for the terminal output to
/// settle, then returns everything drawn.
fn screen_after_exit(session: &Session) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                session.pid() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc == 0 && unsafe { info.si_pid() } != 0 {
            break;
        }
        assert!(Instant::now() < deadline, "focal did not exit within 20 s");
        thread::sleep(Duration::from_millis(20));
    }
    let mut last = session.screen();
    loop {
        thread::sleep(Duration::from_millis(100));
        let now = session.screen();
        if now.len() == last.len() {
            return now;
        }
        last = now;
    }
}

fn rindex(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn assert_restored(screen: &[u8], done: &Finished) {
    assert_eq!(
        done.after.c_lflag, done.before.c_lflag,
        "terminal local modes were not restored"
    );
    assert_ne!(done.after.c_lflag & libc::ICANON, 0);
    assert_ne!(done.after.c_lflag & libc::ECHO, 0);
    let enter = rindex(screen, b"\x1b[?1049h").expect("never entered the alternate screen");
    let leave = rindex(screen, b"\x1b[?1049l").expect("never left the alternate screen");
    assert!(leave > enter);
}

/// Runs focal in a new session with no controlling terminal and stdin from /dev/null.
fn focal_without_terminal(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().expect("run focal")
}

#[test]
fn files_multi_select_survives_filtering() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["files"], &[]);
    session.wait_for("Files: ");
    session.wait_for("1 │ ");
    session.send(&[b"beta", TAB, CTRL_U, b"alpha", TAB, ENTER]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(headings(&done.stdout), ["# src/alpha.py", "# src/beta.py"]);
    assert!(
        done.stderr
            .contains("added: src/alpha.py\nadded: src/beta.py\n"),
        "{}",
        done.stderr
    );
    assert_restored(&screen, &done);
}

#[test]
fn files_enter_takes_cursor_item_and_previews() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["files"], &[]);
    session.wait_for("TAB: multi-select | CTRL-A: select all | ENTER: confirm");
    session.send(&[b"notes"]);
    session.wait_for("1 │ notes");
    session.send(&[ENTER]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(headings(&done.stdout), ["# notes.md"]);
    assert_restored(&screen, &done);
}

#[test]
fn files_select_all() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["files"], &[]);
    session.wait_for("Files: ");
    session.send(&[b".py", CTRL_A, ENTER]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(headings(&done.stdout), ["# src/alpha.py", "# src/beta.py"]);
    assert_restored(&screen, &done);
}

fn files_cancel(key: &[u8]) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["files"], &[]);
    session.wait_for("Files: ");
    session.send(&[TAB, key]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(done.stdout, "");
    assert_eq!(done.stderr, "info: No files selected.\n");
    assert_restored(&screen, &done);
}

#[test]
fn files_cancel_esc() {
    files_cancel(ESC);
}

#[test]
fn files_cancel_ctrl_c() {
    files_cancel(CTRL_C);
}

#[test]
fn context_cancel_still_emits_core_context() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["context", "notes.md"], &[]);
    session.wait_for("Auto-included: README.md");
    session.wait_for("Pre-selected: notes.md");
    session.send(&[ESC]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(
        headings(&done.stdout),
        ["# Repository Tree", "# README.md", "# notes.md"]
    );
    assert_restored(&screen, &done);
}

#[test]
fn context_adds_picked_files_after_core_and_preselected() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let session = Session::start(&repo, &["context", "notes.md"], &[]);
    session.wait_for("Additional files: ");
    // Core and pre-selected files are not offered again.
    session.send(&[b"beta", TAB, ENTER]);
    let screen = screen_after_exit(&session);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(
        headings(&done.stdout),
        [
            "# Repository Tree",
            "# README.md",
            "# notes.md",
            "# src/beta.py"
        ]
    );
    assert_restored(&screen, &done);
}

#[test]
fn no_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let home = tmp.path().to_str().unwrap();
    let result = focal_without_terminal(&repo, &env(&[("PATH", ""), ("HOME", home)]), &["files"]);
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("interactive selection needs a terminal"),
        "{stderr}"
    );
    assert!(
        stderr.contains("pass paths or a flag such as --all"),
        "{stderr}"
    );
}
