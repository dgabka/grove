# Project guidance

- Keep Grove a small synchronous Rust CLI for discovery, selection, and tmux navigation. No worktree/branch management or extra infrastructure.
- Read `README.md` and `example-config.toml` before changing behavior.
- Initial discovery stops at Git-confirmed normal checkouts and bare repositories; never enumerate worktrees during that scan.
- Enumerate worktrees only for the selected bare repository, using Git CLI output for ownership and classification, never path inference.
- Use canonical checkout identity and tmux metadata, not session names, to identify checkouts. Preserve independent tmux sessions.
- Invoke subprocesses with argument arrays. Preserve spaces and Unicode; picker cancellation must remain a no-op.
- Cover regressions with temporary Git repositories. Real tmux tests must use a dedicated isolated socket and configuration, NEVER the normal tmux server; clean up only their own server.

## Development checks

Optionally enter the existing development shell with `nix develop`.

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
