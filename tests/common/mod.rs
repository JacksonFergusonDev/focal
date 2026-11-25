//! Shared harness for the CLI contract tests.
//!
//! - [`focal`] runs the binary under test with a cleared environment.
//! - [`FakeGitHub`] serves canned GitHub API replies over HTTP and records requests.
//! - [`github_checkout`] creates a repository whose `origin` is `github.com/octo/widget`.
//! - [`Session`] runs the binary on a pseudo-terminal for picker tests.
//!
//! Every test binary includes this module, and none uses all of it.
#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const OWNER: &str = "octo";
pub const REPO: &str = "widget";

/// The binary under test.
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_focal"))
}

/// Environment variables, applied to a cleared environment.
pub type Env = Vec<(String, String)>;

/// Clears a command's environment like `env_clear`, but keeps the variables that coverage
/// instrumentation needs (`LLVM_PROFILE_FILE` and friends from `cargo llvm-cov`), so the
/// binary's own lines count when tests run it.
pub trait CleanEnv {
    fn clean_env(&mut self) -> &mut Self;
}

impl CleanEnv for Command {
    fn clean_env(&mut self) -> &mut Self {
        self.env_clear();
        for (key, value) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("LLVM_PROFILE") || name.starts_with("CARGO_LLVM_COV") {
                self.env(key, value);
            }
        }
        self
    }
}

/// Builds an [`Env`] from string pairs.
pub fn env(pairs: &[(&str, &str)]) -> Env {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Returns `base` with `extra` added, replacing variables of the same name.
pub fn with(base: &Env, extra: &[(&str, &str)]) -> Env {
    let mut out: Env = base
        .iter()
        .filter(|(k, _)| !extra.iter().any(|(e, _)| e == k))
        .cloned()
        .collect();
    out.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    out
}

/// Runs `focal <args>` in `cwd` with exactly `env` (PATH included only if `env` sets it),
/// stdin from `stdin` or /dev/null, and captured stdout and stderr.
pub fn focal<S: AsRef<OsStr>>(cwd: &Path, env: &Env, args: &[S], stdin: Option<&[u8]>) -> Output {
    run_with(&bin(), cwd, env, args, stdin)
}

/// Like [`focal`] for any program.
pub fn run_with<S: AsRef<OsStr>>(
    program: &Path,
    cwd: &Path,
    env: &Env,
    args: &[S],
    stdin: Option<&[u8]>,
) -> Output {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .clean_env()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().expect("spawn focal");
    if let Some(input) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        let input = input.to_vec();
        thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    child.wait_with_output().expect("wait for focal")
}

/// Decodes bytes as UTF-8, replacing invalid sequences.
pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs `git` with a fixed identity and no system config; panics on failure and returns
/// trimmed stdout.
pub fn git(cwd: &Path, args: &[&str]) -> String {
    let output = git_output(cwd, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        text(&output.stderr)
    );
    text(&output.stdout).trim().to_owned()
}

/// Runs `git` like [`git`] but returns the output whatever the status.
pub fn git_output(cwd: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(["-c", "user.name=a", "-c", "user.email=a@example.com"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "a")
        .env("GIT_AUTHOR_EMAIL", "a@example.com")
        .env("GIT_COMMITTER_NAME", "a")
        .env("GIT_COMMITTER_EMAIL", "a@example.com")
        .output()
        .expect("run git")
}

/// Writes `contents` to `root/relative`, creating parent directories.
pub fn write(root: &Path, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

/// A repository on branch `main` with one commit (`README.md`) and origin set to
/// `https://github.com/octo/widget.git`, at `tmp/checkout`.
pub fn github_checkout(tmp: &Path) -> PathBuf {
    let root = tmp.join("checkout");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, "README.md", "widget\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "initial"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("https://github.com/{OWNER}/{REPO}.git"),
        ],
    );
    root
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// One request the fake received. Header names are lower-cased.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Default)]
struct State {
    routes: HashMap<(String, String), Vec<Reply>>,
    requests: Vec<Request>,
}

/// A local fake of the GitHub REST and GraphQL API.
///
/// Replies are keyed by method and path; a full path with query wins over the bare path.
/// Several replies for one key are served in order, and the last one repeats. Unknown
/// routes get a 404 JSON error.
pub struct FakeGitHub {
    pub url: String,
    state: Arc<Mutex<State>>,
    server: Arc<tiny_http::Server>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FakeGitHub {
    pub fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind fake"));
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let worker = {
            let server = Arc::clone(&server);
            let state = Arc::clone(&state);
            thread::spawn(move || {
                for mut request in server.incoming_requests() {
                    let state = Arc::clone(&state);
                    // One thread per request, so a client with requests in flight is served
                    // concurrently, as the Python ThreadingHTTPServer did.
                    thread::spawn(move || {
                        let mut body = Vec::new();
                        let _ = request.as_reader().read_to_end(&mut body);
                        let method = request.method().as_str().to_owned();
                        let path = request.url().to_owned();
                        let headers = request
                            .headers()
                            .iter()
                            .map(|h| {
                                (
                                    h.field.as_str().as_str().to_ascii_lowercase(),
                                    h.value.as_str().to_owned(),
                                )
                            })
                            .collect();
                        let reply = {
                            let mut state = state.lock().unwrap();
                            state.requests.push(Request {
                                method: method.clone(),
                                path: path.clone(),
                                headers,
                                body,
                            });
                            reply_for(&mut state, &method, &path)
                        };
                        let mut response = tiny_http::Response::from_data(reply.body)
                            .with_status_code(reply.status);
                        for (name, value) in reply.headers {
                            response.add_header(
                                tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
                                    .unwrap(),
                            );
                        }
                        let _ = request.respond(response);
                    });
                }
            })
        };
        Self {
            url,
            state,
            server,
            worker: Some(worker),
        }
    }

    /// Registers a JSON reply (`Content-Type: application/json`) with status 200.
    pub fn add(&self, path: &str, body: serde_json::Value) {
        self.add_reply("GET", path, 200, json_body(&body), json_headers(&[]));
    }

    /// Registers a JSON reply with a status and extra headers.
    pub fn add_with(
        &self,
        method: &str,
        path: &str,
        status: u16,
        body: serde_json::Value,
        headers: &[(&str, &str)],
    ) {
        self.add_reply(
            method,
            path,
            status,
            json_body(&body),
            json_headers(headers),
        );
    }

    /// Registers a raw-bytes reply with status 200 and no content type.
    pub fn add_raw(&self, path: &str, raw: impl Into<Vec<u8>>) {
        self.add_reply("GET", path, 200, raw.into(), Vec::new());
    }

    /// Registers a reply with no body (for example a redirect).
    pub fn add_empty(&self, method: &str, path: &str, status: u16, headers: &[(&str, &str)]) {
        let headers = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        self.add_reply(method, path, status, Vec::new(), headers);
    }

    /// Registers any reply.
    pub fn add_reply(
        &self,
        method: &str,
        path: &str,
        status: u16,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    ) {
        self.state
            .lock()
            .unwrap()
            .routes
            .entry((method.to_owned(), path.to_owned()))
            .or_default()
            .push(Reply {
                status,
                body,
                headers,
            });
    }

    /// An absolute URL on the fake, for Link headers and redirects.
    pub fn link(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    /// Requests received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }

    /// The standard environment for GitHub commands: empty PATH, `tmp/home` as HOME, the
    /// fake as the API base, and a test token, plus `extra`.
    pub fn env(&self, tmp: &Path, extra: &[(&str, &str)]) -> Env {
        let home = tmp.join("home");
        fs::create_dir_all(&home).unwrap();
        let base = env(&[
            ("PATH", ""),
            ("HOME", home.to_str().unwrap()),
            ("LC_ALL", "C.UTF-8"),
            ("NO_COLOR", "1"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("FOCAL_GITHUB_API_URL", &self.url),
            ("GH_TOKEN", "test-token"),
        ]);
        with(&base, extra)
    }
}

fn json_body(body: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(body).unwrap()
}

fn json_headers(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
    headers.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    headers
}

fn reply_for(state: &mut State, method: &str, target: &str) -> Reply {
    let bare = target.split(['?', '#']).next().unwrap_or(target).to_owned();
    for key in [
        (method.to_owned(), target.to_owned()),
        (method.to_owned(), bare),
    ] {
        if let Some(replies) = state.routes.get_mut(&key)
            && !replies.is_empty()
        {
            return if replies.len() > 1 {
                replies.remove(0)
            } else {
                replies[0].clone()
            };
        }
    }
    Reply {
        status: 404,
        body: serde_json::to_vec(
            &serde_json::json!({"message": format!("no fixture for {method} {target}")}),
        )
        .unwrap(),
        headers: Vec::new(),
    }
}

impl Drop for FakeGitHub {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Terminal attributes, for checking that a picker restored them.
pub type Termios = libc::termios;

pub fn same_termios(a: &Termios, b: &Termios) -> bool {
    a.c_iflag == b.c_iflag
        && a.c_oflag == b.c_oflag
        && a.c_cflag == b.c_cflag
        && a.c_lflag == b.c_lflag
        && a.c_cc == b.c_cc
}

/// What [`Session::finish`] returns.
pub struct Finished {
    pub code: Option<i32>,
    pub stderr: String,
    /// The payload written to stdout (`<cwd>/../result.md`).
    pub stdout: String,
    /// Terminal attributes before the run and after it exited.
    pub before: Termios,
    pub after: Termios,
}

/// Runs `focal <args>` on a 120x30 pseudo-terminal that is the process's controlling
/// terminal; stdout goes to `cwd/../result.md` and stderr is captured.
///
/// Environment: empty PATH, HOME=`cwd/..`, TERM=xterm-256color, LC_ALL=C.UTF-8,
/// NO_COLOR=1, FOCAL_OMIT_WORKSPACE=1, plus `extra` (which wins).
pub struct Session {
    master: OwnedFd,
    slave: OwnedFd,
    name: PathBuf,
    before: Termios,
    output: PathBuf,
    child: Child,
    screen: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    pub fn start<S: AsRef<OsStr>>(cwd: &Path, args: &[S], extra: &[(&str, &str)]) -> Self {
        let (master, slave) = openpty();
        let name = unsafe {
            let raw = libc::ttyname(slave.as_raw_fd());
            assert!(!raw.is_null(), "ttyname");
            PathBuf::from(std::ffi::CStr::from_ptr(raw).to_str().unwrap())
        };
        let before = tcgetattr(slave.as_raw_fd());
        let output = cwd.parent().unwrap().join("result.md");
        let stdout = fs::File::create(&output).unwrap();
        let home = cwd.parent().unwrap().to_str().unwrap().to_owned();
        let base = env(&[
            ("PATH", ""),
            ("HOME", &home),
            ("TERM", "xterm-256color"),
            ("LC_ALL", "C.UTF-8"),
            ("NO_COLOR", "1"),
            ("FOCAL_OMIT_WORKSPACE", "1"),
        ]);
        let environment = with(&base, extra);
        let stdin = slave.try_clone().unwrap();
        let tty = slave.as_raw_fd();
        let mut command = Command::new(bin());
        command
            .args(args)
            .current_dir(cwd)
            .clean_env()
            .envs(environment.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(tty, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("spawn focal on a pty");
        let screen = Arc::new(Mutex::new(Vec::new()));
        {
            let screen = Arc::clone(&screen);
            let mut reader = fs::File::from(master.try_clone().unwrap());
            thread::spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => screen.lock().unwrap().extend_from_slice(&buf[..n]),
                    }
                }
            });
        }
        Self {
            master,
            slave,
            name,
            before,
            output,
            child,
            screen,
        }
    }

    /// Everything drawn on the terminal so far.
    pub fn screen(&self) -> Vec<u8> {
        self.screen.lock().unwrap().clone()
    }

    /// Waits up to 10 seconds for `text` to be drawn.
    pub fn wait_for(&self, text: &str) {
        self.wait_for_within(text, Duration::from_secs(10));
    }

    pub fn wait_for_within(&self, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let needle = text.as_bytes();
        loop {
            if self
                .screen
                .lock()
                .unwrap()
                .windows(needle.len().max(1))
                .any(|w| w == needle)
            {
                return;
            }
            assert!(Instant::now() < deadline, "never drew {text:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Sends each key, pausing 0.3 s after each.
    pub fn send(&self, keys: &[&[u8]]) {
        self.send_with_pause(keys, Duration::from_millis(300));
    }

    pub fn send_with_pause(&self, keys: &[&[u8]], pause: Duration) {
        let mut tty = fs::File::from(self.master.try_clone().unwrap());
        for key in keys {
            tty.write_all(key).unwrap();
            thread::sleep(pause);
        }
    }

    /// The child's process id, for sending signals.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Waits up to 20 seconds for the process to exit and collects its results.
    pub fn finish(mut self) -> Finished {
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("focal did not exit within 20 s");
            }
            thread::sleep(Duration::from_millis(20));
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let after = tcgetattr_checked(self.slave.as_raw_fd()).unwrap_or_else(|| {
            // macOS revokes the terminal from other holders when the session leader exits.
            let path = std::ffi::CString::new(self.name.to_str().unwrap()).unwrap();
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
            assert!(fd >= 0, "reopen pty");
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            tcgetattr(fd.as_raw_fd())
        });
        use std::os::unix::process::ExitStatusExt;
        let code = status.code().or_else(|| status.signal().map(|s| 128 + s));
        Finished {
            code,
            stderr,
            stdout: fs::read_to_string(&self.output).unwrap_or_default(),
            before: self.before,
            after,
        }
    }
}

fn openpty() -> (OwnedFd, OwnedFd) {
    let mut master = 0;
    let mut slave = 0;
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // A raw pointer: the parameter is `*mut` on macOS and `*const` on Linux.
            &raw mut size,
        )
    };
    assert_eq!(rc, 0, "openpty");
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

fn tcgetattr_checked(fd: i32) -> Option<Termios> {
    let mut attrs: Termios = unsafe { std::mem::zeroed() };
    (unsafe { libc::tcgetattr(fd, &mut attrs) } == 0).then_some(attrs)
}

fn tcgetattr(fd: i32) -> Termios {
    tcgetattr_checked(fd).expect("tcgetattr")
}
