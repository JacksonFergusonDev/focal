//! Clipboard delivery contracts.
//!
//! Redirected output always goes to stdout. With a terminal on stdout, the payload goes to
//! the clipboard, or to stdout with a warning when no clipboard is available.

mod common;

use common::*;
use std::fs;
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn base_env(tmp: &Path, fake: Option<&Path>) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    let path = fake
        .map(|dir| dir.to_str().unwrap().to_owned())
        .unwrap_or_default();
    env(&[
        ("PATH", &path),
        ("HOME", home.to_str().unwrap()),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("FOCAL_OMIT_WORKSPACE", "1"),
    ])
}

/// Runs focal with a pseudo-terminal on stdout; returns (terminal output with CRLF folded
/// to LF, stderr, exit code).
fn run_on_terminal(cwd: &Path, env: &Env, args: &[&str]) -> (Vec<u8>, String, Option<i32>) {
    let (mut master, mut slave) = (0, 0);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty");
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    // Read the terminal while the child runs, so a full pty buffer never blocks it.
    let shown = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let shown = Arc::clone(&shown);
        let mut master = fs::File::from(master.try_clone().unwrap());
        thread::spawn(move || {
            let mut buf = [0u8; 65536];
            loop {
                match master.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => shown.lock().unwrap().extend_from_slice(&buf[..n]),
                }
            }
        })
    };
    let output = Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::from(slave))
        .stderr(Stdio::piped())
        .output()
        .expect("run focal");
    // Every slave descriptor is now closed, so the reader sees EOF or EIO once drained.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !reader.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    drop(master);
    let shown = shown.lock().unwrap().clone();
    let shown = String::from_utf8_lossy(&shown)
        .replace("\r\n", "\n")
        .into_bytes();
    (shown, text(&output.stderr), output.status.code())
}

/// A fake `xclip` that writes what it receives to `tmp/clipboard.txt`.
fn fake_xclip(tmp: &Path) -> (PathBuf, PathBuf) {
    let bin_dir = tmp.join("fake-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let sink = tmp.join("clipboard.txt");
    let helper = write(
        &bin_dir,
        "xclip",
        format!("#!/bin/sh\n/bin/cat > \"{}\"\n", sink.display()),
    );
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    (bin_dir, sink)
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "macOS copies in process, not via a helper"
)]
fn terminal_copies_through_helper() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin_dir, sink) = fake_xclip(tmp.path());
    write(tmp.path(), "note.txt", "marker\n");
    let env = with(&base_env(tmp.path(), Some(&bin_dir)), &[("DISPLAY", ":99")]);
    let (shown, err, code) = run_on_terminal(tmp.path(), &env, &["files", "note.txt"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(text(&shown), "");
    let copied = text(&fs::read(&sink).unwrap());
    assert!(copied.starts_with("<context>\n# note.txt\n"), "{copied}");
    assert!(
        copied.ends_with("marker\n```\n\n\n</context>\n"),
        "{copied}"
    );
    assert!(
        err.contains("done: Copied note.txt to clipboard (~"),
        "{err}"
    );
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "macOS copies in process, not via a helper"
)]
fn redirected_output_skips_clipboard() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin_dir, sink) = fake_xclip(tmp.path());
    write(tmp.path(), "note.txt", "marker\n");
    let env = with(&base_env(tmp.path(), Some(&bin_dir)), &[("DISPLAY", ":99")]);
    let result = focal(tmp.path(), &env, &["files", "note.txt"], None);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("marker"));
    assert!(!sink.exists());
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "macOS copies in process, not via a helper"
)]
fn terminal_without_clipboard_falls_back_to_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "note.txt", "marker\n");
    let (shown, err, code) = run_on_terminal(
        tmp.path(),
        &base_env(tmp.path(), None),
        &["files", "note.txt"],
    );
    assert_eq!(code, Some(0), "{err}");
    let shown = text(&shown);
    assert!(shown.starts_with("<context>\n# note.txt\n"), "{shown}");
    assert!(shown.contains("marker"));
    assert!(err.contains("Dumping to stdout..."), "{err}");
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "uses the macOS clipboard")]
fn macos_clipboard_persists_after_exit() {
    if std::env::var_os("FOCAL_TEST_CLIPBOARD").is_none_or(|v| v.is_empty()) {
        eprintln!("skipped: replaces the real macOS clipboard; set FOCAL_TEST_CLIPBOARD=1 to run");
        return;
    }
    let saved = Command::new("pbpaste").output().unwrap();
    assert!(saved.status.success());
    let tmp = tempfile::tempdir().unwrap();
    let marker = format!("marker {}", std::process::id());
    let outcome = std::panic::catch_unwind(|| {
        write(tmp.path(), "note.txt", format!("{marker}\n"));
        let (shown, err, code) = run_on_terminal(
            tmp.path(),
            &base_env(tmp.path(), None),
            &["files", "note.txt"],
        );
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(text(&shown), "");
        let pasted = Command::new("pbpaste").output().unwrap();
        assert!(pasted.status.success());
        let pasted = text(&pasted.stdout);
        assert!(pasted.contains(&marker), "{pasted}");
        assert!(pasted.ends_with("</context>\n"), "{pasted}");
    });
    let restored = run_with(
        Path::new("/usr/bin/pbcopy"),
        tmp.path(),
        &env(&[]),
        &[] as &[&str],
        Some(&saved.stdout),
    );
    assert!(restored.status.success());
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
