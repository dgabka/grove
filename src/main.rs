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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_accepts_optional_repo_flag() {
        assert!(matches!(
            Cli::try_parse_from(["grove", "switch"]).unwrap().command,
            Some(Command::Switch { repo: false })
        ));
        assert!(matches!(
            Cli::try_parse_from(["grove", "switch", "--repo"])
                .unwrap()
                .command,
            Some(Command::Switch { repo: true })
        ));
    }
}
