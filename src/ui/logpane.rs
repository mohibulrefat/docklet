//! A log view with Follow and Refresh — the widget group shared by the
//! container detail pane and Compose's per-service log view, so the two never
//! carry separate copies of the trim/scroll/follow-toggle logic.

use gtk::prelude::*;
use gtk::Label;
use gtk::{
    Align, Box as GtkBox, Button, Orientation, PolicyType, ScrolledWindow, TextView, ToggleButton,
    Widget,
};

/// How many log lines to keep in view. Bounded so following a chatty
/// container cannot grow the buffer without limit.
const MAX_LOG_LINES: i32 = 2000;

pub struct LogPane {
    root: GtkBox,
    logs: TextView,
    logs_scroll: ScrolledWindow,
    refresh: Button,
    follow: ToggleButton,
}

impl LogPane {
    pub fn new() -> Self {
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

        let title = Label::builder()
            .label("Logs")
            .halign(Align::Start)
            .hexpand(true)
            .build();
        title.add_css_class("heading");

        let refresh = Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Refresh logs"));
        refresh.add_css_class("flat");

        let follow = ToggleButton::builder().label("Follow").build();
        follow.set_tooltip_text(Some("Stream new log output as it arrives"));

        let header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(6)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        header.append(&title);
        header.append(&follow);
        header.append(&refresh);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(&logs_scroll);

        LogPane {
            root,
            logs,
            logs_scroll,
            refresh,
            follow,
        }
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub fn connect_refresh(&self, handler: impl Fn() + 'static) {
        self.refresh.connect_clicked(move |_| handler());
    }

    /// Called with the new state whenever Follow is toggled.
    pub fn connect_follow(&self, handler: impl Fn(bool) + 'static) {
        self.follow
            .connect_toggled(move |button| handler(button.is_active()));
    }

    /// Turn Follow off without firing the handler's side effects twice.
    pub fn set_following(&self, following: bool) {
        if self.follow.is_active() != following {
            self.follow.set_active(following);
        }
    }

    /// Replace the log view's contents.
    pub fn set_logs(&self, text: &str) {
        self.logs.buffer().set_text(text);
    }

    /// Append streamed output, trimming the buffer and scrolling to the end.
    pub fn append_logs(&self, text: &str) {
        let buffer = self.logs.buffer();
        buffer.insert(&mut buffer.end_iter(), text);
        self.trim_logs();
        self.scroll_to_end();
    }

    /// Keep only the most recent lines, so a chatty container cannot grow the
    /// buffer without bound while it is being followed.
    fn trim_logs(&self) {
        let buffer = self.logs.buffer();
        let excess = buffer.line_count() - MAX_LOG_LINES;
        if excess <= 0 {
            return;
        }
        let start = buffer.start_iter();
        let Some(cut) = buffer.iter_at_line(excess) else {
            return;
        };
        buffer.delete(&mut start.clone(), &mut cut.clone());
    }

    fn scroll_to_end(&self) {
        let adjustment = self.logs_scroll.vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
    }
}
