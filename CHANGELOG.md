# Changelog

## [0.1.0] - 2026-09-12

First public release.

Docklet is a lightweight native GTK4 GUI for Docker Engine. It is a thin
layer over the Docker Engine API — not a Docker Desktop clone. It talks
directly to `dockerd` over its unix socket with a small, hand-rolled HTTP
client; it does not shell out to the `docker` CLI, run a background
service, or keep a database. Docker Engine is the source of truth for
everything Docklet shows.

### Features

- **Containers** — list, start, stop, restart, remove; details, inspect,
  logs (one-shot and live follow); live CPU/memory/network stats for the
  container you're looking at
- **Images** — list, pull (with progress), remove, details
- **Volumes** — list, create, remove, details (including which containers
  use one)
- **Networks** — list, create, remove, details (including connected
  containers); Docker's own `bridge`/`host`/`none` networks can't be
  removed
- **Compose** — projects are detected from containers' own Compose labels
  and shown as an expandable tree (project → services), with a status
  indicator that reflects the current container states — including a
  Partial state when a project's services disagree; start/stop existing
  containers and view per-service logs. This is not a `compose up`
  replacement — it only ever acts on containers that already exist.
- Live Docker connection status in the footer, and a clear "stale" notice
  on any page whose data could not be refreshed because Docker was
  unreachable.

### Requirements

- Docker Engine, reachable over its unix socket (`DOCKER_HOST`, an active
  Docker context, or `/var/run/docker.sock`, tried in that order — the
  same resolution the `docker` CLI uses)
- GTK 4.10 or newer
- Linux (the transport is unix-socket only for now)

### Installation notes

See `README.md` (or `INSTALL.txt` in the release archive) for install
steps. The release binary is self-contained: it depends only on your
system's GTK4 libraries, not on the `docker` CLI or on Cargo/Rust being
installed.

### Known limitations

- A handful of harmless `Gtk-WARNING` messages about slider sizing may be
  printed to stderr on startup; they do not affect functionality.
- Compose support is read/label-based only — it does not run `docker
  compose` itself, so it cannot create containers that don't exist yet.
