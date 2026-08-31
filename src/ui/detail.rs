//! The container detail pane.

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Grid, Label, Orientation, Separator, Widget};

use super::logpane::LogPane;
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
    logs: LogPane,
    cpu: Label,
    memory: Label,
    network: Label,
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
        let cpu = field(&grid, 5, "CPU", false);
        let memory = field(&grid, 6, "Memory", false);
        let network = field(&grid, 7, "Network", false);
        let created = field(&grid, 8, "Created", false);
        let command = field(&grid, 9, "Command", true);
        let restart = field(&grid, 10, "Restart", false);
        let networks = field(&grid, 11, "Networks", false);
        let mounts = field(&grid, 12, "Mounts", false);

        let logs = LogPane::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(&grid);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(logs.widget());
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
            cpu,
            memory,
            network,
        }
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub fn connect_back(&self, handler: impl Fn() + 'static) {
        self.back.connect_clicked(move |_| handler());
    }

    pub fn connect_logs_refresh(&self, handler: impl Fn() + 'static) {
        self.logs.connect_refresh(handler);
    }

    /// Called with the new state whenever Follow is toggled.
    pub fn connect_follow(&self, handler: impl Fn(bool) + 'static) {
        self.logs.connect_follow(handler);
    }

    /// Turn Follow off without firing the handler's side effects twice.
    pub fn set_following(&self, following: bool) {
        self.logs.set_following(following);
    }

    /// Append streamed output.
    pub fn append_logs(&self, text: &str) {
        self.logs.append_logs(text);
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
            &self.cpu,
            &self.memory,
            &self.network,
        ] {
            label.set_text("");
        }
        self.set_logs("");
    }

    /// Update the live CPU and memory readings. Called for each sample while
    /// stats are running; does nothing to the rest of the pane.
    pub fn set_stats(&self, cpu_percent: Option<f64>, memory_usage: u64, memory_limit: u64) {
        self.cpu.set_text(&match cpu_percent {
            Some(percent) => format!("{percent:.1}%"),
            // A real "no data yet", not a fabricated 0%.
            None => "—".to_string(),
        });

        self.memory.set_text(&if memory_limit > 0 {
            format!(
                "{} / {}",
                crate::docker::human_size(memory_usage),
                crate::docker::human_size(memory_limit)
            )
        } else {
            crate::docker::human_size(memory_usage)
        });
    }

    /// Show the current network transfer rate, or a placeholder before the
    /// second sample makes a rate computable.
    pub fn set_network_rate(&self, rate: Option<(f64, f64)>) {
        self.network.set_text(&match rate {
            Some((rx, tx)) => format!(
                "\u{2193} {}/s   \u{2191} {}/s",
                crate::docker::human_size(rx as u64),
                crate::docker::human_size(tx as u64)
            ),
            None => "—".to_string(),
        });
    }

    /// Stats stopped or never started for this container; show nothing rather
    /// than a stale reading from whatever was open before.
    pub fn clear_stats(&self) {
        self.cpu.set_text("");
        self.memory.set_text("");
        self.network.set_text("");
    }

    /// Replace the log view's contents.
    pub fn set_logs(&self, text: &str) {
        self.logs.set_logs(text);
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
pub fn field(grid: &Grid, row: i32, name: &str, monospace: bool) -> Label {
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
