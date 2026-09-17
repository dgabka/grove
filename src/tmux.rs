use crate::{
    config::{Pane, Preset},
    git::Checkout,
};
use anyhow::{Context, Result, bail};
use std::{
    cmp::Reverse,
    process::{Command, Stdio},
};

fn hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn unhex(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(2) || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let decoded = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(decoded).ok()
}

fn tmux_path(path: &str) -> String {
    path.replace('#', "##")
}

#[derive(Debug, Clone)]
pub struct Session {
    /// Stable tmux machine identifier (for example `$1`).
    pub id: String,
    pub name: String,
    pub repo: Option<String>,
    pub worktree: Option<String>,
    pub label_meta: Option<String>,
    pub name_meta: Option<String>,
    pub branch: Option<String>,
    pub(crate) activity: u64,
}

fn recent_first(sessions: &mut [Session]) {
    sessions.sort_by_key(|session| Reverse(session.activity));
}

impl Session {
    pub fn columns(&self, nerd_fonts: bool) -> [String; 4] {
        match (&self.name_meta, &self.worktree) {
            (Some(name), Some(path)) => [
                String::new(),
                name.clone(),
                path.clone(),
                crate::branch_label(self.branch.as_deref(), nerd_fonts),
            ],
            _ => [
                String::new(),
                self.label(nerd_fonts),
                String::new(),
                String::new(),
            ],
        }
    }

    pub fn label(&self, nerd_fonts: bool) -> String {
        let base = self.label_meta.clone().unwrap_or_else(|| self.name.clone());
        match &self.branch {
            Some(branch) => format!(
                "{base}  {} {branch}",
                if nerd_fonts { "" } else { "branch:" }
            ),
            None => base,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tmux {
    socket: Option<String>,
    config: Option<String>,
}

impl Tmux {
    pub fn from_env() -> Self {
        Self {
            socket: std::env::var("GROVE_TMUX_SOCKET").ok(),
            config: None,
        }
    }

    fn command(&self, args: &[String], interactive: bool) -> Result<String> {
        let mut cmd = Command::new("tmux");
        if let Some(config) = &self.config {
            cmd.args(["-f", config]);
        }
        if let Some(socket) = &self.socket {
            cmd.args(["-L", socket]);
        }
        if interactive {
            let status = cmd
                .args(args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("start tmux")?;
            if !status.success() {
                bail!("tmux {} failed with {status}", args.join(" "));
            }
            return Ok(String::new());
        }
        let out = cmd.args(args).output().context("start tmux")?;
        if !out.status.success() {
            bail!(
                "tmux {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim_end()
            );
        }
        String::from_utf8(out.stdout).context("tmux returned non-UTF-8 output")
    }

    fn run(&self, args: &[String]) -> Result<String> {
        self.command(args, false)
    }

    fn refs(&self, args: &[&str]) -> Result<String> {
        self.run(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let output = match self.refs(&[
            "list-sessions",
            "-F",
            "#{session_id}:#{session_name}:#{@grove_repo}:#{@grove_worktree}:#{@grove_label}:#{@grove_name}:#{@grove_branch}:#{session_activity}",
        ]) {
            Ok(output) => output,
            Err(error)
                if error.to_string().contains("no server running on")
                    || error.to_string().contains("failed to connect to server")
                    || (error.to_string().contains("error connecting to")
                        && error.to_string().contains("(No such file or directory)")) =>
            {
                return Ok(vec![]);
            }
            Err(error) => return Err(error),
        };
        let mut sessions = output
            .lines()
            .map(|line| {
                let mut fields = line.splitn(8, ':');
                let mut field = || fields.next().context("tmux returned invalid session data");
                let id = field()?.to_owned();
                let name = field()?.to_owned();
                let option = |value: &str| (!value.is_empty()).then(|| unhex(value)).flatten();
                Ok(Session {
                    id,
                    name,
                    repo: option(field()?),
                    worktree: option(field()?),
                    label_meta: option(field()?),
                    name_meta: option(field()?),
                    branch: option(field()?),
                    activity: field()?
                        .parse()
                        .context("tmux returned invalid session activity")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        recent_first(&mut sessions);
        Ok(sessions)
    }

    pub fn current_session(&self) -> Result<Option<String>> {
        let Some(pane) = std::env::var_os("TMUX_PANE") else {
            return Ok(None);
        };
        let id = self.run(&[
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            pane.into_string()
                .map_err(|_| anyhow::anyhow!("TMUX_PANE is not valid Unicode"))?,
            "#{session_id}".into(),
        ])?;
        Ok(Some(id.trim().to_owned()))
    }

    pub fn navigate(&self, id: &str) -> Result<()> {
        let args = if std::env::var_os("TMUX").is_some() {
            vec!["switch-client".into(), "-t".into(), id.into()]
        } else {
            vec!["attach-session".into(), "-t".into(), id.into()]
        };
        self.command(&args, true)?;
        Ok(())
    }

    pub fn kill_session(&self, id: &str) -> Result<()> {
        self.run(&["kill-session".into(), "-t".into(), id.into()])?;
        Ok(())
    }

    fn with_command(args: &mut Vec<String>, pane: &Pane) {
        if !pane.command.is_empty() {
            args.extend(["--".into(), "/usr/bin/env".into(), "--".into()]);
            args.extend(pane.command.iter().map(|arg| {
                if arg.ends_with(';') && arg[..arg.len() - 1].bytes().all(|b| b == b'\\') {
                    format!("\\{arg}")
                } else {
                    arg.clone()
                }
            }));
        }
    }

    fn execute(&self, args: Vec<String>) -> Result<String> {
        self.run(&args)
    }

    pub fn create(&self, name: &str, checkout: &Checkout, preset: &Preset) -> Result<String> {
        let mut created_session = None;
        let result = (|| {
            let cwd = checkout.worktree.to_string_lossy().into_owned();
            let cwd_arg = tmux_path(&cwd);
            let first = preset
                .windows
                .first()
                .cloned()
                .unwrap_or_else(|| crate::config::Window {
                    name: "shell".into(),
                    panes: vec![],
                });
            let mut new = vec![
                "new-session".into(),
                "-d".into(),
                "-P".into(),
                "-F".into(),
                "#{session_id}\t#{window_id}".into(),
                "-s".into(),
                name.into(),
                "-c".into(),
                cwd_arg.clone(),
            ];
            if let Some(pane) = first.panes.first() {
                Self::with_command(&mut new, pane);
            }
            let created = self.execute(new)?;
            let (session, window) = created
                .trim_end()
                .split_once('\t')
                .context("tmux did not return session and window IDs")?;
            created_session = Some(session.to_owned());
            self.execute(vec![
                "rename-window".into(),
                "-t".into(),
                window.into(),
                first.name,
            ])?;
            let branch = checkout
                .linked
                .then(|| checkout.branch.clone().unwrap_or_else(|| "detached".into()));
            for (key, value) in [
                ("@grove_repo", Some(checkout.repo.clone())),
                ("@grove_worktree", Some(cwd.clone())),
                ("@grove_label", Some(checkout.base_label())),
                ("@grove_name", Some(checkout.display_name())),
                ("@grove_branch", branch),
            ] {
                let Some(value) = value else { continue };
                self.execute(vec![
                    "set-option".into(),
                    "-t".into(),
                    session.into(),
                    key.into(),
                    hex(&value),
                ])?;
            }
            self.add_panes(window, &first.panes, &cwd_arg)?;
            for window in preset.windows.iter().skip(1) {
                let mut args = vec![
                    "new-window".into(),
                    "-d".into(),
                    "-P".into(),
                    "-F".into(),
                    "#{window_id}".into(),
                    "-t".into(),
                    session.into(),
                    "-n".into(),
                    window.name.clone(),
                    "-c".into(),
                    cwd_arg.clone(),
                ];
                if let Some(pane) = window.panes.first() {
                    Self::with_command(&mut args, pane);
                }
                let id = self.execute(args)?;
                self.add_panes(id.trim_end(), &window.panes, &cwd_arg)?;
            }
            Ok(session.to_owned())
        })();
        if result.is_err()
            && let Some(session) = created_session.as_deref()
        {
            let _ = self.refs(&["kill-session", "-t", session]);
        }
        result
    }

    fn add_panes(&self, window: &str, panes: &[Pane], cwd: &str) -> Result<()> {
        for pane in panes.iter().skip(1) {
            let mut args = vec![
                "split-window".into(),
                "-d".into(),
                "-t".into(),
                window.into(),
                "-c".into(),
                cwd.into(),
            ];
            Self::with_command(&mut args, pane);
            self.execute(args)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    use tempfile::TempDir;

    struct Server {
        tmux: Tmux,
    }
    impl Server {
        fn new(config_text: &str) -> Option<Self> {
            Command::new("tmux").arg("-V").output().ok()?;
            let config = tempfile::NamedTempFile::new().ok()?;
            fs::write(config.path(), config_text).ok()?;
            let tmux = Tmux {
                socket: Some(format!(
                    "grove-test-{}-{}",
                    std::process::id(),
                    config.path().file_name().unwrap().to_string_lossy()
                )),
                config: Some(config.path().to_string_lossy().into_owned()),
            };
            // Keep the file alive by persisting it; Drop removes it after killing the server.
            let (_, path) = config.keep().ok()?;
            let mut server = Self { tmux };
            server.tmux.config = Some(path.to_string_lossy().into_owned());
            Some(server)
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.tmux.refs(&["kill-server"]);
            if let Some(path) = &self.tmux.config {
                let _ = fs::remove_file(path);
            }
        }
    }

    #[test]
    fn layout_metadata_literal_argv_and_base_index() {
        let Some(server) = Server::new("set -g base-index 1\nset -g pane-base-index 1\n") else {
            return;
        };
        let parent = TempDir::new().unwrap();
        let directory = parent.path().join("literal #{path}");
        fs::create_dir(&directory).unwrap();
        let output = parent.path().join("argv");
        let output2 = parent.path().join("argv2");
        let script = parent.path().join("one executable");
        let script2 = parent.path().join("many executable");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" > '{}'\nsleep 60\n",
                output.display()
            ),
        )
        .unwrap();
        fs::write(
            &script2,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\" \"$4\" > '{}'\nsleep 60\n",
                output2.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        for executable in [&script, &script2] {
            let mut permissions = fs::metadata(executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).unwrap();
        }
        let checkout = Checkout {
            repo: "repo\t\nidentity".into(),
            worktree: directory.canonicalize().unwrap(),
            repo_name: "repo".into(),
            branch: Some("main".into()),
            linked: true,
        };
        let preset = Preset {
            name: "x".into(),
            windows: vec![
                crate::config::Window {
                    name: "one".into(),
                    panes: vec![
                        Pane { command: vec![] },
                        Pane {
                            command: vec![script.to_string_lossy().into_owned()],
                        },
                    ],
                },
                crate::config::Window {
                    name: "two".into(),
                    panes: vec![Pane {
                        command: vec![
                            script2.to_string_lossy().into_owned(),
                            "space arg".into(),
                            "$(not-expanded)".into(),
                            ";".into(),
                            r"\;".into(),
                        ],
                    }],
                },
            ],
        };
        let id = server
            .tmux
            .create("grove-test", &checkout, &preset)
            .unwrap();
        let sessions = server.tmux.sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].repo.as_deref(), Some("repo\t\nidentity"));
        assert_eq!(
            sessions[0].worktree.as_deref(),
            Some(directory.canonicalize().unwrap().to_string_lossy().as_ref())
        );
        assert_eq!(sessions[0].label(true), checkout.label(true));
        assert_eq!(sessions[0].label(false), checkout.label(false));
        assert_eq!(sessions[0].branch.as_deref(), Some("main"));
        assert_eq!(
            sessions[0].name_meta.as_deref(),
            Some(checkout.display_name().as_str())
        );
        for nerd_fonts in [true, false] {
            assert_eq!(
                sessions[0].columns(nerd_fonts),
                checkout.columns(nerd_fonts)
            );
        }
        let windows = server
            .tmux
            .refs(&[
                "list-windows",
                "-t",
                &id,
                "-F",
                "#{window_index}:#{window_name}:#{window_panes}:#{pane_current_path}",
            ])
            .unwrap();
        assert!(windows.contains("1:one:2:"));
        assert!(windows.contains("2:two:1:"));
        let expected_cwd = directory.canonicalize().unwrap();
        assert!(
            windows
                .lines()
                .all(|line| line.ends_with(expected_cwd.to_string_lossy().as_ref()))
        );
        for _ in 0..200 {
            if output.exists() && output2.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(fs::read_to_string(output).unwrap(), "0\n\n\n");
        assert_eq!(
            fs::read_to_string(output2).unwrap(),
            "4\nspace arg\n$(not-expanded)\n;\n\\;\n"
        );
    }

    #[test]
    fn session_labels_render_new_branch_metadata_and_preserve_old_labels() {
        let mut session = Session {
            id: "$1".into(),
            name: "name".into(),
            repo: None,
            worktree: None,
            label_meta: Some("base".into()),
            name_meta: None,
            branch: Some("feature".into()),
            activity: 0,
        };
        assert_eq!(session.label(true), "base   feature");
        assert_eq!(session.label(false), "base  branch: feature");
        session.label_meta = Some("old   frozen".into());
        session.branch = None;
        assert_eq!(session.label(false), "old   frozen");
        assert_eq!(session.columns(false), ["", "old   frozen", "", ""]);
        session.name_meta = Some("friendly/worktree".into());
        session.worktree = Some("/path with spaces/界".into());
        session.branch = Some("feature".into());
        for nerd_fonts in [true, false] {
            assert_eq!(
                session.columns(nerd_fonts),
                [
                    String::new(),
                    "friendly/worktree".into(),
                    "/path with spaces/界".into(),
                    crate::branch_label(Some("feature"), nerd_fonts),
                ]
            );
        }
        session.branch = None;
        assert_eq!(session.columns(false)[3], "");
        session.name_meta = None;
        session.label_meta = None;
        assert_eq!(session.columns(false), ["", "name", "", ""]);
    }

    #[test]
    fn sessions_are_most_recent_first() {
        let mut sessions = [
            Session {
                id: "$1".into(),
                name: "old".into(),
                repo: None,
                worktree: None,
                label_meta: None,
                name_meta: None,
                branch: None,
                activity: 1,
            },
            Session {
                id: "$2".into(),
                name: "new".into(),
                repo: None,
                worktree: None,
                label_meta: None,
                name_meta: None,
                branch: None,
                activity: 2,
            },
        ];
        recent_first(&mut sessions);
        assert_eq!(sessions.map(|session| session.name), ["new", "old"]);
    }

    #[test]
    fn foreign_session_has_no_grove_metadata() {
        let Some(server) = Server::new("") else {
            return;
        };
        server
            .tmux
            .refs(&["new-session", "-d", "-s", "foreign"])
            .unwrap();
        let sessions = server.tmux.sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label(true), "foreign");
        assert_eq!(sessions[0].repo, None);
        assert_eq!(sessions[0].worktree, None);
        assert_eq!(sessions[0].label_meta, None);
        assert_eq!(sessions[0].name_meta, None);
        assert_eq!(sessions[0].columns(true), ["", "foreign", "", ""]);
        assert_eq!(sessions[0].branch, None);
    }

    #[test]
    fn failed_layout_rolls_back_created_session() {
        let Some(server) = Server::new("") else {
            return;
        };
        let directory = TempDir::new().unwrap();
        let checkout = Checkout {
            repo: "r".into(),
            worktree: directory.path().canonicalize().unwrap(),
            repo_name: "r".into(),
            branch: None,
            linked: false,
        };
        let preset = Preset {
            name: "bad".into(),
            windows: vec![crate::config::Window {
                name: "bad\0name".into(),
                panes: vec![],
            }],
        };
        assert!(server.tmux.create("rollback", &checkout, &preset).is_err());
        assert!(server.tmux.sessions().unwrap().is_empty());
    }
}
