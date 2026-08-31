//! The container detail pane.

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, Grid, Label, Orientation, PolicyType, ScrolledWindow, Separator,
    TextView, Widget,
};

use super::object::ContainerObject;
use crate::docker::Inspect;

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
    created: Label,
    command: Label,
    restart: Label,
    networks: Label,
    mounts: Label,
    logs: TextView,
    logs_refresh: Button,
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
        let created = field(&grid, 5, "Created", false);
        let command = field(&grid, 6, "Command", true);
        let restart = field(&grid, 7, "Restart", false);
        let networks = field(&grid, 8, "Networks", false);
        let mounts = field(&grid, 9, "Mounts", false);

        let logs = TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .left_margin(6)
            .right_margin(6)
            .top_margin(6)
            .bottom_margin(6)
            .build();

        let logs_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&logs)
            .build();

        let logs_title = Label::builder()
            .label("Logs")
            .halign(Align::Start)
            .hexpand(true)
            .build();
        logs_title.add_css_class("heading");

        let logs_refresh = Button::from_icon_name("view-refresh-symbolic");
        logs_refresh.set_tooltip_text(Some("Refresh logs"));
        logs_refresh.add_css_class("flat");

        let logs_header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(6)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        logs_header.append(&logs_title);
        logs_header.append(&logs_refresh);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(&grid);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(&logs_header);
        root.append(&logs_scroll);
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
            created,
            command,
            restart,
            networks,
            mounts,
            logs,
            logs_refresh,
        }
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub fn connect_back(&self, handler: impl Fn() + 'static) {
        self.back.connect_clicked(move |_| handler());
    }

    pub fn connect_logs_refresh(&self, handler: impl Fn() + 'static) {
        self.logs_refresh.connect_clicked(move |_| handler());
    }

    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Fill the pane from what the list already knows.
    ///
    /// The inspect fields are blanked rather than left stale, so a slow inspect
    /// never shows the previous container's details next to this one's name.
    pub fn show(&self, container: &ContainerObject) {
        self.title.set_text(&container.name());
        self.name.set_text(&container.name());
        self.id.set_text(&container.id());
        self.image.set_text(&container.image());
        self.state.set_text(&container.state());
        self.status.set_text(&container.status());

        for label in [
            &self.created,
            &self.command,
            &self.restart,
            &self.networks,
            &self.mounts,
        ] {
            label.set_text("");
        }
        self.set_logs("");
    }

    /// Replace the log view's contents.
    pub fn set_logs(&self, text: &str) {
        self.logs.buffer().set_text(text);
    }

    /// Fill in the fields that only a full inspect provides.
    pub fn set_inspect(&self, inspect: &Inspect) {
        self.created.set_text(&inspect.created);
        self.command.set_text(&inspect.command());
        self.restart
            .set_text(&inspect.host_config.restart_policy.name);
        self.networks.set_text(&inspect.networks());
        self.mounts.set_text(&inspect.mount_list());
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
