//! The container detail pane.

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Grid, Label, Orientation, Separator, Widget};

use super::object::ContainerObject;

/// A read-only view of one container.
pub struct DetailView {
    root: GtkBox,
    back: Button,
    title: Label,
    name: Label,
    id: Label,
    image: Label,
    state: Label,
    status: Label,
}

impl DetailView {
    pub fn new() -> Self {
        let back = Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Back to containers"));

        let title = Label::builder().halign(Align::Start).build();
        title.add_css_class("heading");

        let header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        header.append(&back);
        header.append(&title);

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let name = field(&grid, 0, "Name", false);
        let id = field(&grid, 1, "ID", true);
        let image = field(&grid, 2, "Image", false);
        let state = field(&grid, 3, "State", false);
        let status = field(&grid, 4, "Status", false);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(&grid);
        root.set_visible(false);

        DetailView {
            root,
            back,
            title,
            name,
            id,
            image,
            state,
            status,
        }
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub fn connect_back(&self, handler: impl Fn() + 'static) {
        self.back.connect_clicked(move |_| handler());
    }

    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Fill the pane from what the list already knows.
    pub fn show(&self, container: &ContainerObject) {
        self.title.set_text(&container.name());
        self.name.set_text(&container.name());
        self.id.set_text(&container.id());
        self.image.set_text(&container.image());
        self.state.set_text(&container.state());
        self.status.set_text(&container.status());
    }
}

/// A labelled row; returns the value label so it can be updated later.
fn field(grid: &Grid, row: i32, name: &str, monospace: bool) -> Label {
    let key = Label::builder().label(name).halign(Align::End).build();
    key.add_css_class("dim-label");

    let value = Label::builder()
        .halign(Align::Start)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    if monospace {
        value.add_css_class("monospace");
    }

    grid.attach(&key, 0, row, 1, 1);
    grid.attach(&value, 1, row, 1, 1);
    value
}
