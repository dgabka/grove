# Scripted Session Creation

**Date:** 2026-09-20
**Status:** Draft
**Directory:** /Users/dgabka/repos/grove.git/main

## Intent

Add a minimal non-interactive entry path for opening or creating a Grove tmux session from an explicitly supplied Git checkout. Scripts may select a configured preset directly, while invocations without a preset retain the existing layout picker and all existing identity, naming, validation, and tmux behavior.

## User Story

As a Grove user writing shell scripts or shortcuts, I want to open a checkout with `grove --path <checkout> [--preset <name>]`, so that I can reuse or create the correct tmux session without repository discovery or, when named, any picker interaction.

## Behavior

### Happy Path

1. The user invokes `grove --path /absolute/checkout --preset dev`.
2. Clap validates that the path is absolute and that opening flags are not mixed with subcommands.
3. Grove loads and validates the normal configuration.
4. Git identifies the canonical non-bare checkout root and whether it is a linked worktree.
5. Grove revalidates checkout identity and reads tmux sessions.
6. If canonical worktree metadata matches an existing session, Grove navigates to it immediately.
7. Otherwise Grove resolves `dev` by exact configured preset name.
8. Grove revalidates the checkout, applies existing collision-safe naming, creates the session, stores normal metadata, and navigates.
9. If `--preset` is omitted, step 7 uses the existing layout picker; cancellation exits successfully without creation.

### Edge Cases & Error Handling

- `--preset` without `--path`: reject as a Clap usage error.
- Relative `--path`: reject as a Clap usage error.
- Opening flags combined with a subcommand: reject as a Clap usage error.
- Missing or non-Git path: return a contextual error without picker or tmux mutation.
- Checkout subdirectory: reject because the explicit path is not the Git top-level.
- Bare repository: reject; do not enumerate worktrees in explicit mode.
- Linked worktree: accept and derive identity from Git CLI output.
- Symlink to a checkout root: canonicalize and use the checkout's canonical identity.
- Unknown preset when creation is required: return a clear error naming the preset.
- Unknown preset when a session already exists: navigate to the existing session without resolving the preset.
- Checkout replaced while the layout picker is open: reject during the second validation before creation.
- Occupied preferred session name: preserve the unrelated session and use the existing stable suffix.
- Spaces and Unicode: preserve them through path arguments, metadata, and naming.

## Scope

### In Scope

- Top-level `--path` and optional `--preset` arguments.
- Explicit Git checkout probing without discovery-root restrictions.
- Exact configured-preset selection.
- Shared post-selection orchestration for interactive and explicit modes.
- Targeted CLI, Git, and integration regression tests.
- Concise README usage and behavior documentation.

### Out of Scope

- Preset-only operation with interactive repository selection.
- Worktree or branch creation/removal/management.
- Bare-repository worktree enumeration in explicit mode.
- Preset aliases, a special CLI name for the synthetic shell preset, or new configuration.
- Async behavior, daemonization, or extra infrastructure.
- Changes to tmux layout creation or metadata format.

## Effort & Quality

- **Level:** Production
- **Tests:** Thorough targeted parser, Git-probe, and isolated integration coverage
- **Docs:** README update

## Constraints

- Keep Grove synchronous and small; add no dependency.
- Load the authoritative config in explicit mode, but do not require the path to fall under `roots` or require `roots` to be nonempty.
- Derive checkout ownership and classification from Git CLI facts, never path inference.
- Preserve canonical checkout metadata reuse and independent tmux sessions.
- Invoke subprocesses with argument arrays; preserve spaces and Unicode.
- Retain validation both before reuse and immediately before creation.
- Any real tmux test must use a dedicated isolated socket; prefer the existing fake isolated harness for this change.

## Ideal State Criteria

### Core Functionality

- [ ] ISC-1: `--path` accepts an absolute normal Git checkout.
- [ ] ISC-2: `--path` accepts an absolute linked worktree.
- [ ] ISC-3: Named presets bypass the layout picker.
- [ ] ISC-4: Omitted presets open the existing layout picker.
- [ ] ISC-5: Existing canonical checkout sessions are reused.
- [ ] ISC-6: New sessions retain existing collision-safe naming.
- [ ] ISC-7: Picker cancellation creates no tmux session.

### Validation

- [ ] ISC-8: `--preset` without `--path` fails during argument parsing.
- [ ] ISC-9: Relative paths are rejected.
- [ ] ISC-10: Non-Git paths are rejected.
- [ ] ISC-11: Bare repositories are rejected.
- [ ] ISC-12: Unknown preset names produce a clear error.

### Anti-Criteria

- [ ] ISC-A-1: Plain `grove` retains its current interactive behavior.
- [ ] ISC-A-2: Explicit paths are not restricted to configured roots.
- [ ] ISC-A-3: No new runtime dependency is added.

## Approach

Extract the existing post-selection checkout-opening tail into one private helper. Interactive discovery continues to select a `Checkout` exactly as today, then calls that helper without a preset name. A new public explicit entry point obtains a canonical `Checkout` directly from Git and calls the same helper with the optional preset name.

### Key Decisions

- Use top-level flags rather than a new subcommand, matching `grove --path ... --preset ...`.
- Reject `--preset` without `--path`; do not alter interactive repository selection.
- Probe explicit paths directly; do not route them through configured-root discovery.
- Resolve a supplied preset by exact configured name only.
- Reuse matching sessions before preset lookup because layouts affect creation, not navigation.
- Keep both existing checkout validation points around picker/session creation races.
- Reuse the existing test fixture and tmux behavior rather than introduce new harnesses or abstractions.

### Architecture

#### `src/main.rs`

Add optional top-level path and preset fields to `Cli`. Use a small Clap value parser to require an absolute `PathBuf`; declare that preset requires path and opening arguments conflict with subcommands. Dispatch explicit mode to the new library entry point and leave all existing subcommands unchanged. Extend `Cli::try_parse_from` tests for accepted and rejected combinations.

#### `src/git.rs`

Add a focused public function that takes an absolute path, canonicalizes/probes it with existing Git helpers, verifies that it is a non-bare top-level checkout, and builds `Checkout` from the probe's common directory, Git directory, top-level, and branch. Reuse existing `display_name`, `branch`, and linked-worktree classification. Return contextual errors rather than exposing the private probe type.

#### `src/lib.rs`

Move the section beginning with checkout validation from `open` into a private helper accepting `&Checkout` and `Option<&str>`. The helper validates, checks metadata reuse, selects either an exact configured preset or the current layout picker, validates again, names, creates, and navigates. Add a public explicit-path orchestration function that calls the Git constructor and shared helper. The normal `open` retains only discovery and picker responsibilities before calling the helper.

#### `tests/two_stage.rs`

Extend the fixture runner so integration invocations can pass opening arguments while retaining the fake picker, fake tmux, isolated socket requirement, and mutation support. Add focused tests proving normal and linked checkout behavior, preset picker bypass, omitted-preset selection/cancellation, root independence, unknown-preset handling, reuse, collision naming, and invalid path classes. Avoid duplicating cases already proven by unit tests unless an end-to-end assertion is needed.

#### `README.md`

Document the command form, required absolute checkout-root semantics, linked-worktree support, exact configured preset lookup, picker fallback, and preset-without-path rejection. State that explicit paths need not be under configured roots.

### Data Flow

```text
CLI parse
  -> config load
  -> explicit Git checkout probe (path mode)
     OR discovery/repository/worktree picker (plain mode)
  -> shared checkout validation
  -> tmux sessions
  -> matching metadata? navigate and stop
  -> named preset lookup OR layout picker
  -> cancellation? stop successfully
  -> second checkout validation
  -> existing session_name logic
  -> existing tmux create
  -> navigate
```

## Dependencies

- Existing `clap`, `anyhow`, Git CLI, tmux, and fzf only.
- No Cargo dependency changes.

## Risks & Open Questions

- **Probe-to-Checkout fidelity:** Mitigated with Git unit tests for normal, linked, bare, subdirectory, and symlink inputs.
- **Clap arguments silently mixed with subcommands:** Mitigated with parser rejection tests.
- **Interactive regression from helper extraction:** Mitigated by leaving discovery unchanged and retaining existing integration tests.
- **Named mode accidentally invokes fzf:** Mitigated by an end-to-end test asserting zero picker invocations.
- **Explicit mode accidentally requires configured roots:** Mitigated by an empty/outside-root integration case.
- **Stale checkout after picker wait:** Mitigated by preserving the second validation and mutation regression coverage.
- **Invalid preset appears accepted during reuse:** Accepted and documented; existing-session navigation precedes creation-layout resolution.
- **Open questions:** None.
