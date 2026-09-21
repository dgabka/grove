use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "tmux sessions for Git worktrees",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[arg(long, value_parser = absolute_path)]
    path: Option<PathBuf>,
    #[arg(long, requires = "path")]
    preset: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

fn absolute_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    path.is_absolute()
        .then_some(path)
        .ok_or_else(|| "path must be absolute".to_owned())
}
#[derive(Subcommand)]
enum Command {
    Switch {
        #[arg(long)]
        repo: bool,
    },
    Close {
        #[arg(long)]
        repo: bool,
    },
    Refresh {
        #[arg(long)]
        force: bool,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let tmux = grove::tmux::Tmux::from_env();
    match cli.command {
        Some(Command::Switch { repo }) => grove::switch(&tmux, repo),
        Some(Command::Close { repo }) => grove::close(&tmux, repo),
        Some(Command::Refresh { force }) => grove::refresh(&grove::config::load()?, &tmux, force),
        None => {
            let config = grove::config::load()?;
            match cli.path {
                Some(path) => grove::open_path(&config, &tmux, &path, cli.preset.as_deref()),
                None => grove::open(&config, &tmux),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_and_close_accept_optional_repo_flag() {
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
        assert!(matches!(
            Cli::try_parse_from(["grove", "close"]).unwrap().command,
            Some(Command::Close { repo: false })
        ));
        assert!(matches!(
            Cli::try_parse_from(["grove", "close", "--repo"])
                .unwrap()
                .command,
            Some(Command::Close { repo: true })
        ));
        assert!(matches!(
            Cli::try_parse_from(["grove", "refresh", "--force"])
                .unwrap()
                .command,
            Some(Command::Refresh { force: true })
        ));
    }

    #[test]
    fn path_accepts_an_absolute_checkout_and_optional_preset() {
        let cli = Cli::try_parse_from(["grove", "--path", "/checkout"]).unwrap();
        assert_eq!(cli.path, Some(PathBuf::from("/checkout")));
        assert_eq!(cli.preset, None);

        let cli = Cli::try_parse_from(["grove", "--path", "/checkout", "--preset", "dev"]).unwrap();
        assert_eq!(cli.path, Some(PathBuf::from("/checkout")));
        assert_eq!(cli.preset.as_deref(), Some("dev"));
    }

    #[test]
    fn opening_arguments_reject_invalid_combinations() {
        assert!(Cli::try_parse_from(["grove", "--preset", "dev"]).is_err());
        assert!(Cli::try_parse_from(["grove", "--path", "checkout"]).is_err());
        for command in ["switch", "close", "refresh"] {
            assert!(Cli::try_parse_from(["grove", "--path", "/checkout", command]).is_err());
            assert!(
                Cli::try_parse_from(["grove", "--path", "/checkout", "--preset", "dev", command])
                    .is_err()
            );
        }
    }
}
