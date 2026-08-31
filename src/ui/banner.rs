//! A dismissible error strip, shown above a page's content.

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, Revealer, Widget};

/// An error message the user can dismiss.
pub struct Banner {
    revealer: Revealer,
    label: Label,
}

impl Banner {
    pub fn new() -> Self {
        let label = Label::builder()
            .halign(Align::Start)
            .hexpand(true)
            .wrap(true)
            .xalign(0.0)
            .build();

        let dismiss = Button::from_icon_name("window-close-symbolic");
        dismiss.add_css_class("flat");
        dismiss.set_tooltip_text(Some("Dismiss"));

        let row = GtkBox::new(Orientation::Horizontal, 6);
        row.add_css_class("docklet-banner");
        row.append(&label);
        row.append(&dismiss);

        let revealer = Revealer::builder().child(&row).reveal_child(false).build();
        dismiss.connect_clicked({
            let revealer = revealer.clone();
            move |_| revealer.set_reveal_child(false)
        });

        Banner { revealer, label }
    }

    pub fn widget(&self) -> &Widget {
        self.revealer.upcast_ref()
    }

    pub fn show(&self, message: &str) {
        self.label.set_text(message);
        self.revealer.set_reveal_child(true);
    }

    pub fn clear(&self) {
        self.revealer.set_reveal_child(false);
    }
}
