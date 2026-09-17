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
        elif [ -n "$REUSE_WORKTREE" ]; then
            printf '$7:old manually named session:7265706f:%s::::0\n' "$REUSE_WORKTREE"
        fi;;
    new-session) printf '$8\t@1\n';;
    rename-window|set-option|attach-session) :;;
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
        let output = self.run_mutating(choices, reuse, nerd_fonts, None);
        assert!(output.status.success(), "{:?}", output);
        output
    }

    fn run_mutating(
        &self,
        choices: &[&str],
        reuse: Option<&Path>,
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
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .output()
            .unwrap();
        assert_eq!(self.text("count"), choices.len().to_string());
        output
    }

    fn run_session(
        &self,
        args: &[&str],
        choices: &[&str],
        sessions: &str,
        current: &str,
        fail_tmux: &str,
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
            .env("TMUX", "fake,0,0")
            .env("TMUX_PANE", "%1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("REUSE_WORKTREE")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .output()
            .unwrap();
        assert_eq!(self.text("count"), choices.len().to_string());
        output
    }

    fn text(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }
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
    );
    assert!(output.status.success(), "{output:?}");
    let log = fixture.text("tmux.log");
    assert!(log.contains("display-message\0-p\0-t\0%1\0#{session_id}\0"));
    assert!(log.contains("list-sessions\0-F\0"));
    assert!(log.contains("switch-client\0-t\0$target\0"));
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
                    &choices,
                    (!layout).then_some(path.as_path()),
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
        let output =
            fixture.run_mutating(choices, None, false, Some((choices.len(), path, "missing")));
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
