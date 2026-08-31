mod docker;

use gtk::glib;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow};

const APP_ID: &str = "dev.docklet.Docklet";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt::init();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_window);
    app.run()
}

fn build_window(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Docklet")
        .default_width(900)
        .default_height(600)
        .build();

    window.present();
}
