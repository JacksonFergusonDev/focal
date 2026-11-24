//! Copies payloads to the desktop clipboard so they remain pasteable after `focal` exits.
//!
//! macOS keeps pasteboard contents after the writing process exits, so the copy happens in
//! process. On Linux, X11 and Wayland clipboards are served by the owning process, so the
//! copy is handed to an installed helper (wl-copy, xclip, or xsel) that stays resident.

/// Copies `text`, or explains why the clipboard is unavailable.
#[cfg(target_os = "macos")]
pub fn copy(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| format!("the macOS pasteboard rejected the copy ({error})"))
}

#[cfg(not(target_os = "macos"))]
pub fn copy(text: &str) -> Result<(), String> {
    use std::env;
    use std::io::Write;
    use std::process::{Command, Stdio};

    let set = |name: &str| env::var_os(name).is_some_and(|value| !value.is_empty());
    let mut helpers: Vec<(&str, &[&str])> = Vec::new();
    if set("WAYLAND_DISPLAY") {
        helpers.push(("wl-copy", &[]));
    }
    if set("DISPLAY") {
        helpers.push(("xclip", &["-selection", "clipboard"]));
        helpers.push(("xsel", &["--clipboard", "--input"]));
    }
    if helpers.is_empty() {
        return Err(
            "no graphical session was detected (WAYLAND_DISPLAY and DISPLAY are unset)".into(),
        );
    }
    let mut failures = Vec::new();
    for (program, args) in helpers {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                failures.push(format!("{program}: {error}"));
                continue;
            }
        };
        let written = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(text.as_bytes()));
        match (written, child.wait()) {
            (Some(Ok(())), Ok(status)) if status.success() => return Ok(()),
            (_, Ok(status)) if !status.success() => {
                failures.push(format!("{program} exited with {status}"))
            }
            (Some(Err(error)), _) | (_, Err(error)) => failures.push(format!("{program}: {error}")),
            _ => failures.push(format!("{program} could not receive input")),
        }
    }
    if failures.is_empty() {
        Err("no clipboard helper is installed (install wl-clipboard, xclip, or xsel)".into())
    } else {
        Err(failures.join("; "))
    }
}
