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
    Switch,
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let tmux = grove::tmux::Tmux::from_env();
    match cli.command {
        Some(Command::Switch) => grove::switch(&tmux),
        None => grove::open(&grove::config::load()?, &tmux),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_accepts_no_options_and_rejects_repo() {
        assert!(matches!(
            Cli::try_parse_from(["grove", "switch"]).unwrap().command,
            Some(Command::Switch)
        ));
        let error = Cli::try_parse_from(["grove", "switch", "--repo"])
            .err()
            .expect("--repo must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
}
