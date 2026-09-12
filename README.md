# Docklet

A lightweight native GTK4 GUI for Docker Engine.

Docklet is a thin layer over the Docker Engine API, not a Docker Desktop
clone. It talks directly to `dockerd` over its unix socket with a small,
hand-rolled HTTP client; it does not shell out to the `docker` CLI, run a
background service, or keep a database. Docker Engine is the source of
truth for everything Docklet shows.

## Features

- **Containers**: list, start, stop, restart, remove; details, inspect,
  logs (one-shot and live follow); live CPU/memory/network stats for the
  container you're looking at
- **Images**: list, pull (with progress), remove, details
- **Volumes**: list, create, remove, details (including which containers
  use one)
- **Networks**: list, create, remove, details (including connected
  containers); Docker's own `bridge`/`host`/`none` networks can't be removed
- **Compose**: projects are detected from containers' own Compose labels
  and shown as an expandable tree (project to services), with a status
  indicator that reflects the current container states; start/stop
  existing containers and view per-service logs. This is not a `compose
  up` replacement, it only ever acts on containers that already exist.

## Requirements

- Docker Engine, reachable over its unix socket (`DOCKER_HOST`, an active
  Docker context, or `/var/run/docker.sock`, tried in that order, the same
  resolution the `docker` CLI uses)
- GTK 4.10 or newer
- Linux (the transport is unix-socket only for now)

## Installation

### From a release (recommended)

Download the latest `docklet-<version>-linux-x86_64.tar.gz` from the
[Releases page](https://github.com/mohibulrefat/docklet/releases),
verify it, and install it:

```sh
tar -xzf docklet-<version>-linux-x86_64.tar.gz
cd docklet-<version>-linux-x86_64

# Optional but recommended: check the download against the published
# checksum.
sha256sum -c docklet-<version>-linux-x86_64.tar.gz.sha256

# Run it directly:
./docklet

# Or install it as a regular desktop application:
install -Dm755 docklet ~/.local/bin/docklet
install -Dm644 dev.docklet.Docklet.desktop \
  ~/.local/share/applications/dev.docklet.Docklet.desktop
install -Dm644 icons/dev.docklet.Docklet.svg \
  ~/.local/share/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg
```

Make sure `~/.local/bin` is on your `PATH` (it is by default on most
distributions). Once installed, Docklet appears in your application
launcher, or you can run it directly with `docklet`.

To uninstall, remove the three files above.

### From source

Requires a Rust toolchain and GTK 4.10 development headers.

```sh
git clone https://github.com/mohibulrefat/docklet.git
cd docklet
cargo build --release
```

The release profile is tuned for a small binary (`opt-level = "s"`, LTO,
stripped symbols) rather than raw execution speed, appropriate for a
utility whose work is mostly waiting on Docker's API, not computing.

Desktop integration files live under `data/` and install the same way as
in the release archive above, just from `data/` instead of the archive
root:

```sh
install -Dm644 data/dev.docklet.Docklet.desktop \
  ~/.local/share/applications/dev.docklet.Docklet.desktop
install -Dm644 data/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg \
  ~/.local/share/icons/hicolor/scalable/apps/dev.docklet.Docklet.svg
install -Dm755 target/release/docklet ~/.local/bin/docklet
```

## Running

```sh
cargo run --release
# or, after building or installing:
./target/release/docklet
docklet
```

By default Docklet renders with GTK4's software (cairo) renderer rather
than its GPU-accelerated one, which measurably reduces idle memory use for
an app with no animations or GPU-bound rendering to benefit from. Set
`GSK_RENDERER` yourself (e.g. `GSK_RENDERER=gl`) to override this.

## Development

### Testing

```sh
cargo test
```

Tests cover Docker API parsing, the request/response framing of the
hand-rolled HTTP client (including a real unix-socket round trip), and the
UI's list-diffing logic. None require a running Docker daemon.

### Project structure

```
src/
├── main.rs
├── docker/    Docker Engine API client, no GTK dependency
└── ui/        GTK4 widgets, no direct HTTP/socket access
```

`docker/` never imports GTK; `ui/` never touches sockets, `DOCKER_HOST`, or
Docker contexts directly, it calls typed functions and renders what comes
back. Endpoint resolution lives entirely in `docker/endpoint.rs`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT
