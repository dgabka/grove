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
    if config.roots.is_empty() {
        bail!("configuration has no roots");
    }
    if config.roots.iter().any(|root| !root.is_absolute()) {
        bail!("configuration roots must be absolute paths");
    }
    if config.max_depth == 0 {
        bail!("max_depth must be at least 1");
    }
    let mut names = HashSet::new();
    for preset in &config.presets {
        if preset.name.is_empty() || preset.name.contains(['\t', '\n', '\0']) {
            bail!("preset names must be non-empty and contain no tabs, newlines, or NULs");
        }
        if !names.insert(&preset.name) {
            bail!("preset names must be unique");
        }
        for window in &preset.windows {
            if window.name.is_empty() || window.name.contains('\0') {
                bail!("window names must be non-empty and contain no NULs");
            }
            for pane in &window.panes {
                if pane.command.first().is_some_and(String::is_empty) {
                    bail!("pane executable must not be empty");
                }
            }
        }
    }
    Ok(config)
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
}
