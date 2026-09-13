pub mod config;
pub mod git;
pub mod tmux;

use anyhow::{Context, Result, bail};
use config::{Config, Preset};
use git::{Checkout, Repository, discover};
use std::{
    collections::HashSet,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tmux::{Session, Tmux};

pub fn choose(items: &[(String, String)], prompt: &str) -> Result<Option<String>> {
    choose_with(std::ffi::OsStr::new("fzf"), items, prompt)
}

fn choose_with(
    program: &std::ffi::OsStr,
    items: &[(String, String)],
    prompt: &str,
) -> Result<Option<String>> {
    if items.is_empty() {
        return Ok(None);
    }
    let input = items
        .iter()
        .map(|(id, label)| format!("{id}\t{label}\0"))
        .collect::<String>();
    let mut child = Command::new(program)
        .args([
            "--read0",
            "--print0",
            "--delimiter",
            "\t",
            "--with-nth",
            "2..",
            "--prompt",
            prompt,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start fzf")?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().context("wait for fzf")?;
    let write_error = writer.join().expect("fzf input writer panicked").err();
    if matches!(output.status.code(), Some(1 | 130)) {
        return Ok(None);
    }
    if !output.status.success() {
        bail!(
            "fzf failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    if let Some(error) = write_error {
        return Err(error).context("write choices to fzf");
    }
    let chosen = std::str::from_utf8(&output.stdout).context("fzf returned non-UTF-8 output")?;
    let Some(record) = chosen.strip_suffix('\0') else {
        bail!("fzf returned an invalid selection");
    };
    if record.contains('\0') {
        bail!("fzf returned multiple selections");
    }
    let Some((id, _)) = record.split_once('\t') else {
        bail!("fzf returned an invalid selection");
    };
    Ok(Some(id.to_owned()))
}

pub fn session_name(checkout: &Checkout, occupied: &HashSet<String>) -> String {
    use sha2::{Digest, Sha256};
    let clean = |value: &str, fallback: &str| -> String {
        if value.is_empty() {
            return fallback.into();
        }
        value
            .chars()
            .map(|c| {
                if c == ':' || c == '.' || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .collect()
    };
    let mut name = clean(&checkout.repo_name, "repo");
    if checkout.linked {
        name.push('/');
        name.push_str(&clean(
            checkout
                .worktree
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
            "worktree",
        ));
    }
    if !occupied.contains(&name) {
        return name;
    }
    let mut hash = Sha256::new();
    hash.update(checkout.worktree.to_string_lossy().as_bytes());
    let stem = format!("{name}-{}", &format!("{:x}", hash.finalize())[..10]);
    name = stem.clone();
    let mut n = 2;
    while occupied.contains(&name) {
        name = format!("{stem}-{n}");
        n += 1;
    }
    name
}

pub fn select_layout(config: &Config) -> Result<Option<Preset>> {
    let mut choices = vec![("0".to_owned(), "shell (one shell window)".to_owned())];
    choices.extend(
        config
            .presets
            .iter()
            .enumerate()
            .map(|(i, preset)| ((i + 1).to_string(), preset.name.clone())),
    );
    let Some(id) = choose(&choices, "layout> ")? else {
        return Ok(None);
    };
    let index = id
        .parse::<usize>()
        .context("fzf selected an invalid layout")?;
    if index == 0 {
        Ok(Some(Preset::shell()))
    } else {
        config
            .presets
            .get(index - 1)
            .cloned()
            .map(Some)
            .context("fzf selected an unknown layout")
    }
}

fn session_for_worktree<'a>(sessions: &'a [Session], worktree: &Path) -> Option<&'a Session> {
    sessions
        .iter()
        .find(|session| session.worktree.as_deref() == Some(worktree.to_string_lossy().as_ref()))
}

pub fn open(config: &Config, tmux: &Tmux) -> Result<()> {
    let repositories = discover(&config.roots, config.max_depth)?;
    if repositories.is_empty() {
        bail!(
            "no Git repositories found; configure search roots in {}",
            config::config_path().display()
        );
    }
    let choices: Vec<_> = repositories
        .iter()
        .enumerate()
        .map(|(i, entry)| (i.to_string(), entry.label(config.nerd_fonts)))
        .collect();
    let Some(id) = choose(&choices, "repository> ")? else {
        return Ok(());
    };
    let repository = repositories
        .get(id.parse::<usize>()?)
        .context("fzf selected an unknown repository")?;
    let checkouts;
    let checkout = match repository {
        Repository::Checkout(checkout) => checkout,
        Repository::Bare(bare) => {
            checkouts = git::worktrees(bare)?;
            if checkouts.is_empty() {
                eprintln!("no active worktrees in {}", bare.path.display());
                return Ok(());
            }
            let choices: Vec<_> = checkouts
                .iter()
                .enumerate()
                .map(|(i, checkout)| (i.to_string(), checkout.label(config.nerd_fonts)))
                .collect();
            let Some(id) = choose(&choices, "worktree> ")? else {
                return Ok(());
            };
            checkouts
                .get(id.parse::<usize>()?)
                .context("fzf selected an unknown worktree")?
        }
    };
    git::validate_checkout(checkout)?;
    let sessions = tmux.sessions()?;
    if let Some(existing) = session_for_worktree(&sessions, &checkout.worktree) {
        return tmux.navigate(&existing.id);
    }
    let Some(layout) = select_layout(config)? else {
        return Ok(());
    };
    git::validate_checkout(checkout)?;
    let occupied = sessions.iter().map(|s| s.name.clone()).collect();
    let name = session_name(checkout, &occupied);
    let id = tmux.create(&name, checkout, &layout)?;
    tmux.navigate(&id)
}

pub fn switch(tmux: &Tmux, repo_only: bool) -> Result<()> {
    let nerd_fonts = config::optional_nerd_fonts()?;
    let sessions = tmux.sessions()?;
    let current_repo = if repo_only {
        let in_grove = std::env::var_os("TMUX")
            .and_then(|_| tmux.current_session().ok().flatten())
            .and_then(|id| {
                sessions
                    .iter()
                    .find(|s| s.id == id)
                    .and_then(|s| s.repo.clone())
            });
        in_grove.or_else(|| git::repo_id_from_dir(Path::new(".")).ok().flatten())
    } else {
        None
    };
    if repo_only && current_repo.is_none() {
        bail!("cannot determine repository: run inside a Grove session or Git worktree");
    }
    let filtered: Vec<&Session> = sessions
        .iter()
        .filter(|session| !repo_only || session.repo.as_deref() == current_repo.as_deref())
        .collect();
    if filtered.is_empty() {
        bail!("no matching tmux sessions")
    };
    let choices: Vec<_> = filtered
        .iter()
        .enumerate()
        .map(|(i, s)| (i.to_string(), s.label(nerd_fonts)))
        .collect();
    if let Some(id) = choose(&choices, "session> ")? {
        let session = filtered
            .get(id.parse::<usize>()?)
            .context("fzf selected an unknown session")?;
        tmux.navigate(&session.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    fn script(body: &str) -> tempfile::TempPath {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(file.path()).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(file.path(), permissions).unwrap();
        file.into_temp_path()
    }

    #[test]
    fn session_names_have_exact_requested_shape_and_collide_by_path() {
        let mut checkout = Checkout {
            repo: "r".into(),
            worktree: PathBuf::from("/tmp/tree"),
            repo_name: "same repo_日本-a_b".into(),
            branch: Some("main".into()),
            linked: false,
        };
        assert_eq!(
            session_name(&checkout, &HashSet::new()),
            "same repo_日本-a_b"
        );
        checkout.linked = true;
        assert_eq!(
            session_name(&checkout, &HashSet::new()),
            "same repo_日本-a_b/tree"
        );
        checkout.branch = Some("feature".into());
        assert_eq!(
            session_name(&checkout, &HashSet::new()),
            "same repo_日本-a_b/tree"
        );

        checkout.repo_name = "bad:.\nname".into();
        assert_eq!(session_name(&checkout, &HashSet::new()), "bad---name/tree");
        checkout.repo_name = "same repo_日本-a_b".into();
        let collision = session_name(
            &checkout,
            &HashSet::from(["same repo_日本-a_b/tree".into()]),
        );
        assert!(collision.starts_with("same repo_日本-a_b/tree-"));
        assert_eq!(collision.len(), "same repo_日本-a_b/tree-".len() + 10);
    }

    #[test]
    fn picker_cancellation_wins_over_early_stdin_close() {
        let items = (0..20_000)
            .map(|i| (i.to_string(), "x".repeat(100)))
            .collect::<Vec<_>>();
        for code in [1, 130] {
            let fake = script(&format!("exit {code}"));
            assert_eq!(choose_with(fake.as_os_str(), &items, "test").unwrap(), None);
        }
    }

    #[test]
    fn picker_preserves_machine_id_and_rejects_bad_output() {
        let good = script("printf '7\\tlabel with\\nnewline\\0'");
        assert_eq!(
            choose_with(good.as_os_str(), &[("7".into(), "ignored".into())], "test")
                .unwrap()
                .as_deref(),
            Some("7")
        );
        for body in [
            "printf 'missing-nul'",
            "printf 'x\\0y\\0'",
            "printf 'no-tab\\0'",
        ] {
            let bad = script(body);
            assert!(choose_with(bad.as_os_str(), &[("0".into(), "x".into())], "test").is_err());
        }
    }

    #[test]
    fn picker_program_receives_nul_mode() {
        let fake = script("[ \"$1\" = --read0 ] || exit 2; cat >/dev/null; printf '0\\tx\\0'");
        assert_eq!(
            choose_with(fake.as_os_str(), &[("0".into(), "x".into())], "test")
                .unwrap()
                .as_deref(),
            Some("0")
        );
    }

    #[test]
    fn worktree_reuse_and_repository_grouping_use_metadata() {
        let sessions = vec![
            Session {
                id: "$1".into(),
                name: "unrelated name".into(),
                repo: Some("repo-a".into()),
                worktree: Some("/tmp/a".into()),
                label_meta: None,
                branch: None,
            },
            Session {
                id: "$2".into(),
                name: "grove-looking".into(),
                repo: Some("repo-b".into()),
                worktree: Some("/tmp/b".into()),
                label_meta: None,
                branch: None,
            },
            Session {
                id: "$3".into(),
                name: "foreign".into(),
                repo: None,
                worktree: None,
                label_meta: None,
                branch: None,
            },
        ];
        assert_eq!(
            session_for_worktree(&sessions, Path::new("/tmp/a"))
                .unwrap()
                .id,
            "$1"
        );
        let grouped = sessions
            .iter()
            .filter(|session| session.repo.as_deref() == Some("repo-a"))
            .collect::<Vec<_>>();
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].id, "$1");
    }
}
