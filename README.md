# grove

`grove` creates or navigates independent tmux sessions for Git worktrees. It uses Git for discovery and `fzf` for selection; it never changes repositories.

## Install

Requires a current Rust toolchain, `git`, `tmux`, and `fzf` on `PATH`.

```sh
cargo install --path .
```

Copy [`example-config.toml`](example-config.toml) to `$XDG_CONFIG_HOME/grove/config.toml` (or `~/.config/grove/config.toml`). Roots must be absolute paths; `~` is not expanded. `max_depth` limits directory scanning; selecting a bare repository uses Git's registry to find its linked worktrees even outside those roots or below that depth.

A pane `command` is an argv array; Grove passes its elements to tmux as separate process arguments instead of constructing a shell command string. Omitting panes leaves a shell window. New sessions first offer the built-in one-shell layout plus configured presets.

## Commands

- `grove` — select a normal checkout directly, or select a `[bare]` repository and then one of its active linked worktrees. Reuse the checkout's Grove session, or select a layout and create one.
- `grove switch` — select any running tmux session.
- `grove switch --repo` — select running Grove sessions for the current repository. It first uses current Grove-session metadata, then falls back to Git discovery from the current directory.

Cancelling either repository/worktree picker or the layout picker is a no-op. Bare entries show `repository [bare]` and full path, never a branch. A bare repository is never a session checkout; if it has no eligible worktrees, Grove prints a message and exits without a layout picker or session creation. Main checkout labels show repository and full path without a branch; linked worktrees show `repository/worktree`, full path, and their branch (`nerd_fonts = true`, the default, uses a Nerd Font branch glyph; `false` uses `branch:`). Grove stores the plain label and branch separately from repository and checkout identity, so both pickers render the current font preference. Older label-only sessions remain readable. `grove switch` reads this presentation setting when the config exists and still works with no config file.

Main checkout sessions are named `repository`; linked worktree sessions are named `repository/worktree`. Unicode, spaces, hyphens, and underscores are preserved; tmux-invalid colons, periods, and control characters are replaced with `-`. Names stay stable across branch changes. A canonical-path hash suffix is added only when that base name is already occupied.

Initial discovery stops at both Git-confirmed checkouts and bare repositories: it does not scan their contents or list any worktrees. Normal checkout selection never enumerates linked worktrees or opens a second worktree picker. This supports ordinary checkouts and direct bare containers such as `repo.git/main` and `repo.git/feature/x`, as well as `.bare` plus `.git` pointer containers when Git confirms they are bare. A `.git` pointer alone does not stop traversal. Explicit roots below a checkout or bare container are still scanned independently with their own depth limit.

Only the selected bare repository gets one Git worktree porcelain listing. Bare, missing, and prunable records are excluded; remaining checkouts must be accessible and Git-confirmed members of that repository. Git-reported common directories, absolute Git directories, and top-level paths remain authoritative for ownership and normal/linked classification, including separate-git-dir layouts. Canonical identities deduplicate entries and probes are cached within discovery or selected-repository expansion.

Deterministic process-count regressions measure 5 Git calls for one ordinary checkout, 9 for two direct bare roots, and zero worktree-list calls during either initial scan. Expanding a selected bare repository with three active linked checkouts costs 10 Git calls, including exactly one worktree listing. These are process counts, not latency benchmarks.

## tmux popup bindings

```tmux
bind g display-popup -E -w 80% -h 80% 'grove'
bind G display-popup -E -w 80% -h 80% 'grove switch'
bind r display-popup -E -w 80% -h 80% 'grove switch --repo'
```

The popup command is a tmux configuration example; Grove itself invokes Git, tmux, and fzf with process argument arrays.

## Tests

`cargo test` uses temporary Git repositories. The tmux tests use unique `-L` server sockets with isolated temporary configuration files and kill only those sockets. Discovery covers checkout/bare-content pruning, independent nested roots, invalid markers, direct-bare/`.bare` hubs, ordinary and separate-git-dir repositories, linked-only anchors, and stale worktree records. Picker protocol tests use fake `fzf` executables. CLI flow tests use fake `fzf` and tmux executables to check both selection stages, cancellation, empty bare repositories, and metadata reuse without accessing a real tmux server; isolated real tmux tests also run when tmux is installed.
