//! Shell completion scripts generated from the command definitions.
//!
//! Owned by migration track E. Reference: completions/focal.bash and completions/_focal.

use std::io;

#[derive(clap::Args)]
pub struct Args {
    /// bash or zsh.
    #[arg(value_name = "SHELL")]
    pub shell: String,
}

pub fn run(_args: Args) -> io::Result<()> {
    Err(crate::output::fail(
        "completion generation is not ported to the Rust preview yet",
        "use the scripts in completions/",
    ))
}
