use ignore::WalkBuilder;
use std::env;
use std::ffi::OsStr;
use std::fmt::Write;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};

#[derive(Default)]
struct Node {
    label: String,
    children: Vec<Node>,
}

fn name(value: &OsStr, unicode: bool) -> String {
    if unicode && let Some(value) = value.to_str() {
        let mut out = String::new();
        for ch in value.chars() {
            if ch.is_ascii_control() {
                write!(out, "\\{:03o}", ch as u32).unwrap();
            } else {
                out.push(ch);
            }
        }
        return out;
    }
    let mut out = String::new();
    for &byte in value.as_bytes() {
        match byte {
            b' ' => out.push_str("\\ "),
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            12 => out.push_str("\\f"),
            33..=126 => out.push(byte as char),
            _ => write!(out, "\\{byte:03o}").unwrap(),
        }
    }
    out
}

fn suffix(meta: &fs::Metadata) -> &'static str {
    let kind = meta.file_type();
    if kind.is_dir() {
        "/"
    } else if kind.is_fifo() {
        "|"
    } else if kind.is_socket() {
        "="
    } else if kind.is_file() && meta.permissions().mode() & 0o111 != 0 {
        "*"
    } else {
        ""
    }
}

fn draw(node: &Node, prefix: &str, unicode: bool, output: &mut String) {
    let (branch, end, bar) = if unicode {
        ("├── ", "└── ", "│\u{a0}\u{a0} ")
    } else {
        ("|-- ", "`-- ", "|   ")
    };
    for (i, child) in node.children.iter().enumerate() {
        let last = i + 1 == node.children.len();
        write!(
            output,
            "\n{prefix}{}{}",
            if last { end } else { branch },
            child.label
        )
        .unwrap();
        if !child.children.is_empty() {
            draw(
                child,
                &format!("{prefix}{}", if last { "    " } else { bar }),
                unicode,
                output,
            );
        }
    }
}

pub fn render() -> io::Result<String> {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(|key| env::var(key).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let unicode = locale
        .to_ascii_lowercase()
        .replace('-', "")
        .contains("utf8");
    let walker = WalkBuilder::new(".")
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .require_git(false)
        .max_depth(Some(6))
        .sort_by_file_name(OsStr::cmp)
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".git" | "node_modules" | ".venv" | "__pycache__" | "dist" | "build")
            )
        })
        .build();
    let mut root = Node::default();
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if entry.depth() == 0 {
            continue;
        }
        let mut label = name(entry.file_name(), unicode);
        let path = entry.path();
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            label.push_str(" -> ");
            label.push_str(&name(fs::read_link(path)?.as_os_str(), unicode));
            if let Ok(meta) = fs::metadata(path) {
                label.push_str(suffix(&meta));
            }
        } else {
            label.push_str(suffix(&entry.metadata().map_err(io::Error::other)?));
        }
        let mut parent = &mut root;
        for _ in 1..entry.depth() {
            parent = parent
                .children
                .last_mut()
                .expect("depth-first walk visits parent first");
        }
        parent.children.push(Node {
            label,
            children: Vec::new(),
        });
    }
    let mut text = String::from("./");
    draw(&root, "", unicode, &mut text);
    Ok(text)
}
