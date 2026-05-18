use clap::Parser;
use gwm::cli;

fn main() {
  let args = cli::Cli::parse();
  if let Err(e) = cli::run(args) {
    eprintln!("error: {}", e);
    std::process::exit(1);
  }
}
