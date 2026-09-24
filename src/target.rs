use crate::git::{self, Checkout};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// A directory Grove can open in a tmux session.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Target {
    Checkout(Checkout),
    Bookmark(PathBuf),
}

impl Target {
    pub fn cwd(&self) -> &Path {
        match self {
            Self::Checkout(checkout) => &checkout.worktree,
            Self::Bookmark(path) => path,
        }
    }

    /// Picker fields: marker, name, path, branch.
    pub fn columns(&self, nerd_fonts: bool) -> [String; 4] {
        match self {
            Self::Checkout(checkout) => checkout.columns(nerd_fonts),
            Self::Bookmark(path) => [
                if nerd_fonts { "\u{f02e}" } else { "[bookmark]" }.into(),
                bookmark_name(path),
                path.to_string_lossy().into_owned(),
                String::new(),
            ],
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Checkout(checkout) => checkout.display_name(),
            Self::Bookmark(path) => bookmark_name(path),
        }
    }

    pub fn repository(&self) -> Option<&str> {
        match self {
            Self::Checkout(checkout) => Some(&checkout.repo),
            Self::Bookmark(_) => None,
        }
    }

    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Checkout(checkout) => checkout.branch.as_deref(),
            Self::Bookmark(_) => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Checkout(_) => "checkout",
            Self::Bookmark(_) => "bookmark",
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Checkout(checkout) => git::validate_checkout(checkout),
            Self::Bookmark(path) if path.is_dir() => Ok(()),
            Self::Bookmark(path) => bail!(
                "selected bookmark {} is no longer an existing directory; rerun grove and select it again",
                path.display()
            ),
        }
    }
}

fn bookmark_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("bookmark")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout() -> Checkout {
        Checkout {
            repo: "/repos/example/.git".into(),
            worktree: PathBuf::from("/repos/example"),
            repo_name: "example".into(),
            branch: Some("main".into()),
            linked: false,
        }
    }

    #[test]
    fn bookmark_columns_use_the_configured_marker() {
        let target = Target::Bookmark(PathBuf::from("/tmp/notes"));
        assert_eq!(
            target.columns(true),
            ["\u{f02e}", "notes", "/tmp/notes", ""].map(str::to_owned)
        );
        assert_eq!(
            target.columns(false),
            ["[bookmark]", "notes", "/tmp/notes", ""].map(str::to_owned)
        );
    }

    #[test]
    fn bookmark_name_falls_back_without_a_basename() {
        assert_eq!(bookmark_name(Path::new("/")), "bookmark");
    }

    #[test]
    fn properties_distinguish_bookmarks_from_checkouts() {
        let checkout = checkout();
        let target = Target::Checkout(checkout.clone());
        assert_eq!(target.cwd(), checkout.worktree);
        assert_eq!(target.display_name(), "example");
        assert_eq!(target.repository(), Some("/repos/example/.git"));
        assert_eq!(target.branch(), Some("main"));
        assert_eq!(target.kind(), "checkout");

        let target = Target::Bookmark(PathBuf::from("/tmp/notes"));
        assert_eq!(target.cwd(), Path::new("/tmp/notes"));
        assert_eq!(target.display_name(), "notes");
        assert_eq!(target.repository(), None);
        assert_eq!(target.branch(), None);
        assert_eq!(target.kind(), "bookmark");
    }

    #[test]
    fn bookmark_validation_rejects_removed_or_replaced_paths() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("bookmark");
        std::fs::create_dir(&path).unwrap();
        let target = Target::Bookmark(path.clone());
        assert!(target.validate().is_ok());
        std::fs::remove_dir(&path).unwrap();
        assert!(target.validate().is_err());
        std::fs::write(&path, "not a directory").unwrap();
        assert!(target.validate().is_err());
    }

    #[test]
    fn checkout_columns_delegate_unchanged() {
        let checkout = checkout();
        assert_eq!(
            Target::Checkout(checkout.clone()).columns(true),
            checkout.columns(true)
        );
        assert_eq!(
            Target::Checkout(checkout.clone()).columns(false),
            checkout.columns(false)
        );
    }
}
