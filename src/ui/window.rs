//! The main window: a header bar, a page stack, and a status footer.
//!
//! The pages are placeholders for now; each is replaced by the milestone that
//! implements it.

use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, HeaderBar, Label, Orientation, Separator, Stack,
    StackSwitcher,
};

const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 600;

/// Build the main window.
pub fn build(app: &Application) -> ApplicationWindow {
    let stack = Stack::builder().vexpand(true).build();
    stack.add_titled(&placeholder("Containers"), Some("containers"), "Containers");
    stack.add_titled(&placeholder("Images"), Some("images"), "Images");
    stack.add_titled(&placeholder("Volumes"), Some("volumes"), "Volumes");
    stack.add_titled(&placeholder("Networks"), Some("networks"), "Networks");

    let header = HeaderBar::new();
    header.set_title_widget(Some(&StackSwitcher::builder().stack(&stack).build()));

    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&Separator::new(Orientation::Horizontal));
    content.append(&status_bar());

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Docklet")
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .child(&content)
        .build();
    window.set_titlebar(Some(&header));
    window
}

/// A page that has not been implemented yet.
fn placeholder(name: &str) -> Label {
    let label = Label::new(Some(name));
    label.add_css_class("dim-label");
    label.set_vexpand(true);
    label
}

/// The footer that reports Docker's status.
fn status_bar() -> Label {
    let label = Label::new(Some("—"));
    label.add_css_class("dim-label");
    label.set_halign(Align::Start);
    label.set_margin_top(4);
    label.set_margin_bottom(4);
    label.set_margin_start(8);
    label.set_margin_end(8);
    label
}
