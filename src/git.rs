use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};
use walkdir::{DirEntry, WalkDir};

#[cfg(test)]
thread_local! {
    static GIT_INVOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WORKTREE_LISTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn git_command() -> Command {
    #[cfg(test)]
    GIT_INVOCATIONS.with(|count| count.set(count.get() + 1));
    Command::new("git")
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Checkout {
    pub repo: String,
    pub worktree: PathBuf,
    pub repo_name: String,
    pub branch: Option<String>,
    pub linked: bool,
}
impl Checkout {
    pub fn display_name(&self) -> String {
        if self.linked {
            format!(
                "{}/{}",
                self.repo_name,
                self.worktree
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or("worktree")
            )
        } else {
            self.repo_name.clone()
        }
    }

    pub fn base_label(&self) -> String {
        format!("{}  {}", self.display_name(), self.worktree.display())
    }

    /// Picker fields: marker, name, path, branch.
    pub fn columns(&self, nerd_fonts: bool) -> [String; 4] {
        [
            String::new(),
            self.display_name(),
            self.worktree.to_string_lossy().into_owned(),
            crate::branch_label(
                self.linked
                    .then(|| self.branch.as_deref().unwrap_or("detached")),
                nerd_fonts,
            ),
        ]
    }

    pub fn label(&self, nerd_fonts: bool) -> String {
        let base = self.base_label();
        if self.linked {
            format!(
                "{base}  {} {}",
                if nerd_fonts { "" } else { "branch:" },
                self.branch.as_deref().unwrap_or("detached")
            )
        } else {
            base
        }
    }
}

/// A bare repository is a picker entry, never a session checkout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Repository {
    Checkout(Checkout),
    Bare(BareRepository),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BareRepository {
    pub common: PathBuf,
    pub path: PathBuf,
}

impl Repository {
    pub fn columns(&self, nerd_fonts: bool) -> [String; 4] {
        match self {
            Self::Checkout(checkout) => checkout.columns(nerd_fonts),
            Self::Bare(bare) => [
                if nerd_fonts { "\u{f418}" } else { "[bare]" }.into(),
                display_name(&bare.common),
                bare.path.to_string_lossy().into_owned(),
                String::new(),
            ],
        }
    }

    pub fn label(&self, nerd_fonts: bool) -> String {
        match self {
            Self::Checkout(checkout) => checkout.label(nerd_fonts),
            Self::Bare(bare) => format!(
                "{} [bare]  {}",
                display_name(&bare.common),
                bare.path.display()
            ),
        }
    }
}

fn output_without_newline(value: String) -> String {
    value.strip_suffix('\n').unwrap_or(&value).to_owned()
}
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    #[cfg(test)]
    if args.starts_with(&["worktree", "list"]) {
        WORKTREE_LISTS.with(|count| count.set(count.get() + 1));
    }
    let out = git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .with_context(|| format!("run git in {}", dir.display()))?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }
    String::from_utf8(out.stdout)
        .map(output_without_newline)
        .context("Git returned non-UTF-8 output")
}
fn canonical(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))
}
fn bare_marker(path: &Path) -> bool {
    path.join("HEAD").is_file() && path.join("objects").is_dir() && path.join("refs").is_dir()
}
fn root_candidate(path: &Path) -> Option<PathBuf> {
    (path.join(".git").exists() || bare_marker(path))
        .then(|| canonical(path).ok())
        .flatten()
}

fn git_file(path: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(path).ok()?;
    let target = contents.lines().next()?.strip_prefix("gitdir: ")?;
    canonical(&path.parent()?.join(target)).ok()
}

fn common_dir(git_dir: &Path) -> PathBuf {
    std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .and_then(|path| canonical(&git_dir.join(path.trim_end())).ok())
        .unwrap_or_else(|| git_dir.to_owned())
}

fn branch(git_dir: &Path) -> Option<String> {
    std::fs::read_to_string(git_dir.join("HEAD"))
        .ok()?
        .strip_prefix("ref: refs/heads/")
        .map(|branch| branch.trim_end().to_owned())
}

fn is_bare(git_dir: &Path) -> bool {
    let Ok(config) = std::fs::read_to_string(git_dir.join("config")) else {
        return true;
    };
    let mut core = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            core = line.eq_ignore_ascii_case("[core]");
        } else if core
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("bare")
        {
            return value.trim().eq_ignore_ascii_case("true");
        }
    }
    true
}

fn inspect(anchor: PathBuf) -> Option<Repository> {
    let dot_git = anchor.join(".git");
    if dot_git.is_file() {
        let git_dir = git_file(&dot_git)?;
        if bare_marker(&git_dir) && is_bare(&git_dir) {
            return Some(Repository::Bare(BareRepository {
                common: git_dir,
                path: anchor,
            }));
        }
        let common = common_dir(&git_dir);
        return Some(Repository::Checkout(Checkout {
            repo: common.to_string_lossy().into_owned(),
            repo_name: display_name(&common),
            worktree: anchor,
            branch: branch(&git_dir),
            linked: git_dir != common,
        }));
    }
    if dot_git.is_dir() {
        let git_dir = canonical(&dot_git).ok()?;
        return Some(Repository::Checkout(Checkout {
            repo: git_dir.to_string_lossy().into_owned(),
            repo_name: display_name(&git_dir),
            worktree: anchor,
            branch: branch(&git_dir),
            linked: false,
        }));
    }
    (bare_marker(&anchor) && is_bare(&anchor)).then(|| {
        Repository::Bare(BareRepository {
            common: anchor.clone(),
            path: anchor,
        })
    })
}
fn descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || entry.file_name() != ".git" {
        return !matches!(
            entry.file_name().to_str(),
            Some("objects" | "refs" | "hooks" | "logs")
        ) || !entry.path().parent().is_some_and(bare_marker);
    }
    false
}

#[derive(Clone, Debug)]
struct Probe {
    common: PathBuf,
    git_dir: PathBuf,
    top: Option<PathBuf>,
    bare: bool,
}

fn probe(path: &Path, cache: &mut HashMap<PathBuf, Option<Probe>>) -> Option<Probe> {
    let key = canonical(path).ok()?;
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let result = (|| {
        // One path per invocation keeps Git's trailing LF as framing while preserving
        // embedded newlines and a trailing carriage return in the path itself.
        let common = git(
            &key,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .and_then(|path| canonical(Path::new(&path)).ok())?;
        let git_dir = git(&key, &["rev-parse", "--absolute-git-dir"])
            .ok()
            .and_then(|path| canonical(Path::new(&path)).ok())?;
        let top = git(&key, &["rev-parse", "--show-toplevel"])
            .ok()
            .and_then(|path| canonical(Path::new(&path)).ok());
        let bare = top.is_none()
            && git(&key, &["rev-parse", "--is-bare-repository"])
                .ok()
                .as_deref()
                == Some("true");
        Some(Probe {
            common,
            git_dir,
            top,
            bare,
        })
    })();
    cache.insert(key, result.clone());
    result
}

/// Picker waits can invalidate discovery metadata; never reuse a cached probe here.
pub fn validate_checkout(checkout: &Checkout) -> Result<()> {
    if probe(&checkout.worktree, &mut HashMap::new()).is_some_and(|info| {
        !info.bare
            && info.top.as_ref() == Some(&checkout.worktree)
            && info.common.to_string_lossy() == checkout.repo
            && (info.git_dir != info.common) == checkout.linked
    }) {
        return Ok(());
    }
    bail!(
        "selected checkout {} is no longer the same non-bare Git worktree; rerun grove and select it again",
        checkout.worktree.display()
    )
}

fn candidate_matches(path: &Path, info: &Probe) -> bool {
    info.top
        .as_ref()
        .is_some_and(|top| path == top || path == info.git_dir)
}

fn display_name(common: &Path) -> String {
    let base = common
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repository");
    if base == ".git" || base == ".bare" {
        common
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("repository")
            .to_owned()
    } else {
        base.strip_suffix(".git").unwrap_or(base).to_owned()
    }
}
#[derive(Default)]
struct Listed {
    path: PathBuf,
    branch: Option<String>,
    prunable: bool,
    bare: bool,
}
fn listed_worktrees(listing: &str) -> Vec<Listed> {
    let mut listed = Vec::<Listed>::new();
    for field in listing.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            listed.push(Listed {
                path: path.into(),
                ..Default::default()
            });
        } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
            if let Some(last) = listed.last_mut() {
                last.branch = Some(branch.into());
            }
        } else if field == "bare" {
            if let Some(last) = listed.last_mut() {
                last.bare = true;
            }
        } else if field == "detached" {
            if let Some(last) = listed.last_mut() {
                last.branch = None;
            }
        } else if (field == "prunable" || field.starts_with("prunable "))
            && let Some(last) = listed.last_mut()
        {
            last.prunable = true;
        }
    }
    listed
}

pub fn discover(roots: &[PathBuf], max_depth: usize) -> Result<Vec<Repository>> {
    let mut found = BTreeMap::new();
    for root in roots {
        if root.exists() {
            let mut walker = WalkDir::new(root)
                .follow_links(false)
                .max_depth(max_depth)
                .into_iter();
            while let Some(entry) = walker.next() {
                let entry =
                    entry.with_context(|| format!("scan search root {}", root.display()))?;
                if !descend(&entry) {
                    if entry.file_type().is_dir() {
                        walker.skip_current_dir();
                    }
                    continue;
                }
                if !entry.file_type().is_dir() {
                    continue;
                }
                let Some(anchor) = root_candidate(entry.path()) else {
                    continue;
                };
                let Some(repository) = inspect(anchor) else {
                    continue;
                };
                walker.skip_current_dir();
                match repository {
                    Repository::Bare(bare) => {
                        // Pointer containers and their explicitly configured .bare roots share identity.
                        let existing = found
                            .entry((true, bare.common.clone()))
                            .or_insert_with(|| Repository::Bare(bare.clone()));
                        if let Repository::Bare(previous) = existing
                            && bare.path < previous.path
                        {
                            *previous = bare;
                        }
                    }
                    Repository::Checkout(checkout) => {
                        found
                            .entry((false, checkout.worktree.clone()))
                            .or_insert(Repository::Checkout(checkout));
                    }
                }
            }
        }
    }
    let mut found: Vec<_> = found.into_values().collect();
    found.sort_by_key(|entry| entry.label(false));
    Ok(found)
}

/// Expand only the selected bare repository using Git's registry, not directory depth.
pub fn worktrees(bare: &BareRepository) -> Result<Vec<Checkout>> {
    let listing = git(&bare.common, &["worktree", "list", "--porcelain", "-z"])?;
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut cache = HashMap::new();
    for candidate in listed_worktrees(&listing) {
        if candidate.bare || candidate.prunable {
            continue;
        }
        let Ok(candidate_path) = canonical(&candidate.path) else {
            continue;
        };
        let Some(info) = probe(&candidate_path, &mut cache) else {
            continue;
        };
        if info.common != bare.common
            || info.git_dir == bare.common
            || !candidate_matches(&candidate_path, &info)
        {
            continue;
        }
        let Some(worktree) = info.top else { continue };
        if seen.insert(worktree.clone()) {
            found.push(Checkout {
                repo: bare.common.to_string_lossy().into_owned(),
                repo_name: display_name(&bare.common),
                worktree,
                branch: candidate.branch,
                linked: true,
            });
        }
    }
    found.sort_by_key(|checkout| checkout.label(false));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;
    fn run(dir: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fn commit(repo: &Path) {
        run(repo, &["config", "user.email", "a@b.c"]);
        run(repo, &["config", "user.name", "a"]);
        std::fs::write(repo.join("x"), "x").unwrap();
        run(repo, &["add", "x"]);
        run(repo, &["commit", "-m", "x"]);
    }
    fn discover_checkouts(roots: &[PathBuf], depth: usize) -> Result<Vec<Checkout>> {
        Ok(discover(roots, depth)?
            .into_iter()
            .map(|entry| match entry {
                Repository::Checkout(checkout) => checkout,
                Repository::Bare(_) => panic!("expected only directly discovered checkouts"),
            })
            .collect())
    }

    fn reset_counts() {
        GIT_INVOCATIONS.with(|count| count.set(0));
        WORKTREE_LISTS.with(|count| count.set(0));
    }

    fn counts() -> (usize, usize) {
        (
            GIT_INVOCATIONS.with(std::cell::Cell::get),
            WORKTREE_LISTS.with(std::cell::Cell::get),
        )
    }

    #[test]
    fn labels_distinguish_main_linked_and_font() {
        let main = Checkout {
            repo: "r".into(),
            worktree: "/tmp/repo".into(),
            repo_name: "repo".into(),
            branch: Some("main".into()),
            linked: false,
        };
        let linked = Checkout {
            worktree: "/tmp/wt".into(),
            branch: Some("feature".into()),
            linked: true,
            ..main.clone()
        };
        assert!(!main.label(true).contains("main"));
        assert!(linked.label(true).contains("/tmp/wt   feature"));
        assert!(linked.label(false).contains("/tmp/wt  branch: feature"));
        assert_eq!(main.columns(true), ["", "repo", "/tmp/repo", ""]);
        assert_eq!(
            linked.columns(true),
            ["", "repo/wt", "/tmp/wt", " feature"]
        );
        assert_eq!(
            linked.columns(false),
            ["", "repo/wt", "/tmp/wt", "branch: feature"]
        );
        let detached = Checkout {
            branch: None,
            ..linked
        };
        assert_eq!(detached.columns(false)[3], "branch: detached");
        let bare = Repository::Bare(BareRepository {
            common: "/tmp/repo.git".into(),
            path: "/tmp/repo.git".into(),
        });
        assert_eq!(
            bare.columns(true),
            ["\u{f418}", "repo", "/tmp/repo.git", ""]
        );
        assert_eq!(bare.columns(false), ["[bare]", "repo", "/tmp/repo.git", ""]);
    }
    #[test]
    fn normal_discovery_does_not_expand_external_worktrees() {
        let d = TempDir::new().unwrap();
        let root = d.path().join("root");
        let repo = root.join("repo name");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init"]);
        commit(&repo);
        let outside = TempDir::new().unwrap();
        let wt = outside.path().join("外 tree");
        run(
            &repo,
            &["worktree", "add", "-b", "feature", wt.to_str().unwrap()],
        );
        reset_counts();
        let all = discover_checkouts(&[root], 3).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worktree, repo.canonicalize().unwrap());
        assert_eq!(counts(), (0, 0));
    }
    #[test]
    fn checkout_contents_are_pruned_but_explicit_nested_roots_are_scanned() {
        let d = TempDir::new().unwrap();
        let repo = d.path().join("repo");
        let nested = repo.join("children/nested");
        std::fs::create_dir_all(&nested).unwrap();
        run(&repo, &["init"]);
        run(&nested, &["init"]);
        // A deep invalid marker would cost a failed probe if the scanner reached it.
        let deep = repo.join("build/cache/deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join(".git"), "gitdir: missing\n").unwrap();
        for root in [d.path(), repo.as_path()] {
            GIT_INVOCATIONS.with(|count| count.set(0));
            let all = discover_checkouts(&[root.to_path_buf()], 10).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].worktree, repo.canonicalize().unwrap());
            assert_eq!(GIT_INVOCATIONS.with(std::cell::Cell::get), 0);
        }
        let children = repo.join("children");
        for roots in [
            vec![repo.clone(), children.clone()],
            vec![children, repo.clone()],
        ] {
            let all = discover_checkouts(&roots, 1).unwrap();
            assert_eq!(all.len(), 2);
            assert!(
                all.iter()
                    .any(|c| c.worktree == nested.canonicalize().unwrap())
            );
        }
    }

    #[test]
    fn invalid_marker_does_not_hide_child_repository() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join(".git"), "gitdir: missing\n").unwrap();
        let child = d.path().join("child");
        std::fs::create_dir(&child).unwrap();
        run(&child, &["init"]);
        let all = discover_checkouts(&[d.path().to_path_buf()], 1).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worktree, child.canonicalize().unwrap());
    }

    #[test]
    fn bare_hubs_stop_discovery_and_only_selected_hub_expands() {
        for pointer in [false, true] {
            let d = TempDir::new().unwrap();
            let source = d.path().join("source");
            std::fs::create_dir(&source).unwrap();
            run(&source, &["init", "-b", "main"]);
            commit(&source);
            let hub = d.path().join(if pointer { "repo" } else { "repo.git" });
            std::fs::create_dir(&hub).unwrap();
            let bare = if pointer {
                hub.join(".bare")
            } else {
                hub.clone()
            };
            run(
                d.path(),
                &[
                    "clone",
                    "--bare",
                    source.to_str().unwrap(),
                    bare.to_str().unwrap(),
                ],
            );
            if pointer {
                std::fs::write(hub.join(".git"), "gitdir: .bare\n").unwrap();
            }
            let main = hub.join("main");
            let feature = hub.join("feature/x");
            let outside = d.path().join("outside/deep/linked");
            for (path, branch) in [
                (&main, "main-work"),
                (&feature, "feature/x"),
                (&outside, "outside"),
            ] {
                run(
                    &bare,
                    &["worktree", "add", "-b", branch, path.to_str().unwrap()],
                );
            }
            for checkout in [&main, &feature] {
                let nested = checkout.join("nested");
                std::fs::create_dir(&nested).unwrap();
                run(&nested, &["init"]);
            }
            // Neither independent children nor invalid markers inside a hub are scanned.
            let independent = hub.join("independent");
            std::fs::create_dir(&independent).unwrap();
            run(&independent, &["init"]);
            std::fs::write(hub.join("main/nested/.git/bad"), "ignored").unwrap();
            let unselected = d.path().join("unselected.git");
            run(d.path(), &["init", "--bare", unselected.to_str().unwrap()]);
            reset_counts();
            let all = discover(&[hub.clone(), bare.clone(), unselected], 10).unwrap();
            assert_eq!(all.len(), 2);
            // Initial discovery reads repository metadata directly.
            assert_eq!(counts(), (0, 0));
            let Repository::Bare(selected) = &all[0] else {
                panic!("bare entry")
            };
            assert_eq!(selected.common, bare.canonicalize().unwrap());
            assert_eq!(selected.path, hub.canonicalize().unwrap());
            assert_eq!(all[0].label(true), all[0].label(false));
            assert!(all[0].label(true).starts_with("repo [bare]  "));
            reset_counts();
            let expanded = worktrees(selected).unwrap();
            assert_eq!(expanded.len(), 3);
            assert_eq!(counts(), (10, 1));
            for path in [&main, &feature, &outside] {
                assert!(
                    expanded
                        .iter()
                        .any(|c| c.worktree == path.canonicalize().unwrap() && c.linked)
                );
            }
            assert!(
                expanded
                    .iter()
                    .any(|c| c.branch.as_deref() == Some("feature/x"))
            );
            let shallow = discover(std::slice::from_ref(&hub), 1).unwrap();
            assert_eq!(shallow, vec![all[0].clone()]);
            eprintln!("bare pointer={pointer}: initial 0 Git / 0 lists; selected 10 Git / 1 list");
        }
    }

    #[test]
    fn separate_git_dir_absolute_and_relative_gitfiles_are_normal() {
        for relative in [false, true] {
            let d = TempDir::new().unwrap();
            let root = d.path().join("root");
            let work = root.join("checkout");
            let metadata = root.join("metadata.git");
            std::fs::create_dir_all(&root).unwrap();
            assert!(
                Command::new("git")
                    .args(["init", "-b", "main", "--separate-git-dir"])
                    .arg(&metadata)
                    .arg(&work)
                    .status()
                    .unwrap()
                    .success()
            );
            commit(&work);
            if relative {
                std::fs::write(work.join(".git"), "gitdir: ../metadata.git\r\n\r\n").unwrap();
            }
            let nested = work.join("nested");
            std::fs::create_dir(&nested).unwrap();
            run(&nested, &["init"]);
            let all = discover_checkouts(std::slice::from_ref(&root), 3).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].worktree, work.canonicalize().unwrap());
            assert_eq!(
                all[0].repo,
                metadata.canonicalize().unwrap().to_string_lossy()
            );
            assert_eq!(all[0].branch.as_deref(), Some("main"));
            assert!(!all[0].linked);
        }
    }
    #[test]
    fn git_paths_with_embedded_newline_and_trailing_carriage_return_are_preserved() {
        let d = TempDir::new().unwrap();
        let source = d.path().join("source");
        std::fs::create_dir(&source).unwrap();
        run(&source, &["init", "-b", "main"]);
        commit(&source);
        let root = d.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let metadata = root.join("metadata\nline\r");
        assert!(
            Command::new("git")
                .args(["clone", "--bare"])
                .arg(&source)
                .arg(&metadata)
                .status()
                .unwrap()
                .success()
        );
        let work = root.join("checkout");
        run(
            &metadata,
            &["worktree", "add", work.to_str().unwrap(), "main"],
        );

        let entries = discover(std::slice::from_ref(&metadata), 3).unwrap();
        let Repository::Bare(bare) = &entries[0] else {
            panic!("bare entry")
        };
        let all = worktrees(bare).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worktree, work.canonicalize().unwrap());
        assert_eq!(
            all[0].repo,
            metadata.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(all[0].branch.as_deref(), Some("main"));
        assert!(all[0].linked);
    }

    #[test]
    fn stale_listed_subdirectory_cannot_normalize_to_parent_checkout() {
        let d = TempDir::new().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run(&repo, &["init", "-b", "main"]);
        commit(&repo);
        let linked = d.path().join("linked");
        run(
            &repo,
            &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
        );
        let admin = PathBuf::from(git(&linked, &["rev-parse", "--absolute-git-dir"]).unwrap());
        std::fs::remove_dir_all(&linked).unwrap();
        let stale = repo.join("stale-subdirectory");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(admin.join("gitdir"), format!("{}/.git\n", stale.display())).unwrap();

        let listing = git(&repo, &["worktree", "list", "--porcelain", "-z"]).unwrap();
        assert!(
            listed_worktrees(&listing)
                .iter()
                .any(|item| item.path == stale)
        );
        let stale = stale.canonicalize().unwrap();
        let info = probe(&stale, &mut HashMap::new()).unwrap();
        assert_eq!(
            info.top.as_deref(),
            Some(repo.canonicalize().unwrap().as_path())
        );
        assert!(!candidate_matches(&stale, &info));
        let all = discover_checkouts(std::slice::from_ref(&repo), 2).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worktree, repo.canonicalize().unwrap());
    }

    #[test]
    fn dot_bare_pointer_parent_and_bare_anchor_retain_branch() {
        let d = TempDir::new().unwrap();
        let project = d.path().join("pointer-project");
        let bare = project.join(".bare");
        std::fs::create_dir_all(&project).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--bare", "-b", "main"])
                .arg(&bare)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join(".git"), "gitdir: ./.bare\n").unwrap();
        run(&project, &["config", "core.bare", "false"]);
        run(&project, &["config", "core.worktree", ".."]);
        commit(&project);
        for roots in [
            vec![project.clone(), bare.clone()],
            vec![bare.clone(), project.clone()],
        ] {
            let all = discover_checkouts(&roots, 2).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].worktree, project.canonicalize().unwrap());
            assert_eq!(all[0].branch.as_deref(), Some("main"));
            assert!(!all[0].linked);
        }
    }
    #[test]
    fn linked_only_anchor_into_bare_repo_is_linked() {
        let d = TempDir::new().unwrap();
        let source = d.path().join("source");
        std::fs::create_dir(&source).unwrap();
        run(&source, &["init", "-b", "main"]);
        commit(&source);
        let bare = d.path().join("a.git");
        assert!(
            Command::new("git")
                .args(["clone", "--bare"])
                .arg(&source)
                .arg(&bare)
                .status()
                .unwrap()
                .success()
        );
        let linked = d.path().join("linked");
        run(
            &bare,
            &["worktree", "add", linked.to_str().unwrap(), "main"],
        );
        let all = discover_checkouts(std::slice::from_ref(&linked), 1).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worktree, linked.canonicalize().unwrap());
        assert!(all[0].linked);
    }
    #[test]
    fn prunable_replacement_is_separate_and_primary_branch_survives() {
        let d = TempDir::new().unwrap();
        let root = d.path().join("root");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-b", "main"]);
        commit(&repo);
        let replaced = root.join("linked");
        run(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                replaced.to_str().unwrap(),
            ],
        );
        std::fs::remove_dir_all(&replaced).unwrap();
        std::fs::create_dir(&replaced).unwrap();
        run(&replaced, &["init"]);
        let all = discover_checkouts(std::slice::from_ref(&root), 3).unwrap();
        let primary = all
            .iter()
            .find(|c| c.worktree == repo.canonicalize().unwrap())
            .unwrap();
        assert_eq!(primary.branch.as_deref(), Some("main"));
        let replacement = all
            .iter()
            .find(|c| c.worktree == replaced.canonicalize().unwrap())
            .unwrap();
        assert!(!replacement.linked);
        assert_ne!(replacement.repo, primary.repo);
    }
    #[test]
    fn selected_bare_filters_registry_records_and_deduplicates_checkouts() {
        let d = TempDir::new().unwrap();
        let source = d.path().join("source");
        std::fs::create_dir(&source).unwrap();
        run(&source, &["init", "-b", "main"]);
        commit(&source);
        let bare = d.path().join("repo.git");
        run(
            d.path(),
            &[
                "clone",
                "--bare",
                source.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let good = d.path().join("外 tree\nline");
        run(
            &bare,
            &["worktree", "add", "--detach", good.to_str().unwrap()],
        );
        let admin = PathBuf::from(git(&good, &["rev-parse", "--absolute-git-dir"]).unwrap());
        let duplicate = bare.join("worktrees/duplicate");
        std::fs::create_dir(&duplicate).unwrap();
        for file in ["HEAD", "commondir", "gitdir"] {
            std::fs::copy(admin.join(file), duplicate.join(file)).unwrap();
        }
        for kind in ["foreign", "missing", "prunable", "stale-subdirectory"] {
            let path = d.path().join(kind);
            run(
                &bare,
                &["worktree", "add", "-b", kind, path.to_str().unwrap()],
            );
            let admin = PathBuf::from(git(&path, &["rev-parse", "--absolute-git-dir"]).unwrap());
            if kind != "prunable" {
                run(&bare, &["worktree", "lock", path.to_str().unwrap()]);
            }
            if kind == "stale-subdirectory" {
                // Its parent is not otherwise listed: deduplication cannot hide acceptance.
                let stale = path.join("stale");
                std::fs::create_dir(&stale).unwrap();
                std::fs::write(admin.join("gitdir"), format!("{}/.git\n", stale.display()))
                    .unwrap();
            } else {
                std::fs::remove_dir_all(&path).unwrap();
                if kind == "foreign" {
                    std::fs::create_dir(&path).unwrap();
                    run(&path, &["init"]);
                }
            }
        }
        let listing =
            listed_worktrees(&git(&bare, &["worktree", "list", "--porcelain", "-z"]).unwrap());
        assert!(listing.iter().any(|record| record.bare));
        assert!(
            listing
                .iter()
                .any(|record| record.path.ends_with("prunable") && record.prunable)
        );
        assert!(
            listing
                .iter()
                .any(|record| record.path.ends_with("foreign") && !record.prunable)
        );
        assert_eq!(
            listing
                .iter()
                .filter(|record| record.path == good.canonicalize().unwrap())
                .count(),
            2
        );
        reset_counts();
        let entries = discover(&[bare.clone(), bare.clone()], 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(counts(), (0, 0));
        let Repository::Bare(selected) = &entries[0] else {
            panic!("bare entry")
        };
        let expanded = worktrees(selected).unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].worktree, good.canonicalize().unwrap());
        assert_eq!(
            expanded[0].repo,
            bare.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(expanded[0].branch, None);
        assert!(expanded[0].label(false).ends_with("branch: detached"));
    }

    #[test]
    fn initial_discovery_starts_no_git_processes() {
        let d = TempDir::new().unwrap();
        let root = d.path().join("root");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init"]);
        commit(&repo);
        for index in 0..8 {
            let wt = root.join(format!("worktree-{index}"));
            run(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-b",
                    &format!("branch-{index}"),
                    wt.to_str().unwrap(),
                ],
            );
        }
        let ordinary = d.path().join("ordinary");
        std::fs::create_dir(&ordinary).unwrap();
        run(&ordinary, &["init"]);
        reset_counts();
        let one = discover_checkouts(std::slice::from_ref(&ordinary), 1).unwrap();
        let ordinary_count = GIT_INVOCATIONS.with(std::cell::Cell::get);
        assert_eq!(one.len(), 1);
        assert_eq!(ordinary_count, 0);
        assert_eq!(counts().1, 0);

        for (scan_root, expected_checkouts) in [(&repo, 1), (&root, 9)] {
            reset_counts();
            let shared = discover_checkouts(std::slice::from_ref(scan_root), 1).unwrap();
            let shared_count = GIT_INVOCATIONS.with(std::cell::Cell::get);
            assert_eq!(shared.len(), expected_checkouts);
            assert_eq!(shared_count, 0);
            assert_eq!(counts().1, 0);
            eprintln!(
                "normal discovery: {} checkouts, {shared_count} Git / 0 lists",
                shared.len()
            );
        }
    }
}
