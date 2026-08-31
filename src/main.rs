mod docker;
mod ui;

use gtk::glib;
use gtk::prelude::*;
use gtk::Application;

const APP_ID: &str = "dev.docklet.Docklet";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| ui::build(app).present());
    app.run()
}
