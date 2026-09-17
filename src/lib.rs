pub mod config;
pub mod git;
pub mod tmux;

use anyhow::{Context, Result, bail};
use config::{Config, Preset};
use git::{Checkout, Repository, discover};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tmux::{Session, Tmux};
use unicode_width::UnicodeWidthStr;

fn branch_label(branch: Option<&str>, nerd_fonts: bool) -> String {
    branch.map_or_else(String::new, |branch| {
        format!("{} {branch}", if nerd_fonts { "" } else { "branch:" })
    })
}

fn home_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home.filter(|path| path.is_absolute()) else {
        return Vec::new();
    };
    let mut paths = vec![home.to_path_buf()];
    if let Ok(canonical) = home.canonicalize()
        && canonical != home
    {
        paths.push(canonical);
    }
    paths
}

fn aligned_choices(rows: impl Iterator<Item = [String; 4]>) -> Vec<(String, String)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    aligned_choices_with_home(rows, &home_paths(home.as_deref()))
}

fn aligned_choices_with_home(
    rows: impl Iterator<Item = [String; 4]>,
    homes: &[PathBuf],
) -> Vec<(String, String)> {
    let rows: Vec<_> = rows
        .map(|mut row| {
            if let Some(relative) = homes
                .iter()
                .find_map(|home| Path::new(&row[2]).strip_prefix(home).ok())
            {
                row[2] = if relative.as_os_str().is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", relative.display())
                };
            }
            row.map(|field| {
                field.chars().fold(String::new(), |mut text, c| {
                    if c.is_control() {
                        text.extend(c.escape_default());
                    } else {
                        text.push(c);
                    }
                    text
                })
            })
        })
        .collect();
    // Keep a blank marker slot even when every entry is a checkout or session.
    let mut widths = [1, 0, 0, 0];
    for row in &rows {
        for (i, field) in row.iter().enumerate() {
            widths[i] = widths[i].max(field.width());
        }
    }
    rows.into_iter()
        .enumerate()
        .map(|(id, row)| {
            let last = row.iter().rposition(|field| !field.is_empty()).unwrap_or(0);
            let mut label = String::new();
            for (i, field) in row.iter().enumerate().take(last + 1) {
                label.push_str(field);
                if i < last {
                    label.push_str(&" ".repeat(widths[i] - field.width() + 2));
                }
            }
            (id.to_string(), label)
        })
        .collect()
}

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
            "--height",
            "100%",
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
    let _write_result = writer.join().expect("fzf input writer panicked");
    if matches!(output.status.code(), Some(1 | 130)) {
        return Ok(None);
    }
    if !output.status.success() {
        bail!(
            "fzf failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
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
    let choices = aligned_choices(
        repositories
            .iter()
            .map(|entry| entry.columns(config.nerd_fonts)),
    );
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
            let choices = aligned_choices(
                checkouts
                    .iter()
                    .map(|checkout| checkout.columns(config.nerd_fonts)),
            );
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

fn selectable_sessions(
    sessions: Vec<Session>,
    current: Option<&str>,
    repo_only: bool,
) -> Vec<Session> {
    let current_repo = repo_only
        .then(|| {
            sessions
                .iter()
                .find(|session| Some(session.id.as_str()) == current)
        })
        .flatten()
        .and_then(|session| session.repo.as_deref())
        .map(str::to_owned);
    let restrict = current_repo.as_deref().is_some_and(|repo| {
        sessions.iter().any(|session| {
            Some(session.id.as_str()) != current && session.repo.as_deref() == Some(repo)
        })
    });
    sessions
        .into_iter()
        .filter(|session| {
            Some(session.id.as_str()) != current
                && (!restrict || session.repo.as_deref() == current_repo.as_deref())
        })
        .collect()
}

pub fn switch(tmux: &Tmux, repo_only: bool) -> Result<()> {
    let nerd_fonts = config::optional_nerd_fonts()?;
    let current = tmux.current_session()?;
    let sessions = selectable_sessions(tmux.sessions()?, current.as_deref(), repo_only);
    if sessions.is_empty() {
        bail!("no matching tmux sessions")
    };
    let choices = aligned_choices(sessions.iter().map(|session| session.columns(nerd_fonts)));
    if let Some(id) = choose(&choices, "session> ")? {
        let session = sessions
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
        let path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn bare_markers_align_separately_from_unicode_names_paths_and_branches() {
        let checkout = Checkout {
            repo: "/home/ann/repo/.git".into(),
            worktree: "/home/ann/e\u{301}".into(),
            repo_name: "e\u{301}".into(),
            branch: Some("main".into()),
            linked: false,
        };
        let repositories = [
            Repository::Bare(git::BareRepository {
                common: "/home/ann/界.git".into(),
                path: "/home/ann/界.git".into(),
            }),
            Repository::Checkout(checkout.clone()),
            Repository::Checkout(Checkout {
                worktree: "/home/ann/短".into(),
                linked: true,
                ..checkout.clone()
            }),
            Repository::Checkout(Checkout {
                worktree: "/home/ann/long".into(),
                linked: true,
                ..checkout
            }),
        ];
        for nerd_fonts in [true, false] {
            let rows: Vec<_> = repositories
                .iter()
                .map(|repo| repo.columns(nerd_fonts))
                .collect();
            let marker = if nerd_fonts { "\u{f418}" } else { "[bare]" };
            assert_eq!(rows[0][0], marker);
            assert!(rows[1..].iter().all(|row| row[0].is_empty()));
            let choices =
                aligned_choices_with_home(rows.clone().into_iter(), &["/home/ann".into()]);
            let name_column = marker.width() + 2;
            let path_column =
                name_column + rows.iter().map(|row| row[1].width()).max().unwrap() + 2;
            for (i, (id, label)) in choices.iter().enumerate() {
                assert_eq!(id, &i.to_string());
                assert_eq!(
                    label[..label.find(&rows[i][1]).unwrap()].width(),
                    name_column
                );
                assert_eq!(label[..label.find("~/").unwrap()].width(), path_column);
                assert!(!label.ends_with(' '));
                if i > 0 {
                    assert!(label.starts_with(&" ".repeat(name_column)));
                    assert!(!label.contains(marker));
                }
            }
            assert_eq!(choices[0].1, format!("{marker}  界      ~/界.git"));
            let branch = if nerd_fonts { "" } else { "branch:" };
            assert_eq!(
                choices[2].1[..choices[2].1.find(branch).unwrap()].width(),
                choices[3].1[..choices[3].1.find(branch).unwrap()].width()
            );
        }
    }

    #[test]
    fn normal_only_repository_picker_reserves_a_blank_marker_slot() {
        let repository = Repository::Checkout(Checkout {
            repo: "/repo/.git".into(),
            worktree: "/repo".into(),
            repo_name: "界".into(),
            branch: Some("main".into()),
            linked: false,
        });
        for nerd_fonts in [true, false] {
            let columns = repository.columns(nerd_fonts);
            assert_eq!(columns, ["", "界", "/repo", ""]);
            assert_eq!(
                aligned_choices_with_home(std::iter::once(columns), &[]),
                [("0".into(), "   界  /repo".into())]
            );
        }
    }

    #[test]
    fn home_paths_shorten_only_component_descendants() {
        let homes = [PathBuf::from("/home/ann")];
        for (path, expected) in [
            ("/home/ann", "~"),
            ("/home/ann/repo", "~/repo"),
            ("/home/ann/space 界\n", "~/space 界\\n"),
            ("/home/anna/repo", "/home/anna/repo"),
            ("/home/ann-other", "/home/ann-other"),
            ("/outside/repo", "/outside/repo"),
            ("", ""),
        ] {
            let rows = [[String::new(), String::new(), path.into(), String::new()]];
            let label = aligned_choices_with_home(rows.into_iter(), &homes)
                .remove(0)
                .1;
            assert_eq!(label.trim_start(), expected);
        }
        for home in [None, Some(Path::new("")), Some(Path::new("relative"))] {
            let homes = home_paths(home);
            assert!(homes.is_empty());
            let rows = [[
                String::new(),
                "repo".into(),
                "/home/ann/repo".into(),
                String::new(),
            ]];
            assert_eq!(
                aligned_choices_with_home(rows.into_iter(), &homes)[0].1,
                "   repo  /home/ann/repo"
            );
        }
    }

    #[test]
    fn symlinked_home_matches_both_lexical_and_canonical_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("real home");
        let alias = dir.path().join("home link");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let homes = home_paths(Some(&alias));
        for home in [&alias, &target.canonicalize().unwrap()] {
            let rows = [[
                String::new(),
                "repo".into(),
                home.join("界 repo").to_string_lossy().into_owned(),
                String::new(),
            ]];
            assert_eq!(
                aligned_choices_with_home(rows.into_iter(), &homes)[0].1,
                "   repo  ~/界 repo"
            );
        }
    }

    #[test]
    fn shortened_paths_are_measured_after_escaping_by_terminal_width() {
        let rows = [
            [
                String::new(),
                "界".into(),
                "/long/home/短".into(),
                "branch: one".into(),
            ],
            [
                String::new(),
                "e\u{301}".into(),
                "/long/home/e\u{301}\t".into(),
                "branch: two".into(),
            ],
        ];
        let choices = aligned_choices_with_home(rows.into_iter(), &["/long/home".into()]);
        assert_eq!(choices[0], ("0".into(), "   界  ~/短   branch: one".into()));
        assert_eq!(
            choices[1],
            (
                "1".into(),
                "   e\u{301}   ~/e\u{301}\\t  branch: two".into()
            )
        );
        for (_, label) in choices {
            assert_eq!(label[..label.find('~').unwrap()].width(), 7);
            assert_eq!(label[..label.find("branch:").unwrap()].width(), 14);
        }
    }

    #[test]
    fn session_shortening_preserves_names_branches_legacy_labels_and_identity() {
        let mut session = Session {
            id: "$7".into(),
            name: "actual name".into(),
            repo: Some("/home/ann/repo/.git".into()),
            worktree: Some("/home/ann/repo".into()),
            label_meta: Some("legacy  /home/ann/repo".into()),
            name_meta: Some("/home/ann/name".into()),
            branch: Some("/home/ann/branch".into()),
            activity: 0,
        };
        let homes = ["/home/ann".into()];
        let choices = aligned_choices_with_home(std::iter::once(session.columns(false)), &homes);
        assert_eq!(
            choices[0],
            (
                "0".into(),
                "   /home/ann/name  ~/repo  branch: /home/ann/branch".into()
            )
        );
        assert_eq!(session.id, "$7");
        assert_eq!(session.name, "actual name");
        assert_eq!(session.repo.as_deref(), Some("/home/ann/repo/.git"));
        assert_eq!(session.worktree.as_deref(), Some("/home/ann/repo"));
        session.name_meta = None;
        session.branch = None;
        let choices = aligned_choices_with_home(std::iter::once(session.columns(false)), &homes);
        assert_eq!(choices[0].1, "   legacy  /home/ann/repo");
        session.label_meta = None;
        session.name = "/home/ann/foreign".into();
        let choices = aligned_choices_with_home(std::iter::once(session.columns(false)), &homes);
        assert_eq!(choices[0].1, "   /home/ann/foreign");
    }

    #[test]
    fn columns_align_by_terminal_width_without_trailing_padding() {
        let rows = [
            [
                String::new(),
                "界".into(),
                "/短".into(),
                "branch: one".into(),
            ],
            [
                String::new(),
                "e\u{301}".into(),
                "/long".into(),
                "branch: two".into(),
            ],
            [String::new(), "main".into(), "/m".into(), String::new()],
            [
                String::new(),
                "old label".into(),
                String::new(),
                String::new(),
            ],
        ];
        let choices = aligned_choices_with_home(rows.into_iter(), &[]);
        assert_eq!(
            choices[0],
            ("0".into(), "   界         /短    branch: one".into())
        );
        assert_eq!(
            choices[1],
            ("1".into(), "   e\u{301}          /long  branch: two".into())
        );
        assert_eq!(choices[2].1, "   main       /m");
        assert_eq!(choices[3].1, "   old label");
        for (_, label) in &choices[..2] {
            assert_eq!(label[..label.find('/').unwrap()].width(), 14);
            assert_eq!(label[..label.find("branch:").unwrap()].width(), 21);
        }
        assert!(aligned_choices_with_home(std::iter::empty(), &[]).is_empty());
    }

    #[test]
    fn displayed_controls_are_escaped_without_changing_picker_ids_or_nul_transport() {
        let rows = [[
            String::new(),
            "a\tb".into(),
            "/p\n\r\0\u{1b}".into(),
            String::new(),
        ]];
        let choices = aligned_choices_with_home(rows.clone().into_iter(), &[]);
        assert_eq!(choices[0].1, r"   a\tb  /p\n\r\u{0}\u{1b}");
        assert_eq!(rows[0][2], "/p\n\r\0\u{1b}");
        let fake = script(
            "[ \"$1\" = --read0 ] && [ \"$2\" = --print0 ] && [ \"$3\" = --delimiter ] && [ \"$5\" = --with-nth ] && [ \"$6\" = 2.. ] || exit 2; cat",
        );
        assert_eq!(
            choose_with(fake.as_os_str(), &choices, "test").unwrap(),
            Some("0".into())
        );
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
        let items = (0..2_000)
            .map(|i| (i.to_string(), "x".repeat(100)))
            .collect::<Vec<_>>();
        assert_eq!(
            choose_with(good.as_os_str(), &items, "test")
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
    fn selectable_sessions_filters_current_metadata_with_fallback() {
        let session = |id: &str, repo: Option<&str>| Session {
            id: id.into(),
            name: id.into(),
            repo: repo.map(str::to_owned),
            worktree: None,
            label_meta: None,
            name_meta: None,
            branch: None,
            activity: 0,
        };
        let sessions = vec![
            session("$current", Some("/repo/.git")),
            session("$match", Some("/repo/.git")),
            session("$other", Some("/other/.git")),
            session("$foreign", None),
        ];
        for (current, repo_only, expected) in [
            (Some("$current"), true, vec!["$match"]),
            (
                Some("$current"),
                false,
                vec!["$match", "$other", "$foreign"],
            ),
            (Some("$foreign"), true, vec!["$current", "$match", "$other"]),
            (
                Some("$missing"),
                true,
                vec!["$current", "$match", "$other", "$foreign"],
            ),
        ] {
            assert_eq!(
                selectable_sessions(sessions.clone(), current, repo_only)
                    .iter()
                    .map(|session| session.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }

        let no_match = vec![
            session("$current", Some("/repo/.git")),
            session("$other", Some("/other/.git")),
            session("$foreign", None),
        ];
        assert_eq!(
            selectable_sessions(no_match, Some("$current"), true)
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["$other", "$foreign"]
        );
    }

    #[test]
    fn worktree_reuse_uses_metadata() {
        let sessions = vec![
            Session {
                id: "$1".into(),
                name: "unrelated name".into(),
                repo: Some("repo-a".into()),
                worktree: Some("/tmp/a".into()),
                label_meta: None,
                name_meta: None,
                branch: None,
                activity: 0,
            },
            Session {
                id: "$2".into(),
                name: "grove-looking".into(),
                repo: Some("repo-b".into()),
                worktree: Some("/tmp/b".into()),
                label_meta: None,
                name_meta: None,
                branch: None,
                activity: 0,
            },
            Session {
                id: "$3".into(),
                name: "foreign".into(),
                repo: None,
                worktree: None,
                label_meta: None,
                name_meta: None,
                branch: None,
                activity: 0,
            },
        ];
        assert_eq!(
            session_for_worktree(&sessions, Path::new("/tmp/a"))
                .unwrap()
                .id,
            "$1"
        );
    }
}
