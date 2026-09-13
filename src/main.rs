use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "tmux sessions for Git worktrees")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Switch {
        #[arg(long)]
        repo: bool,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let tmux = grove::tmux::Tmux::from_env();
    match cli.command {
        Some(Command::Switch { repo }) => grove::switch(&tmux, repo),
        None => grove::open(&grove::config::load()?, &tmux),
    }
}
