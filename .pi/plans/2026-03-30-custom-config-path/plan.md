# Custom Grove Config Path

**Date:** 2026-03-30
**Status:** Ready
**Directory:** `/Users/dgabka/repos/grove.git/main`

## Intent
Allow users to select a Grove config file with `GROVE_CONFIG`, mainly so they can test configuration changes without modifying a Nix-managed primary config. Keep configuration-path resolution centralized so all commands, optional presentation settings, and diagnostics remain consistent.

## User Story
As a Grove user, I want to point Grove at an alternate config file, so that I can test config changes without replacing my managed config.

## Behavior

### Happy Path
1. The user sets `GROVE_CONFIG` to a non-empty file path.
2. Grove uses that exact path instead of its standard XDG/HOME config location.
3. Existing parsing, validation, and command behavior operate on the selected file.

### Edge Cases & Error Handling
- Unset `GROVE_CONFIG`: preserve existing XDG/HOME/current-directory fallback behavior.
- Empty `GROVE_CONFIG`: treat it as unset.
- Relative override path: use it unchanged, relative to the process working directory.
- Path containing spaces, Unicode, or non-UTF-8 bytes: preserve it through `var_os` and `PathBuf`.
- Missing override file: `grove` and `refresh` report the selected path; `switch` and `close` preserve their optional-config behavior.
- Malformed override file: fail without trying the standard config, and identify the selected path.

## Scope

### In Scope
- Add `GROVE_CONFIG` precedence to central config-path resolution.
- Cover precedence with a child-process CLI integration test.
- Document usage, precedence, and empty-value behavior.

### Out of Scope
- A config-path CLI option.
- Config merging or layered files.
- Falling back after a selected override fails to read or parse.
- File watching or runtime config reloads.

## Effort & Quality
- **Level:** Production-small
- **Tests:** Focused integration regression plus the existing suite and static checks
- **Docs:** README update

## Constraints
- Preserve all existing standard config-location precedence.
- Use `std::env::var_os`; do not require UTF-8 environment values.
- Tests must set environment variables on child processes, not mutate process-global environment.
- Add no dependency or new abstraction.

## Ideal State Criteria

### Core Functionality
- [ ] ISC-1: Non-empty `GROVE_CONFIG` selects the supplied config file.
- [ ] ISC-2: `GROVE_CONFIG` takes precedence over standard config locations.
- [ ] ISC-3: Empty `GROVE_CONFIG` uses existing config-path fallback behavior.
- [ ] ISC-4: Override paths preserve non-UTF-8-capable OS path handling.
- [ ] ISC-5: Config-related errors display the effective overridden path.

### Coverage and Documentation
- [ ] ISC-6: A child-process integration test verifies override precedence.
- [ ] ISC-7: README documents usage, precedence, and empty-value behavior.
- [ ] ISC-8: Existing tests and static checks continue to pass.

### Anti-Criteria
- [ ] ISC-A-1: No config CLI option is introduced.
- [ ] ISC-A-2: No config merging or fallback-after-read-failure is introduced.

## Approach
Update `config_path()` to return a non-empty `GROVE_CONFIG` value before evaluating its current fallback chain. Reuse every existing consumer unchanged, verify the behavior through an isolated CLI child process, and add concise README documentation.

### Key Decisions
- Centralize the override in `config_path()` because all config reads and diagnostics already use it.
- Treat an empty override as unset to preserve useful fallback behavior.
- Use the supplied non-empty path exactly, allowing relative paths without extra normalization.
- Test through a child command to avoid unsafe or flaky process-global environment mutation.
- Do not fall back when a selected override is missing or invalid, because silent fallback would conceal configuration mistakes.

### Architecture
`src/config.rs::config_path()` remains the sole location authority. It checks `GROVE_CONFIG`, filters empty values, converts a selected value directly to `PathBuf`, and otherwise executes the existing `XDG_CONFIG_HOME` → `HOME/.config` → local path logic. `load()`, `optional_nerd_fonts()`, and callers in `src/lib.rs` require no changes.

### Data Flow
1. An existing command requests configuration or optional presentation settings.
2. `config_path()` selects the non-empty override or the existing fallback path.
3. Existing filesystem reading, TOML parsing, and validation run unchanged.
4. Existing error contexts display the effective selected path.

## Dependencies
- Rust standard library only; no new crate or external service.

## Risks & Open Questions
- Risk: a consumer bypasses central resolution. Mitigation: confirmed current `load()`, optional presentation loading, and diagnostics use `config_path()`.
- Risk: integration tests become flaky through global environment mutation. Mitigation: configure only the spawned child process.
- Risk: a weak test proves loading but not precedence. Mitigation: make standard and override configs observably conflict.
- Risk: users expect fallback after an invalid override. Accepted and documented: a non-empty override is authoritative.
- Open questions: none.

## Implementation Todos

- [x] **TODO 1 — Add central `GROVE_CONFIG` resolution** `[custom-config-path]`
  - **Target:** `src/config.rs`
  - **Intent:** Make `config_path()` return a non-empty `GROVE_CONFIG` value before the existing XDG/HOME/local fallback chain. Preserve OS-native path values and leave all consumers unchanged.
  - **Key details:** Treat an empty `OsString` as unset; return every non-empty value exactly as supplied; do not fall back when the selected file is missing or invalid.
  - **ISC:** ISC-1, ISC-2, ISC-3, ISC-4, ISC-5, ISC-A-1, ISC-A-2.
  - **Dependencies:** None.
  - **Verification:** Run the focused integration test from TODO 2 and `cargo test`.
  - **Code example:**
    ```rust
    if let Some(path) = std::env::var_os("GROVE_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    ```

- [x] **TODO 2 — Cover override precedence, diagnostics, and empty fallback** `[custom-config-path]`
  - **Target:** `tests/two_stage.rs`
  - **Intent:** Add one child-process CLI regression test proving that a valid alternate config overrides an invalid standard config and that an empty override returns to the standard location.
  - **Key details:** Use `Command::env` only; never mutate the test process environment. Invoke Grove with no subcommand and an empty-roots config so it exits before calling Git, tmux, or fzf. Assert stderr names the effective config path and does not report the bypassed config's parse error.
  - **ISC:** ISC-2, ISC-3, ISC-5, ISC-6.
  - **Dependencies:** TODO 1.
  - **Verification:** `cargo test --test two_stage custom_config_path_overrides_standard_and_empty_value_falls_back`.
  - **Code example:**
    ```rust
    let output = Command::new(env!("CARGO_BIN_EXE_grove"))
        .current_dir(fixture.dir.path())
        .env("HOME", fixture.dir.path())
        .env("XDG_CONFIG_HOME", fixture.dir.path().join("config"))
        .env("GROVE_CONFIG", &alternate)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains(&alternate.display().to_string()));
    ```

- [x] **TODO 3 — Document the override and run release checks** `[custom-config-path]`
  - **Target:** `README.md`
  - **Intent:** Document `GROVE_CONFIG` beside the existing config-location instructions, including precedence, exact relative-path behavior, empty-value fallback, and authoritative failure behavior.
  - **Key details:** Keep the explanation concise and add no CLI option or configuration-merging promise.
  - **ISC:** ISC-7, ISC-8, ISC-A-1, ISC-A-2.
  - **Dependencies:** TODO 1 and TODO 2.
  - **Verification:** Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`.
  - **Code example:**
    ```sh
    GROVE_CONFIG=./config-under-test.toml grove
    ```

- [x] **Review fix P1 — Isolate shared integration child commands from `GROVE_CONFIG`** `[custom-config-path]`
  - **Target:** `tests/two_stage.rs`
  - **Intent:** Ensure `run_mutating` and `run_session` remove any inherited `GROVE_CONFIG`, while TODO 2 continues to set its override explicitly on its direct child commands.
  - **Verification:** `cargo test --test two_stage custom_config_path_overrides_standard_and_empty_value_falls_back`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`.
