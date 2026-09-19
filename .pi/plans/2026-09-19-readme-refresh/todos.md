# Todos: Grove README Refresh

All todos are tagged `readme-refresh` and refer to `.pi/plans/2026-09-19-readme-refresh/plan.md`.

## readme-refresh-1 — Rewrite overview, installation, and quick start

**Status:** Done
**Tag:** `readme-refresh`

Rewrite the start of `README.md` through the first-run instructions. State Grove's purpose and non-goals, distinguish the self-contained Nix package from Cargo installation prerequisites, link the full example config, and show the smallest usable config.

**Files:** `README.md`

**References:**
- `README.md:1-27` — current overview and installation text to replace.
- `flake.nix:25-35` — Nix package wraps Grove with `fzf`, `git`, and `tmux` on `PATH`.
- `src/config.rs:77-98` — config path selection and required config loading.

**Expected example shape:**

```toml
roots = ["/home/you/repos"]
```

Use a clearly illustrative absolute path. Mention `grove --help` and `grove --version`. Do not copy the full example config or add another documentation file.

**Acceptance:** ISC-1, ISC-2, ISC-3, ISC-A-2, ISC-A-3.

## readme-refresh-2 — Replace the command reference

**Status:** Done
**Tag:** `readme-refresh`

Write short descriptions for `grove`, `grove switch [--repo]`, `grove close [--repo]`, and `grove refresh [--force]`. Keep the successful no-op behavior for picker cancellation. State that `close` requires tmux, switches first, and preserves the old session when navigation does not succeed. Put the destructive `refresh --force` warning beside that command.

**Files:** `README.md`

**References:**
- `src/main.rs:11-33` — complete clap command surface.
- `src/lib.rs:229-375` — refresh, open, switch, and close flows.
- `README.md:29-34` — current command descriptions.

Do not invent aliases, background supervision, or worktree-management behavior.

**Acceptance:** ISC-4, ISC-8, ISC-9, ISC-A-2.

## readme-refresh-3 — Write the concise configuration reference

**Status:** Done
**Tag:** `readme-refresh`

Explain config lookup, `GROVE_CONFIG`, `roots`, `max_depth`, `nerd_fonts`, presets, defaults, windows, panes, and strict validation. State the defaults (`max_depth = 3`, `nerd_fonts = true`), absolute-path requirements, lack of `~` expansion, rejection of unknown keys, and default-session name restrictions. Explain that pane commands are argv arrays and example executables must be on `PATH`.

**Files:** `README.md`

**References:**
- `example-config.toml:1-34` — canonical complete configuration shape.
- `src/config.rs:5-74` — schema and defaults.
- `src/config.rs:77-117` — config lookup and optional presentation setting.
- `src/config.rs:119-169` — validation rules.

**Expected command example shape:**

```toml
[[presets.windows.panes]]
command = ["nvim", "-S"]
```

Do not duplicate every validation error or change `example-config.toml`.

**Acceptance:** ISC-5, ISC-A-3, ISC-A-4.

## readme-refresh-4 — Condense repository and session behavior

**Tag:** `readme-refresh`

Replace the implementation-heavy discovery and picker prose with concise user-visible behavior. Cover direct checkout selection, selected-bare-only worktree selection, empty bare repositories, metadata-based session reuse, stable naming, collision suffixes, spaces and Unicode, and optional Nerd Font presentation.

**Files:** `README.md`

**References:**
- `README.md:55-64` — current picker, naming, and discovery details to condense.
- `src/lib.rs:246-334` — open and two-stage selection flow.
- `src/git.rs` — Git-confirmed checkout discovery and selected repository expansion.
- `src/tmux.rs` — tmux metadata and session operations.
- `AGENTS.md:3-9` — behavior constraints that the simplified text must preserve.

Use this level of wording rather than internal algorithms: “Selecting a bare repository opens a second picker for its active linked worktrees.” Do not retain exact subprocess counts or promise support for unusual hand-written Git layouts.

**Acceptance:** ISC-6, ISC-7, ISC-8, ISC-A-1, ISC-A-4.

## readme-refresh-5 — Minimize integration and maintainer sections

**Tag:** `readme-refresh`

Retain the tmux popup examples, release procedure, and development checks in compact sections. Replace the exhaustive test inventory with the standard commands and one safety note: real tmux tests use dedicated isolated sockets and never the normal tmux server.

**Files:** `README.md`

**References:**
- `README.md:66-93` — current popup, release, and test sections.
- `scripts/release` — current release automation behavior.
- `AGENTS.md:11-19` — canonical development commands and tmux-test safety constraint.
- `tests/two_stage.rs:108-118` — isolated socket enforcement.

**Expected development block:**

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Do not add contributor tooling or a separate contributor guide.

**Acceptance:** ISC-10, ISC-11, ISC-A-2, ISC-A-3.

## readme-refresh-6 — Verify the finished README against the plan

**Tag:** `readme-refresh`

Read the complete rewritten `README.md` and check every item in the plan's Ideal State Criteria. Verify command names against clap, configuration claims against `src/config.rs`, packaging claims against `flake.nix`, and safety behavior against `src/lib.rs` plus existing tests. Correct documentation mistakes only.

**Files:** `README.md`

**References:**
- `.pi/plans/2026-09-19-readme-refresh/plan.md` — approved scope and full ISC checklist.
- `src/main.rs:4-33` — CLI source of truth.
- `src/config.rs:5-169` — configuration source of truth.
- `flake.nix:25-35` — packaged runtime tools.
- `tests/two_stage.rs` — user-flow regression coverage.

**Verification shape:**

```text
ISC-1 … ISC-11: pass
ISC-A-1 … ISC-A-4: pass
git diff -- README.md
```

Do not run behavior-changing commands, edit source files, or alter `AGENTS.md` or `example-config.toml`.

**Acceptance:** All ISC and anti-criteria pass.
