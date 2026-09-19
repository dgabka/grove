# grove

`grove` discovers Git checkouts, lets you select one with `fzf`, and opens or reuses an independent tmux session. It does not create, remove, or modify branches or worktrees.

## Install

Nix users can run the self-contained package; it wraps Grove with `git`, `tmux`, and `fzf` on `PATH`:

```sh
nix run github:dgabka/grove
```

Or install from source with a current Rust toolchain. The Cargo installation requires `git`, `tmux`, and `fzf` on `PATH`:

```sh
cargo install --path .
```

Nix flake consumers can use `grove.packages.${pkgs.system}.default` directly.

## Quick start

Create `$XDG_CONFIG_HOME/grove/config.toml` (or `~/.config/grove/config.toml`) with one absolute repository root:

```toml
roots = ["/home/you/repos"]
```

Replace the example path with your repository directory, then run:

```sh
grove
```

See [`example-config.toml`](example-config.toml) for the full configuration. Set `GROVE_CONFIG` to use another config file:

```sh
GROVE_CONFIG=./config.toml grove
```

Use `grove --help` for commands and `grove --version` for the installed version.

## Commands

- `grove` — select a checkout directly, or select a bare repository and then one of its active linked worktrees. Reuse its Grove session, or select a layout and create one.
- `grove switch [--repo]` — select another running tmux session. `--repo` prefers Grove sessions with the current session's repository metadata, then falls back to all other sessions.
- `grove close [--repo]` — requires tmux. Select another session, switch to it, then remove the old session only after navigation succeeds. `--repo` uses the same preference as `switch`; cancellation, no alternatives, or failed navigation preserves the old session.
- `grove refresh [--force]` — create missing configured default sessions on demand; it does not supervise them. **`--force` destructively kills every same-named session, including current or unrelated sessions, before replacement. A failed replacement cannot restore the old session.**

Cancelling a picker exits successfully without changing sessions.

## Configured default sessions

Use `[[defaults]]` to declare a named session with an absolute `cwd` that already exists. Its optional `windows` and `panes` use the same shape as presets. Omitting `windows` creates one shell window; omitting `panes` creates a shell in that window. Pane `command` values are argv arrays, so each configured element is passed literally to tmux rather than joined into a shell command.

```toml
[[defaults]]
name = "main"
cwd = "/Users/you"

[[defaults.windows]]
name = "editor"
[[defaults.windows.panes]]
command = ["nvim"]

[[defaults.windows]]
name = "shell"
```

`grove refresh` creates missing defaults in configuration order. If a session with the exact configured name already exists, Grove skips it regardless of its origin, current directory, or layout; it performs no reconciliation. `grove refresh --force` instead kills and replaces every same-named session, including foreign sessions and the current session. This is destructive: if replacement fails after the kill, Grove cannot restore the old session.

Cancelling either repository/worktree picker or the layout picker is a no-op. Bare entries show a branch/tree Nerd Font glyph (``, U+F418), or `[bare]` when `nerd_fonts = false`, in a dedicated first column; checkout and session rows leave that column blank so names align. Bare entries show a path, never a branch. A bare repository is never a session checkout; if it has no eligible worktrees, Grove prints a message and exits without a layout picker or session creation. Main checkout labels show repository and path without a branch; linked worktrees show `repository/worktree`, path, and their branch (`nerd_fonts = true`, the default, uses a Nerd Font branch glyph; `false` uses `branch:`). Repository, worktree, and structured session picker paths show `~` for `$HOME` and `~/relative/path` beneath it (including a symlinked home's canonical location); paths outside home or with unset/empty `$HOME` stay unchanged. This is display-only: checkout identity, session metadata, and working directories retain their full paths. Picker name, path, and branch columns align by terminal display width after shortening; control characters are escaped for display only. Grove stores the display name, plain label, and branch separately from repository and checkout identity, so both pickers render the current font preference. Older label-only and foreign sessions remain readable as single-column entries. Long rows may exceed narrow terminals, and glyph widths depend on the terminal font. `grove switch` reads this presentation setting when the config exists and still works with no config file.

Main checkout sessions are named `repository`; linked worktree sessions are named `repository/worktree`. Unicode, spaces, hyphens, and underscores are preserved; tmux-invalid colons, periods, and control characters are replaced with `-`. Names stay stable across branch changes. A canonical-path hash suffix is added only when that base name is already occupied.

Initial discovery reads `.git`, `HEAD`, `commondir`, and `config` files directly and starts no Git processes. It stops at detected checkouts and bare repositories, without scanning their contents or listing worktrees. Normal checkout selection never enumerates linked worktrees or opens a second worktree picker. This fast path supports ordinary checkouts, linked worktrees, separate Git directories, direct bare containers, and `.bare` plus `.git` pointer containers; unusual hand-written Git layouts may be skipped or misclassified. A malformed `.git` pointer does not stop traversal. Explicit roots below a checkout or bare container are still scanned independently with their own depth limit.

Only the selected bare repository gets one Git worktree porcelain listing. Bare, missing, and prunable records are excluded; remaining checkouts must be accessible and Git-confirmed members of that repository. Git-reported common directories, absolute Git directories, and top-level paths remain authoritative for ownership and normal/linked classification, including separate-git-dir layouts. Canonical identities deduplicate entries, and probes are cached during selected-repository expansion.

Deterministic process-count regressions require zero Git calls during initial discovery. Expanding a selected bare repository with three active linked checkouts costs 10 Git calls, including exactly one worktree listing. These are process counts, not latency benchmarks.

## tmux popup bindings

```tmux
bind g display-popup -E -w 60% -h 80% 'grove'
bind G display-popup -E -w 60% -h 80% 'grove switch'
```

The popup command is a tmux configuration example; Grove itself invokes Git, tmux, and fzf with process argument arrays.

## Releases

The crate version is Grove's version (`grove --version`). Create a version commit and matching tag from a clean working tree, then push them:

```sh
scripts/release 0.2.0
git push origin HEAD v0.2.0
```

The script updates `Cargo.toml` and `Cargo.lock`, runs the tests, commits, and tags. The release workflow verifies the version and publishes Linux and macOS archives with checksums to a GitHub release.

## Tests

`cargo test` uses temporary Git repositories. The tmux tests use unique `-L` server sockets with isolated temporary configuration files and kill only those sockets. Discovery covers checkout/bare-content pruning, independent nested roots, invalid markers, direct-bare/`.bare` hubs, ordinary and separate-git-dir repositories, linked-only anchors, and stale worktree records. Picker protocol tests use fake `fzf` executables. CLI flow tests use fake `fzf` and tmux executables to check both selection stages, cancellation, empty bare repositories, and metadata reuse without accessing a real tmux server; isolated real tmux tests also run when tmux is installed.
