# Contributing to Docklet

Thanks for considering a contribution. Docklet is a small, focused
project, so the bar for contributions is mostly about fitting its
existing design rather than process.

## Before you start

Docklet is deliberately a thin layer over the Docker Engine API:

- No shelling out to the `docker` CLI.
- No background service, no database, no cached state beyond what a
  refresh reads from Docker.
- `docker/` never imports GTK; `ui/` never touches sockets or Docker
  contexts directly. See the "Project structure" section in
  [README.md](README.md) for the full explanation.

If a change would cross one of those boundaries (for example, shelling
out to `docker compose`, or adding a persistence layer), open an issue to
discuss it first. Small bug fixes, UI polish, and new read-only views over
existing Docker API data are generally welcome without prior discussion.

## Setting up

```sh
git clone https://github.com/mohibulrefat/docklet.git
cd docklet
cargo build
```

You need a Rust toolchain and GTK 4.10 development headers. A running
Docker daemon is only needed to actually use the app, not to build or run
its test suite.

## Making a change

1. Branch from `development`.
2. Keep changes focused. A bug fix should not carry unrelated refactors.
3. Add or update tests for the behavior you changed. Most of the codebase
   is testable without a running Docker daemon: pure parsing and
   diffing logic is unit tested, and the hand-rolled HTTP client is
   tested against a real unix socket in-process.
4. Before opening a pull request, run:

   ```sh
   cargo fmt
   cargo test
   cargo clippy --all-targets --all-features -- -D warnings
   ```

5. Open a pull request against `development`. Describe what changed and
   why, not just what the diff shows.

## Code style

- Match the existing style in the file you're editing. `cargo fmt` handles
  formatting; there is no separate style guide beyond that.
- Comments explain the non-obvious "why," not the "what." Well-named
  functions and variables should make the "what" clear on their own.
- Avoid adding dependencies, background services, or new persistence
  unless the change genuinely requires them.

## Reporting issues

Open a GitHub issue with:

- What you expected to happen and what happened instead.
- Your Docker Engine version (`docker version`) and how Docklet reaches
  it (`DOCKER_HOST`, a Docker context, or the default socket).
- Steps to reproduce, if you have them.

## License

By contributing, you agree that your contributions are licensed under the
project's MIT license.
