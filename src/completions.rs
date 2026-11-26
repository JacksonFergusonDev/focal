//! Shell completion scripts generated from the command definitions.
//!
//! `focal completions bash|zsh` renders a script from `crate::cli::Cli`, so subcommands
//! and their flags stay in step with the parser. Both scripts can be loaded straight from
//! a shell startup file (`source <(focal completions zsh)`), and the Zsh one also works
//! as an `_focal` file on `fpath`.

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
    if shell == Shell::Zsh {
        out = sourceable_zsh(out);
    }
    Ok(out)
}

/// The tail clap_complete ends its Zsh script with. Autoloaded from `fpath` it runs
/// `_focal`; sourced, it calls `compdef`, which only exists once `compinit` has run.
const ZSH_REGISTER: &str = r#"if [ "$funcstack[1]" = "_focal" ]; then
    _focal "$@"
else
    compdef _focal focal
fi
"#;

/// Lets `source <(focal completions zsh)` sit anywhere in `.zshrc`: when `compinit` has
/// not run yet, registration waits for the first prompt, by which point it has.
const ZSH_REGISTER_ANYWHERE: &str = r#"if [ "$funcstack[1]" = "_focal" ]; then
    _focal "$@"
elif (( $+functions[compdef] )); then
    compdef _focal focal
else
    # Loaded before compinit: register at the first prompt instead.
    _focal_register() {
        (( $+functions[compdef] )) && compdef _focal focal
        add-zsh-hook -d precmd _focal_register
        unfunction _focal_register
    }
    autoload -Uz add-zsh-hook
    add-zsh-hook precmd _focal_register
fi
"#;

fn sourceable_zsh(script: Vec<u8>) -> Vec<u8> {
    let text = String::from_utf8(script).expect("clap_complete writes UTF-8");
    let Some(start) = text.rfind(ZSH_REGISTER) else {
        // A clap_complete upgrade changed its tail; the tests catch this.
        return text.into_bytes();
    };
    let mut text = text;
    text.replace_range(start..start + ZSH_REGISTER.len(), ZSH_REGISTER_ANYWHERE);
    text.into_bytes()
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
    fn zsh_script_registers_when_sourced_before_compinit() {
        let text = String::from_utf8(script("zsh").unwrap()).unwrap();
        assert!(text.ends_with(ZSH_REGISTER_ANYWHERE), "{text}");
    }

    #[test]
    fn unknown_shell_is_an_error() {
        let error = script("fish").err().unwrap();
        assert!(error.to_string().starts_with("unsupported shell 'fish'"));
    }
}
