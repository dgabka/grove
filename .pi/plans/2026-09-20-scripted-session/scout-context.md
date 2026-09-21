# Context for: non-interactive session creation (`grove --path ... --preset ...`)

## Relevant Files
- `src/main.rs` — Clap `Cli` currently has an optional subcommand; bare `grove` loads config and calls `open`. Existing subcommands are `switch`, `close`, and `refresh`. Unit tests use `Cli::try_parse_from`.
- `src/lib.rs` — `open` performs discovery, repository/worktree pickers, checkout validation, metadata-based session reuse, layout picker, collision-safe naming, tmux creation, and navigation. `select_layout` offers shell as id `0`, then configured presets. `session_name` is public and implements normal naming/collision rules.
- `src/git.rs` — `Checkout`/`Repository` types and Git probing. `validate_checkout` re-probes the selected path and verifies non-bare status, canonical top-level path, common repo identity, and linked-worktree status. `discover` intentionally does not enumerate worktrees initially; `worktrees` is only for selected bare repositories.
- `src/config.rs` — TOML config, `Preset`, `Preset::shell()`, strict validation and unique preset names. `load()` is authoritative and errors on missing/invalid config.
- `src/tmux.rs` — `Tmux::create(name, checkout, preset)` creates layouts with argv-safe commands and stores `@grove_repo`/`@grove_worktree` plus display metadata. `navigate` attaches/switches. `create_layout` rolls back a partially-created session on failure.
- `tests/two_stage.rs` — integration fixture with temporary Git repositories, linked worktrees, fake picker, isolated fake tmux, and assertions for picker cancellation, validation races, naming, metadata reuse, and tmux command ordering.
- `README.md`, `example-config.toml` — behavior/config contract; explicitly says linked worktrees are supported, names are stable/collision-safe, and picker cancellation is a no-op.

## Project Structure
Small synchronous Rust CLI. `main.rs` owns CLI dispatch; `lib.rs` owns orchestration and picker behavior; `git.rs` owns discovery/identity validation; `tmux.rs` owns subprocess argument construction and isolated tmux tests; config is deserialized from one TOML file.

## Conventions
- Use `anyhow::{Context, Result, bail}` for errors and literal subprocess argument arrays/vectors.
- Canonicalize paths and preserve spaces/Unicode; do not infer linked worktrees from path shape.
- Validate a checkout immediately before session reuse/creation. `validate_checkout` is deliberately independent of stale discovery metadata.
- Reuse existing `session_for_worktree` and `session_name`; session identity is canonical checkout metadata, not session name.
- Existing open flow: validate → list sessions → metadata reuse (no layout picker) → select layout → validate again → compute occupied names → create → navigate.
- tmux tests must use isolated server/socket/config; integration fakes log argv and explicitly prevent normal-server access.

## Key Findings
- Lowest-impact path is to add optional top-level `--path` and `--preset` fields to `Cli` (rather than inventing a subcommand), then dispatch a new library function when either scripted option is present. This matches the requested invocation exactly while retaining `grove` and existing subcommands.
- Scripted mode should bypass repository and layout pickers. It can construct a `Checkout` for the supplied path using existing Git probing/discovery logic, validate it, resolve a named configured preset, then run the same sessions → metadata reuse → naming → `Tmux::create` → `navigate` sequence used by `open`.
- The safest identity implementation is to expose/reuse a Git helper that inspects the supplied path with Git (`rev-parse --show-toplevel`, `--absolute-git-dir`, `--git-common-dir`, bare status), canonicalizes outputs, and constructs `Checkout`; do not use path inference. `validate_checkout` already enforces the final canonical identity and linked flag.
- A supplied checkout can be a normal checkout or linked worktree. Bare repositories must be rejected because the requirement says the selected path is a Git checkout; `validate_checkout` rejects bare paths.
- Preset lookup should use exact configured `Preset.name`; unknown names should return a clear error. `--preset` omitted means the existing layout picker (per task), while `--path` omitted retains normal discovery picker. The exact behavior when only one of the two flags is supplied is ambiguous: likely path selects/validates non-interactively and omitted preset invokes only layout picker; preset without path likely remains discovery picker then should apply the requested preset (or be rejected as not fully scripted).
- Config is still needed for presets, but `config.roots` should not gate scripted operation: `open` currently errors when roots are empty, whereas an explicit path should not require discovery roots. `config::load()` still validates the config and supplies presets.
- Reuse behavior should remain unchanged: if tmux metadata has the canonical checkout path, navigate existing session without requiring/resolving a preset. This also preserves independent sessions.
- Session names should be generated from the Git-derived `Checkout` with `session_name`, including sanitization and stable hash suffix on collisions; do not derive a name directly from the CLI path.

## Existing Tests / Suggested Coverage
- Add CLI parser assertions in `src/main.rs` for `grove --path /x --preset dev`, and likely reject options mixed with subcommands if Clap shape permits.
- Add a focused library/test helper for exact preset resolution and explicit-path checkout construction.
- Extend `tests/two_stage.rs` with temporary normal and linked checkouts, configured `dev` preset, no picker invocation (when both options supplied), tmux `new-session` cwd/name assertions, unknown preset failure, non-Git/bare rejection, metadata reuse, and collision naming. Use the existing fake tmux/picker fixture; never normal tmux.

## Ambiguities / Gotchas
- Clap currently models options only inside subcommands; adding top-level flags changes parsing shape and must avoid accidentally allowing `grove switch --path ...`.
- User says “`--path /absolute/git/checkout --preset dev`” and “picker only for omitted preset”; this strongly implies `--path` is explicit and `--preset` controls only layout selection, but whether `--path` alone still allows discovery/layout selection needs an explicit choice.
- “absolute” should be enforced at CLI boundary (or clearly canonicalized); relative paths should produce a direct error rather than silently resolve against cwd.
- Do not enumerate all worktrees for an explicit path. Git probing the supplied path is enough and supports linked worktrees without violating initial-discovery constraints.
- Keep picker cancellation as success/no-op when preset is omitted. Do not create a session before layout selection.
