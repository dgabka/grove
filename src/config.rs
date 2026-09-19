use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::HashSet, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    #[serde(default = "default_depth")]
    pub max_depth: usize,
    #[serde(default = "default_nerd_fonts")]
    pub nerd_fonts: bool,
    #[serde(default)]
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub defaults: Vec<DefaultSession>,
}
fn default_depth() -> usize {
    3
}
fn default_nerd_fonts() -> bool {
    true
}
impl Default for Config {
    fn default() -> Self {
        Self {
            roots: vec![],
            max_depth: default_depth(),
            nerd_fonts: default_nerd_fonts(),
            presets: vec![],
            defaults: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub windows: Vec<Window>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultSession {
    pub name: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub windows: Vec<Window>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub name: String,
    #[serde(default)]
    pub panes: Vec<Pane>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    #[serde(default)]
    pub command: Vec<String>,
}
impl Preset {
    pub fn shell() -> Self {
        Self {
            name: "shell".into(),
            windows: vec![Window {
                name: "shell".into(),
                panes: vec![],
            }],
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GROVE_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .join("grove/config.toml")
}
pub fn load() -> Result<Config> {
    let path = config_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read configuration {}", path.display()))?;
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parse configuration {}", path.display()))?;
    validate(config)
}

pub fn optional_nerd_fonts() -> Result<bool> {
    optional_nerd_fonts_from(&config_path())
}

fn optional_nerd_fonts_from(path: &std::path::Path) -> Result<bool> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config::default().nerd_fonts);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read configuration {}", path.display()));
        }
    };
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parse configuration {}", path.display()))?;
    Ok(config.nerd_fonts)
}

fn validate(config: Config) -> Result<Config> {
    if config.roots.iter().any(|root| !root.is_absolute()) {
        bail!("configuration roots must be absolute paths");
    }
    if config.max_depth == 0 {
        bail!("max_depth must be at least 1");
    }
    let mut preset_names = HashSet::new();
    for preset in &config.presets {
        if preset.name.is_empty() || preset.name.contains(['\t', '\n', '\0']) {
            bail!("preset names must be non-empty and contain no tabs, newlines, or NULs");
        }
        if !preset_names.insert(&preset.name) {
            bail!("preset names must be unique");
        }
        validate_windows(&preset.windows)?;
    }
    let mut default_names = HashSet::new();
    for default in &config.defaults {
        if default.name.is_empty()
            || default.name.contains(['.', ':'])
            || default.name.chars().any(char::is_control)
        {
            bail!(
                "default names must be non-empty and contain no periods, colons, or control characters"
            );
        }
        if !default_names.insert(&default.name) {
            bail!("default names must be unique");
        }
        if !default.cwd.is_absolute() || !default.cwd.is_dir() {
            bail!("default cwd must be an absolute existing directory");
        }
        validate_windows(&default.windows)?;
    }
    Ok(config)
}

fn validate_windows(windows: &[Window]) -> Result<()> {
    for window in windows {
        if window.name.is_empty() || window.name.contains('\0') {
            bail!("window names must be non-empty and contain no NULs");
        }
        for pane in &window.panes {
            if pane.command.first().is_some_and(String::is_empty) {
                bail!("pane executable must not be empty");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nerd_fonts_default_on_and_can_be_disabled() {
        let default: Config = toml::from_str("roots=['/x']").unwrap();
        let fallback: Config = toml::from_str("roots=['/x']\nnerd_fonts=false").unwrap();
        assert!(default.nerd_fonts);
        assert!(Config::default().nerd_fonts);
        assert_eq!(Config::default().max_depth, default.max_depth);
        assert!(!fallback.nerd_fonts);
    }

    #[test]
    fn optional_presentation_config_can_be_missing() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("missing.toml");
        assert!(optional_nerd_fonts_from(&path).unwrap());
        std::fs::write(&path, "nerd_fonts = false").unwrap();
        assert!(!optional_nerd_fonts_from(&path).unwrap());
    }

    #[test]
    fn rejects_bad_config() {
        assert!(toml::from_str::<Config>("roots=['/x']\nextra=1").is_err());
        assert!(validate(toml::from_str("roots=['relative']").unwrap()).is_err());
        assert!(validate(toml::from_str("roots=['/x']\nmax_depth=0").unwrap()).is_err());
        assert!(
            validate(
                toml::from_str("roots=['/x']\n[[presets]]\nname='x'\n[[presets]]\nname='x'")
                    .unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_documented_defaults_without_roots() {
        let directory = tempfile::TempDir::new().unwrap();
        let config = format!(
            "[[defaults]]\nname = 'main'\ncwd = {:?}\n\n[[defaults.windows]]\nname = 'editor'\n[[defaults.windows.panes]]\ncommand = ['nvim']\n\n[[defaults.windows]]\nname = 'shell'\n",
            directory.path()
        );
        let config: Config = toml::from_str(&config).unwrap();
        let config = validate(config).unwrap();
        assert_eq!(config.defaults.len(), 1);
        assert_eq!(config.defaults[0].cwd, directory.path());
        assert_eq!(config.defaults[0].windows.len(), 2);
    }

    #[test]
    fn rejects_duplicate_and_unsafe_default_names() {
        let directory = tempfile::TempDir::new().unwrap();
        let duplicate = format!(
            "[[defaults]]\nname = 'main'\ncwd = {:?}\n[[defaults]]\nname = 'main'\ncwd = {:?}",
            directory.path(),
            directory.path()
        );
        assert!(validate(toml::from_str(&duplicate).unwrap()).is_err());
        for name in ["", "a.b", "a:b", "a\tb"] {
            let config = format!(
                "[[defaults]]\nname = {name:?}\ncwd = {:?}",
                directory.path()
            );
            assert!(
                validate(toml::from_str(&config).unwrap()).is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_default_cwds() {
        let directory = tempfile::TempDir::new().unwrap();
        let file = directory.path().join("file");
        std::fs::write(&file, "").unwrap();
        for cwd in [
            PathBuf::from("relative"),
            directory.path().join("missing"),
            file,
        ] {
            let config = format!("[[defaults]]\nname = 'main'\ncwd = {cwd:?}");
            assert!(
                validate(toml::from_str(&config).unwrap()).is_err(),
                "{cwd:?}"
            );
        }
    }

    #[test]
    fn defaults_are_strict_and_validate_layouts() {
        let directory = tempfile::TempDir::new().unwrap();
        for layout in [
            "[[defaults.windows]]\nname = ''",
            "[[defaults.windows]]\nname = 'shell'\n[[defaults.windows.panes]]\ncommand = ['']",
        ] {
            let config = format!(
                "[[defaults]]\nname = 'main'\ncwd = {:?}\n{layout}",
                directory.path()
            );
            let config: Config = toml::from_str(&config).unwrap();
            assert!(validate(config).is_err(), "{layout}");
        }
        assert!(
            toml::from_str::<Config>("[[defaults]]\nname = 'main'\ncwd = '/'\nunknown = true")
                .is_err()
        );
    }
}
