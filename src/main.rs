//! The `rbx-switch` binary.
//!
//! The logic lives in the library half so the account-resolution rules stay
//! unit-testable without a terminal; this file is the parser and the exit
//! code, nothing else.

use std::process::ExitCode;

use clap::Parser;
use colored::Colorize;

use rbx_switch::{GlobalFlags, SwitchCli};

#[derive(Parser, Debug)]
#[command(
    name = "rbx-switch",
    version,
    about = "Switch between signed-in Roblox Studio accounts",
    long_about = "Switch between the Roblox accounts signed into Studio, without opening \
                  Studio. Windows only today; the macOS credential store is a stub.\n\n\
                  With no account named, an interactive picker opens."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalFlags,

    #[command(flatten)]
    switch: SwitchCli,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // The error is printed here rather than returned from `main` so it reads
    // as a sentence to a user instead of a `Debug` dump of an `anyhow::Error`.
    match rbx_switch::run(cli.switch, &cli.global) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{} {error:#}", "error:".red().bold());
            ExitCode::FAILURE
        }
    }
}
