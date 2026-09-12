//! The GTK layer.
//!
//! Nothing here knows how Docker is reached — no socket paths, no contexts, no
//! HTTP. It calls typed functions on `docker` and renders what comes back.

mod banner;
mod compose;
mod connection;
mod containers;
mod detail;
mod dialog;
mod images;
mod list;
mod logpane;
mod networks;
mod object;
mod volumes;
mod window;

pub use window::build;
