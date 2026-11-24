//! Shell completion scripts generated from the command definitions.
//!
//! `focal completions bash|zsh` renders a script from `crate::cli::Cli`, so subcommands
//! and their flags stay in step with the parser. The legacy scripts in `completions/`
//! complete command names through `focal --commands`, which the dispatcher still
//! answers, so existing installs keep working.

use clap::{Arg, Command, CommandFactory};
use clap_complete::Shell;
use std::io::{self, Write};

#[derive(clap::Args)]
pub struct Args {
    /// bash or zsh.
    #[arg(value_name = "SHELL")]
    pub shell: String,
}

/// The parser definition without hidden subcommands (clap_complete lists them), plus the
/// dispatcher's `--repo` flag, which `cli::split_repo` handles before clap sees the
/// arguments.
fn command() -> Command {
    let cli = crate::cli::Cli::command();
    Command::new("focal")
        .disable_help_subcommand(true)
        .args(cli.get_arguments().cloned())
        .subcommands(
            cli.get_subcommands()
                .filter(|sub| !sub.is_hide_set())
                .cloned(),
        )
        .arg(
            Arg::new("repo")
                .long("repo")
                .global(true)
                .value_name("URL")
                .value_hint(clap::ValueHint::Url)
                .help("Run the command on a remote GitHub repository (caches locally)"),
        )
}

/// Renders the completion script for `shell` (`bash` or `zsh`).
pub fn script(shell: &str) -> io::Result<Vec<u8>> {
    let shell = match shell {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        other => {
            return Err(crate::output::fail(
                format!("unsupported shell '{other}'"),
                "use 'focal completions bash' or 'focal completions zsh'",
            ));
        }
    };
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut command(), "focal", &mut out);
    Ok(out)
}

pub fn run(args: Args) -> io::Result<()> {
    let text = script(&args.shell)?;
    io::stdout().lock().write_all(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::COMMANDS;

    #[test]
    fn scripts_name_every_public_command_and_no_hidden_one() {
        for shell in ["bash", "zsh"] {
            let text = String::from_utf8(script(shell).unwrap()).unwrap();
            for (name, _) in COMMANDS {
                assert!(text.contains(name), "{shell}: {name}");
            }
            assert!(text.contains("--staged"), "{shell}");
            assert!(text.contains("--repo"), "{shell}");
            assert!(!text.contains("completions"), "{shell}");
        }
    }

    #[test]
    fn unknown_shell_is_an_error() {
        let error = script("fish").err().unwrap();
        assert!(error.to_string().starts_with("unsupported shell 'fish'"));
    }
}
