mod docker;
mod ui;

use gtk::glib;
use gtk::prelude::*;
use gtk::Application;

const APP_ID: &str = "dev.docklet.Docklet";

fn main() -> glib::ExitCode {
    prefer_lightweight_renderer();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| ui::build(app).present());
    app.run()
}

/// Default to GTK4's software (cairo) renderer over its GPU-accelerated one.
///
/// Measured on this machine: the GPU renderer pulls in the graphics driver
/// stack (Vulkan/GL libraries) and costs roughly 87 MB of RSS beyond what
/// cairo needs, for an app with no animations, shaders, or GPU-bound
/// rendering to benefit from — plain widgets and text throughout. Only sets
/// it when nothing has already chosen a renderer, so a user's or session's
/// own `GSK_RENDERER` is always respected over this default.
fn prefer_lightweight_renderer() {
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: called at the very start of main, before any other thread
        // exists and before GTK reads this variable during initialization.
        unsafe {
            std::env::set_var("GSK_RENDERER", "cairo");
        }
    }
}
