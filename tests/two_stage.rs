use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    dir: TempDir,
    normal: PathBuf,
    linked: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repos");
        let normal = root.join("ordinary");
        fs::create_dir_all(&normal).unwrap();
        git(&normal, &["init", "-b", "main"]);
        git(&normal, &["config", "user.email", "a@b.c"]);
        git(&normal, &["config", "user.name", "a"]);
        git(&normal, &["commit", "--allow-empty", "-m", "initial"]);
        let bare = root.join("repo.git");
        git(
            &root,
            &[
                "clone",
                "--bare",
                normal.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let linked = bare.join("feature/topic");
        git(
            &bare,
            &[
                "worktree",
                "add",
                "-b",
                "feature/topic",
                linked.to_str().unwrap(),
            ],
        );
        git(&root, &["init", "--bare", "zzz.git"]);
        let config_dir = dir.path().join("config/grove");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            format!(
                "roots = [{:?}]\nmax_depth = 1\n[[presets]]\nname = 'custom'\n",
                root.to_str().unwrap()
            ),
        )
        .unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        script(
            &bin.join("fzf"),
            r#"
count=0
[ ! -f "$TEST_DIR/count" ] || count=$(cat "$TEST_DIR/count")
count=$((count + 1))
printf '%s' "$count" > "$TEST_DIR/count"
printf '%s\n' "$@" > "$TEST_DIR/picker-$count.args"
cat > "$TEST_DIR/picker-$count.input"
choice=$(cat "$TEST_DIR/choice-$count")
if [ "$count" = "$MUTATE_AT" ]; then
    case "$MUTATION" in
        missing) rm -rf "$MUTATE_PATH";;
        bare)
            rm -rf "$MUTATE_PATH"
            git clone --bare "$TEST_DIR/repos/zzz.git" "$MUTATE_PATH" >&2;;
        identity)
            rm -rf "$MUTATE_PATH/.git"
            git init --separate-git-dir "$TEST_DIR/replacement.git" "$MUTATE_PATH" >&2;;
        top) git -C "$MUTATE_PATH" config core.worktree ../..;;
        *) exit 92;;
    esac
fi
[ "$choice" != cancel ] || exit 130
# Deliberately unrelated human label: only the numeric ID may select a path.
printf '%s\tignored /wrong/path\0' "$choice"
"#,
        );
        script(
            &bin.join("tmux"),
            r#"
# Never permit accidental access to a normal tmux server, even in this fake.
[ "$1" = -L ] && [ "$2" = "$GROVE_TMUX_SOCKET" ] || exit 90
shift 2
printf '%s\0' "$@" >> "$TEST_DIR/tmux.log"
printf '\n' >> "$TEST_DIR/tmux.log"
case "$1" in
    display-message) printf '%s\n' "${CURRENT_SESSION-}";;
    list-sessions)
        if [ -n "${TMUX_SESSIONS-}" ]; then
            printf '%s' "$TMUX_SESSIONS"
        elif [ -n "${REUSE_WORKTREE-}" ]; then
            printf '$7:old manually named session:7265706f:%s::::0\n' "$REUSE_WORKTREE"
        fi;;
    new-session)
        [ "${FAIL_TMUX-}" != new-session ] || exit 1
        printf '$8\t@1\n';;
    rename-window|set-option|attach-session|new-window|split-window) :;;
    switch-client) [ "${FAIL_TMUX-}" != switch ];;
    kill-session) [ "${FAIL_TMUX-}" != kill ];;
    *) exit 91;;
esac
"#,
        );
        Self {
            dir,
            normal,
            linked,
        }
    }

    fn run(&self, choices: &[&str], reuse: Option<&Path>, nerd_fonts: bool) -> Output {
        let output = self.run_mutating(&[], choices, reuse, "", nerd_fonts, None);
        assert!(output.status.success(), "{:?}", output);
        output
    }

    fn run_mutating(
        &self,
        args: &[&str],
        choices: &[&str],
        reuse: Option<&Path>,
        sessions: &str,
        nerd_fonts: bool,
        mutation: Option<(usize, &Path, &str)>,
    ) -> Output {
        for entry in fs::read_dir(self.dir.path()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::remove_file(entry.path()).unwrap();
            }
        }
        for (i, choice) in choices.iter().enumerate() {
            fs::write(self.dir.path().join(format!("choice-{}", i + 1)), choice).unwrap();
        }
        let config = self.dir.path().join("config/grove/config.toml");
        let text = fs::read_to_string(&config).unwrap();
        // Add the preference before the preset table, not inside it.
        let text = text
            .lines()
            .filter(|line| !line.starts_with("nerd_fonts"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(config, format!("nerd_fonts = {nerd_fonts}\n{text}\n")).unwrap();
        let reuse = reuse
            .map(|path| {
                path.canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let (stage, path, mutation) = mutation.unwrap_or((0, self.dir.path(), ""));
        let output = Command::new(env!("CARGO_BIN_EXE_grove"))
            .args(args)
            .current_dir(self.dir.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("TEST_DIR", self.dir.path())
            .env("MUTATE_AT", stage.to_string())
            .env("MUTATE_PATH", path)
            .env("MUTATION", mutation)
            .env(
                "GROVE_TMUX_SOCKET",
                format!(
                    "grove-fake-{}",
                    self.dir.path().file_name().unwrap().to_string_lossy()
                ),
            )
            .env("REUSE_WORKTREE", reuse)
            .env("TMUX_SESSIONS", sessions)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GROVE_CONFIG")
            .output()
            .unwrap();
        assert_eq!(
            self.text("count"),
            if choices.is_empty() {
                String::new()
            } else {
                choices.len().to_string()
            }
        );
        output
    }

    fn run_session(
        &self,
        args: &[&str],
        choices: &[&str],
        sessions: &str,
        current: &str,
        fail_tmux: &str,
        tmux_pane: Option<bool>,
    ) -> Output {
        for entry in fs::read_dir(self.dir.path()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::remove_file(entry.path()).unwrap();
            }
        }
        for (i, choice) in choices.iter().enumerate() {
            fs::write(self.dir.path().join(format!("choice-{}", i + 1)), choice).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_grove"));
        command
            .args(args)
            .current_dir(self.dir.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("TEST_DIR", self.dir.path())
            .env("MUTATE_AT", "0")
            .env("MUTATE_PATH", self.dir.path())
            .env("MUTATION", "")
            .env(
                "GROVE_TMUX_SOCKET",
                format!(
                    "grove-fake-{}",
                    self.dir.path().file_name().unwrap().to_string_lossy()
                ),
            )
            .env("TMUX_SESSIONS", sessions)
            .env("CURRENT_SESSION", current)
            .env("FAIL_TMUX", fail_tmux)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("REUSE_WORKTREE")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GROVE_CONFIG");
        if let Some(has_pane) = tmux_pane {
            command.env("TMUX", "fake,0,0");
            if has_pane {
                command.env("TMUX_PANE", "%1");
            } else {
                command.env_remove("TMUX_PANE");
            }
        } else {
            command.env_remove("TMUX").env_remove("TMUX_PANE");
        }
        let output = command.output().unwrap();
        assert_eq!(
            self.text("count"),
            if choices.is_empty() {
                String::new()
            } else {
                choices.len().to_string()
            }
        );
        output
    }

    fn refresh(&self, defaults: &str, force: bool, sessions: &str, fail_tmux: &str) -> Output {
        fs::write(
            self.dir.path().join("config/grove/config.toml"),
            format!("{defaults}\n"),
        )
        .unwrap();
        self.run_session(
            if force {
                &["refresh", "--force"]
            } else {
                &["refresh"]
            },
            &[],
            sessions,
            "",
            fail_tmux,
            None,
        )
    }

    fn text(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }
}

#[test]
fn custom_config_path_overrides_standard_and_empty_value_falls_back() {
    let fixture = Fixture::new();
    let standard = fixture.dir.path().join("config/grove/config.toml");
    let alternate = fixture.dir.path().join("alternate.toml");
    fs::write(&standard, "roots = [").unwrap();
    fs::write(&alternate, "roots = []").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_grove"))
        .current_dir(fixture.dir.path())
        .env("HOME", fixture.dir.path())
        .env("XDG_CONFIG_HOME", fixture.dir.path().join("config"))
        .env("GROVE_CONFIG", &alternate)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no search roots configured"), "{stderr}");
    assert!(
        stderr.contains(&alternate.display().to_string()),
        "{stderr}"
    );
    assert!(!stderr.contains("parse configuration"), "{stderr}");

    let output = Command::new(env!("CARGO_BIN_EXE_grove"))
        .current_dir(fixture.dir.path())
        .env("HOME", fixture.dir.path())
        .env("XDG_CONFIG_HOME", fixture.dir.path().join("config"))
        .env("GROVE_CONFIG", "")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("parse configuration"), "{stderr}");
    assert!(stderr.contains(&standard.display().to_string()), "{stderr}");
}

#[test]
fn refresh_creates_missing_defaults_without_fzf_or_git() {
    let fixture = Fixture::new();
    script(
        &fixture.dir.path().join("bin/git"),
        "touch \"$TEST_DIR/git-called\"",
    );
    let cwd = fixture.dir.path().join("default cwd");
    fs::create_dir(&cwd).unwrap();
    let output = fixture.refresh(
        &format!(
            "[[defaults]]\nname = 'first'\ncwd = {:?}\n[[defaults]]\nname = 'second'\ncwd = {:?}",
            cwd, cwd
        ),
        false,
        "",
        "",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(fixture.text("count").is_empty());
    assert!(!fixture.dir.path().join("git-called").exists());
    let log = fixture.text("tmux.log");
    assert_eq!(log.matches("list-sessions\0-F\0").count(), 1);
    assert!(log.find("-s\0first\0").unwrap() < log.find("-s\0second\0").unwrap());
}

#[test]
fn refresh_skips_collisions_and_force_kills_ids_before_recreating() {
    let fixture = Fixture::new();
    let cwd = fixture.dir.path();
    let defaults = format!(
        "[[defaults]]\nname = 'main'\ncwd = {:?}\n[[defaults]]\nname = 'other'\ncwd = {:?}",
        cwd, cwd
    );
    let output = fixture.refresh(&defaults, false, "$foreign:main::::::0\n", "");
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(
        !log.contains("-s\0main\0") && log.contains("-s\0other\0"),
        "{log}"
    );

    let output = fixture.refresh(&defaults, true, "$one:main::::::0\n$two:other::::::0\n", "");
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    let kill_one = log.find("kill-session\0-t\0$one\0").unwrap();
    let create_one = log.find("-s\0main\0").unwrap();
    let kill_two = log.find("kill-session\0-t\0$two\0").unwrap();
    let create_two = log.find("-s\0other\0").unwrap();
    assert!(
        kill_one < create_one && create_one < kill_two && kill_two < create_two,
        "{log}"
    );
}

#[test]
fn refresh_stops_at_first_creation_error_and_preserves_literal_argv() {
    let fixture = Fixture::new();
    let cwd = fixture.dir.path();
    let defaults = format!(
        "[[defaults]]\nname = 'first'\ncwd = {:?}\n[[defaults.windows]]\nname = 'window'\n[[defaults.windows.panes]]\ncommand = ['echo space', '界', '$(literal)']\n[[defaults]]\nname = 'later'\ncwd = {:?}",
        cwd, cwd
    );
    let output = fixture.refresh(&defaults, false, "", "");
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(
        log.contains("--\0/usr/bin/env\0--\0echo space\0界\0$(literal)\0"),
        "{log}"
    );

    let output = fixture.refresh(&defaults, false, "", "new-session");
    assert!(!output.status.success());
    let log = fixture.text("tmux.log");
    assert!(
        log.contains("-s\0first\0") && !log.contains("-s\0later\0"),
        "{log}"
    );
}

#[test]
fn defaults_only_refresh_succeeds_but_open_requires_roots() {
    let fixture = Fixture::new();
    let cwd = fixture.dir.path();
    assert!(
        fixture
            .refresh(&format!("roots = [{cwd:?}]"), false, "", "")
            .status
            .success()
    );
    assert!(fixture.text("tmux.log").is_empty());
    let defaults = format!("[[defaults]]\nname = 'main'\ncwd = {:?}", cwd);
    assert!(fixture.refresh(&defaults, false, "", "").status.success());
    fs::write(
        fixture.dir.path().join("config/grove/config.toml"),
        defaults,
    )
    .unwrap();
    let output = fixture.run_session(&[], &[], "", "", "", None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no search roots configured"));
    assert!(fixture.text("tmux.log").is_empty());
}

#[test]
fn session_runner_uses_isolated_current_session_and_configurable_rows() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["switch"],
        &["0"],
        "$current:current:7265706f:::::2\n$target:target:7265706f:::::1\n",
        "$current",
        "",
        Some(true),
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(log.contains("display-message\0-p\0-t\0%1\0#{session_id}\0"));
    assert!(log.contains("list-sessions\0-F\0"));
    assert!(log.contains("switch-client\0-t\0$target\0"));
}

#[test]
fn switch_excludes_current_session_from_a_tmux_popup() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["switch"],
        &["0"],
        "$current:current::::::2\n$target:target::::::1\n",
        "$current",
        "",
        Some(false),
    );
    assert!(output.status.success(), "{output:?}");
    let input = fixture.text("picker-1.input");
    assert!(
        input.contains("target") && !input.contains("current"),
        "{input:?}"
    );
    let log = fixture.text("tmux.log");
    assert!(log.contains("display-message\0-p\0#{session_id}\0"));
    assert!(log.contains("switch-client\0-t\0$target\0"));
}

#[test]
fn close_switches_to_the_selected_stable_id_before_killing_current() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["close"],
        &["0"],
        "$current:human current:7265706f:::::2\n$target:unrelated label:7265706f:::::1\n",
        "$current",
        "",
        Some(true),
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    let switched = log.find("switch-client\0-t\0$target\0").unwrap();
    let killed = log.find("kill-session\0-t\0$current\0").unwrap();
    assert!(switched < killed, "{log}");
}

#[test]
fn close_switch_failure_does_not_kill_current() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["close"],
        &["0"],
        "$current:current::::::2\n$target:target::::::1\n",
        "$current",
        "switch",
        Some(true),
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("tmux switch-client -t $target failed")
    );
    let log = fixture.text("tmux.log");
    assert!(log.contains("switch-client\0-t\0$target\0"));
    assert!(!log.contains("kill-session"), "{log}");
}

#[test]
fn close_kill_failure_follows_successful_switch() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["close"],
        &["0"],
        "$current:current::::::2\n$target:target::::::1\n",
        "$current",
        "kill",
        Some(true),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("tmux kill-session -t $current"));
    let log = fixture.text("tmux.log");
    let switched = log.find("switch-client\0-t\0$target\0").unwrap();
    let killed = log.find("kill-session\0-t\0$current\0").unwrap();
    assert!(switched < killed, "{log}");
}

#[test]
fn close_cancellation_and_no_alternatives_are_noops() {
    let fixture = Fixture::new();
    let output = fixture.run_session(
        &["close"],
        &["cancel"],
        "$current:current::::::2\n$target:target::::::1\n",
        "$current",
        "",
        Some(true),
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(
        !log.contains("switch-client") && !log.contains("kill-session"),
        "{log}"
    );

    let output = fixture.run_session(
        &["close"],
        &[],
        "$current:current::::::2\n",
        "$current",
        "",
        Some(true),
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(
        !log.contains("switch-client") && !log.contains("kill-session"),
        "{log}"
    );
}

#[test]
fn close_outside_tmux_reports_a_clear_error() {
    let fixture = Fixture::new();
    let output = fixture.run_session(&["close"], &[], "", "", "", None);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("grove close must be run inside tmux")
    );
    assert!(fixture.text("tmux.log").is_empty());
}

#[test]
fn cancellation_at_repository_worktree_and_layout_and_empty_bare_are_noops() {
    let fixture = Fixture::new();
    for choices in [
        &["cancel"][..],
        &["1", "cancel"],
        &["0", "cancel"],
        &["1", "0", "cancel"],
    ] {
        fixture.run(choices, None, true);
        let tmux = fixture.text("tmux.log");
        assert!(!tmux.contains("new-session"));
        if choices.len() == 1 || choices == ["1", "cancel"] {
            assert!(tmux.is_empty());
        }
    }
    let output = fixture.run(&["2"], None, true);
    assert!(String::from_utf8_lossy(&output.stderr).contains("no active worktrees"));
    assert!(fixture.text("tmux.log").is_empty());
}

#[test]
fn normal_bypasses_worktree_picker_bare_uses_it_and_metadata_reuse_skips_layout() {
    let fixture = Fixture::new();
    for nerd_fonts in [true, false] {
        fixture.run(&["0", "0"], None, nerd_fonts);
        assert!(fixture.text("picker-1.args").contains("repository> "));
        assert!(fixture.text("picker-2.args").contains("layout> "));
        let initial = fixture.text("picker-1.input");
        assert!(initial.contains(if nerd_fonts {
            "\u{f418}  repo  "
        } else {
            "[bare]  repo  "
        }));
        let marker_width = if nerd_fonts { 1 } else { 6 };
        assert!(initial.starts_with(&format!("0\t{}ordinary", " ".repeat(marker_width + 2))));
        assert_eq!(
            fixture.text("picker-2.input"),
            concat!("0\tshell (one shell window)\0", "1\tcustom\0")
        );
        assert!(initial.contains("~/repos/ordinary"));
        assert!(initial.contains("~/repos/repo.git"));
        assert!(!initial.contains("feature/topic"));
        assert!(!initial.contains("branch:") && !initial.contains(''));
        let log = fixture.text("tmux.log");
        assert!(log.contains("-s\0ordinary\0"));
        assert!(log.contains(&format!(
            "-c\0{}\0",
            fixture.normal.canonicalize().unwrap().display()
        )));

        fixture.run(&["1", "0", "0"], None, nerd_fonts);
        assert!(fixture.text("picker-2.args").contains("worktree> "));
        assert!(fixture.text("picker-3.args").contains("layout> "));
        let linked = fixture.text("picker-2.input");
        assert!(linked.starts_with("0\t   repo/topic"));
        assert!(!linked.contains('\u{f418}') && !linked.contains("[bare]"));
        assert!(linked.contains("~/repos/repo.git/feature/topic"));
        assert!(linked.contains(if nerd_fonts {
            " feature/topic"
        } else {
            "branch: feature/topic"
        }));
        let log = fixture.text("tmux.log");
        assert!(log.contains("-s\0repo/topic\0"));
        assert!(log.contains(&format!(
            "-c\0{}\0",
            fixture.linked.canonicalize().unwrap().display()
        )));
    }
    git(&fixture.linked, &["branch", "-m", "renamed"]);
    for (choices, path) in [
        (&["0"][..], &fixture.normal),
        (&["1", "0"][..], &fixture.linked),
    ] {
        fixture.run(choices, Some(path), true);
        let log = fixture.text("tmux.log");
        assert!(log.contains("attach-session\0-t\0$7\0"));
        assert!(
            !log.contains("new-session")
                && !log.contains("rename-session")
                && !log.contains("set-option")
        );
    }
    fixture.run(&["1", "0", "0"], None, true);
    assert!(fixture.text("picker-2.input").contains(" renamed"));
    assert!(fixture.text("tmux.log").contains("-s\0repo/topic\0"));
}

#[test]
fn repository_and_worktree_picker_columns_align() {
    use unicode_width::UnicodeWidthStr;

    let fixture = Fixture::new();
    let bare = fixture.linked.parent().unwrap().parent().unwrap();
    let other = bare.join("界e\u{301}");
    git(
        bare,
        &["worktree", "add", "-b", "other", other.to_str().unwrap()],
    );
    fixture.run(&["1", "cancel"], None, false);
    for (stage, count) in [(1, 3), (2, 2)] {
        let input = fixture.text(&format!("picker-{stage}.input"));
        let labels: Vec<_> = input
            .strip_suffix('\0')
            .unwrap()
            .split('\0')
            .enumerate()
            .map(|(i, record)| {
                let (id, label) = record.split_once('\t').unwrap();
                assert_eq!(id, i.to_string());
                assert!(!label.ends_with(' '));
                label
            })
            .collect();
        assert_eq!(labels.len(), count);
        let path_columns: Vec<_> = labels
            .iter()
            .map(|label| label[..label.find("~/repos/").unwrap()].width())
            .collect();
        assert!(path_columns.iter().all(|column| *column == path_columns[0]));
        if stage == 2 {
            let branch_columns: Vec<_> = labels
                .iter()
                .map(|label| label[..label.find("branch:").unwrap()].width())
                .collect();
            assert_eq!(branch_columns[0], branch_columns[1]);
        }
    }
    assert!(fixture.text("tmux.log").is_empty());
}

#[test]
fn changed_checkout_is_rejected_after_checkout_and_layout_pickers() {
    for linked in [false, true] {
        for layout in [false, true] {
            for mutation in ["missing", "bare", "identity", "top"] {
                // core.worktree applies to primary checkouts, not linked ones.
                if linked && mutation == "top" {
                    continue;
                }
                let fixture = Fixture::new();
                let path = if linked {
                    &fixture.linked
                } else {
                    &fixture.normal
                };
                let mut choices = vec![if linked { "1" } else { "0" }];
                if linked {
                    choices.push("0");
                }
                if layout {
                    choices.push("0");
                }
                let output = fixture.run_mutating(
                    &[],
                    &choices,
                    (!layout).then_some(path.as_path()),
                    "",
                    false,
                    Some((choices.len(), path, mutation)),
                );
                assert!(!output.status.success(), "{linked} {layout} {mutation}");
                let error = String::from_utf8_lossy(&output.stderr);
                assert!(
                    error.contains("no longer the same non-bare Git worktree"),
                    "{error}"
                );
                assert!(error.contains("rerun grove and select it again"), "{error}");
                let log = fixture.text("tmux.log");
                for command in [
                    "new-session",
                    "switch-client",
                    "attach-session",
                    "set-option",
                ] {
                    assert!(!log.contains(command), "{log}");
                }
                if !layout {
                    assert!(log.is_empty(), "validation must precede session reuse");
                }
            }
        }
    }
}

#[test]
fn explicit_path_uses_named_preset_without_a_picker() {
    let fixture = Fixture::new();
    fs::write(
        fixture.dir.path().join("config/grove/config.toml"),
        "roots = []\n[[presets]]\nname = 'custom'\n",
    )
    .unwrap();
    let normal = fixture.normal.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", normal.to_str().unwrap(), "--preset", "custom"],
        &[],
        None,
        "",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(
        log.contains(&format!("-s\0ordinary\0-c\0{}\0", normal.display())),
        "{log}"
    );
    assert!(log.contains("attach-session\0-t\0$8\0"), "{log}");
}

#[test]
fn explicit_linked_path_uses_linked_name_and_metadata() {
    let fixture = Fixture::new();
    let linked = fixture.linked.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", linked.to_str().unwrap(), "--preset", "custom"],
        &[],
        None,
        "",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(log.contains("-s\0repo/topic\0"), "{log}");
    let worktree = linked
        .to_string_lossy()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(
        log.contains(&format!("@grove_worktree\0{worktree}\0")),
        "{log}"
    );
}

#[test]
fn explicit_path_without_preset_selects_or_cancels_a_layout() {
    let fixture = Fixture::new();
    let path = fixture.normal.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap()],
        &["1"],
        None,
        "",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert!(fixture.text("picker-1.args").contains("layout> "));
    assert!(fixture.text("tmux.log").contains("new-session"));

    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap()],
        &["cancel"],
        None,
        "",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!fixture.text("tmux.log").contains("new-session"));
}

#[test]
fn explicit_path_errors_and_reuses_before_preset_lookup() {
    let fixture = Fixture::new();
    let path = fixture.normal.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap(), "--preset", "missing"],
        &[],
        None,
        "",
        false,
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown preset \"missing\""));
    assert!(fixture.text("tmux.log").contains("list-sessions"));
    assert!(!fixture.text("tmux.log").contains("new-session"));

    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap(), "--preset", "missing"],
        &[],
        Some(&path),
        "",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert!(
        fixture
            .text("tmux.log")
            .contains("attach-session\0-t\0$7\0")
    );
    assert!(!fixture.dir.path().join("count").exists());
}

#[test]
fn explicit_path_preserves_collision_and_rejects_invalid_paths() {
    let fixture = Fixture::new();
    let path = fixture.normal.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap(), "--preset", "custom"],
        &[],
        None,
        "$foreign:ordinary::::::0\n",
        false,
        None,
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(log.contains("-s\0ordinary-"), "{log}");

    for invalid in [
        fixture.linked.parent().unwrap().parent().unwrap(),
        fixture.dir.path(),
    ] {
        let output = fixture.run_mutating(
            &["--path", invalid.to_str().unwrap(), "--preset", "custom"],
            &[],
            None,
            "",
            false,
            None,
        );
        assert!(!output.status.success());
        assert!(fixture.text("tmux.log").is_empty());
    }
}

#[test]
fn explicit_path_revalidates_after_layout_selection() {
    let fixture = Fixture::new();
    let path = fixture.normal.canonicalize().unwrap();
    let output = fixture.run_mutating(
        &["--path", path.to_str().unwrap()],
        &["0"],
        None,
        "",
        false,
        Some((1, &path, "missing")),
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no longer the same non-bare Git worktree")
    );
    assert!(!fixture.text("tmux.log").contains("new-session"));
}

#[test]
fn cancellation_after_checkout_disappears_remains_a_noop() {
    for choices in [
        &["cancel"][..],
        &["1", "cancel"],
        &["0", "cancel"],
        &["1", "0", "cancel"],
    ] {
        let fixture = Fixture::new();
        let path = if choices[0] == "1" {
            &fixture.linked
        } else {
            &fixture.normal
        };
        let output = fixture.run_mutating(
            &[],
            choices,
            None,
            "",
            false,
            Some((choices.len(), path, "missing")),
        );
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let log = fixture.text("tmux.log");
        for command in [
            "new-session",
            "switch-client",
            "attach-session",
            "set-option",
        ] {
            assert!(!log.contains(command), "{log}");
        }
    }
}
