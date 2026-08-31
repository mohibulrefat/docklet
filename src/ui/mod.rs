//! The GTK layer.
//!
//! Nothing here knows how Docker is reached — no socket paths, no contexts, no
//! HTTP. It calls typed functions on `docker` and renders what comes back.

mod containers;
mod detail;
mod object;
mod window;

pub use window::build;
