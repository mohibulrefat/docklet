# Docklet

A lightweight native GTK4 GUI for Docker Engine.

Docklet is a thin layer over the Docker Engine API — not a Docker Desktop
clone. It talks directly to `dockerd` over its unix socket with a small,
hand-rolled HTTP client; it does not shell out to the `docker` CLI, run a
background service, or keep a database. Docker Engine is the source of
truth for everything Docklet shows.

## Features

- **Containers** — list, start, stop, restart, remove; details, inspect,
  logs (one-shot and live follow); live CPU/memory/network stats for the
  container you're looking at
- **Images** — list, pull (with progress), remove, details
- **Volumes** — list, create, remove, details (including which containers
  use one)
- **Networks** — list, create, remove, details (including connected
  containers); Docker's own `bridge`/`host`/`none` networks can't be removed
- **Compose** — projects are detected from containers' own Compose labels;
  start/stop existing containers and view per-service logs. This is not a
  `compose up` replacement — it only ever acts on containers that already
  exist.

## Requirements

- Docker Engine, reachable over its unix socket (`DOCKER_HOST`, an active
  Docker context, or `/var/run/docker.sock`, tried in that order — the same
  resolution the `docker` CLI uses)
- GTK 4.10 or newer
- Linux (the transport is unix-socket only for now)

## Building

```sh
cargo build --release
```

The release profile is tuned for a small binary (`opt-level = "s"`, LTO,
stripped symbols) rather than raw execution speed — appropriate for a
utility whose work is mostly waiting on Docker's API, not computing.

## Running

```sh
cargo run --release
# or, after building:
./target/release/docklet
```

By default Docklet renders with GTK4's software (cairo) renderer rather
than its GPU-accelerated one, which measurably reduces idle memory use for
an app with no animations or GPU-bound rendering to benefit from. Set
`GSK_RENDERER` yourself (e.g. `GSK_RENDERER=gl`) to override this.

### Desktop integration

`data/dev.docklet.Docklet.desktop` and
`data/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg` are provided for
installing Docklet as a regular desktop application, e.g.:

```sh
install -Dm644 data/dev.docklet.Docklet.desktop \
  ~/.local/share/applications/dev.docklet.Docklet.desktop
install -Dm644 data/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg \
  ~/.local/share/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg
install -Dm755 target/release/docklet ~/.local/bin/docklet
```

## Testing

```sh
cargo test
```

Tests cover Docker API parsing, the request/response framing of the
hand-rolled HTTP client (including a real unix-socket round trip), and the
UI's list-diffing logic. None require a running Docker daemon.

## Project structure

```
src/
├── main.rs
├── docker/    Docker Engine API client — no GTK dependency
└── ui/        GTK4 widgets — no direct HTTP/socket access
```

`docker/` never imports GTK; `ui/` never touches sockets, `DOCKER_HOST`, or
Docker contexts directly — it calls typed functions and renders what comes
back. Endpoint resolution lives entirely in `docker/endpoint.rs`.

## License

MIT
