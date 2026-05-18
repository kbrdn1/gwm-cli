mod bootstrap;
mod cli;
mod config;
mod error;
mod naming;
mod tui;
mod worktree;

use clap::Parser;

fn main() {
  let args = cli::Cli::parse();
  if let Err(e) = cli::run(args) {
    eprintln!("error: {}", e);
    std::process::exit(1);
  }
}
