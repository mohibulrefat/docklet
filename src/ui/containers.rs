//! The containers page.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, ColumnView, ColumnViewColumn, CssProvider, Label, ListItem, PolicyType, ScrolledWindow,
    SignalListItemFactory, SingleSelection, Widget,
};

use super::object::ContainerObject;
use crate::docker::Docker;

const LOG_DOMAIN: &str = "docklet";

/// Colours for the state bullet. `alpha(currentColor, …)` follows the theme,
/// so the stopped bullet stays legible in both light and dark.
const STYLE: &str = "
.state-running     { color: #33d17a; }
.state-transitional{ color: #e5a50a; }
.state-dead        { color: #e01b24; }
.state-stopped     { color: alpha(currentColor, 0.45); }
";

/// Every class `state_indicator` can apply, so a recycled row can be cleared.
const STATE_CLASSES: [&str; 4] = [
    "state-running",
    "state-transitional",
    "state-dead",
    "state-stopped",
];

/// The containers list, and the store backing it.
pub struct ContainersPage {
    root: ScrolledWindow,
    store: gio::ListStore,
}

impl ContainersPage {
    pub fn new() -> Self {
        let store = gio::ListStore::new::<ContainerObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        install_style();

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&state_column());
        view.append_column(&text_column("Name", ContainerObject::name));
        view.append_column(&text_column("Image", ContainerObject::image));
        view.append_column(&text_column("Status", ContainerObject::status));

        let root = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let page = ContainersPage { root, store };
        page.refresh();
        page
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    /// Reload the list from Docker.
    pub fn refresh(&self) {
        let store = self.store.clone();
        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.containers(true)).await;

            match listed {
                Ok(Ok(containers)) => {
                    store.remove_all();
                    for container in &containers {
                        store.append(&ContainerObject::new(container));
                    }
                }
                Ok(Err(e)) => glib::g_warning!(LOG_DOMAIN, "could not list containers: {e}"),
                Err(_) => glib::g_warning!(LOG_DOMAIN, "listing containers panicked"),
            }
        });
    }
}

/// A column showing one text field of a container.
fn text_column(title: &str, field: fn(&ContainerObject) -> String) -> ColumnViewColumn {
    let factory = SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let label = Label::builder().halign(Align::Start).build();
        item.downcast_ref::<ListItem>()
            .expect("list item")
            .set_child(Some(&label));
    });

    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<ListItem>().expect("list item");
        let Some(object) = item.item().and_downcast::<ContainerObject>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<Label>() else {
            return;
        };
        label.set_text(&field(&object));
    });

    ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .build()
}

/// The bullet and CSS class for a container state.
///
/// Docker reports `running`, `exited`, `created`, `paused`, `restarting` and
/// `dead`; anything unrecognised is shown as stopped rather than hidden.
fn state_indicator(state: &str) -> (&'static str, &'static str) {
    match state {
        "running" => ("\u{25cf}", "state-running"),
        "paused" | "restarting" => ("\u{25cf}", "state-transitional"),
        "dead" => ("\u{25cf}", "state-dead"),
        _ => ("\u{25cb}", "state-stopped"),
    }
}

/// The leading column: a coloured bullet for the container's state.
fn state_column() -> ColumnViewColumn {
    let factory = SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let label = Label::builder().halign(Align::Center).build();
        item.downcast_ref::<ListItem>()
            .expect("list item")
            .set_child(Some(&label));
    });

    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<ListItem>().expect("list item");
        let Some(object) = item.item().and_downcast::<ContainerObject>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<Label>() else {
            return;
        };

        let (bullet, class) = state_indicator(&object.state());
        label.set_text(bullet);
        label.set_tooltip_text(Some(&object.state()));

        // Rows are recycled, so drop whatever the previous container left.
        for stale in STATE_CLASSES {
            label.remove_css_class(stale);
        }
        label.add_css_class(class);
    });

    ColumnViewColumn::builder().factory(&factory).build()
}

/// Install the state colours once, for the whole display.
fn install_style() {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        let provider = CssProvider::new();
        provider.load_from_data(STYLE);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(state: &str) -> &'static str {
        state_indicator(state).1
    }

    fn bullet(state: &str) -> &'static str {
        state_indicator(state).0
    }

    #[test]
    fn running_is_a_filled_green_bullet() {
        assert_eq!(state_indicator("running"), ("\u{25cf}", "state-running"));
    }

    #[test]
    fn stopped_states_are_hollow() {
        assert_eq!(bullet("exited"), "\u{25cb}");
        assert_eq!(bullet("created"), "\u{25cb}");
        assert_eq!(class("exited"), "state-stopped");
        assert_eq!(class("created"), "state-stopped");
    }

    #[test]
    fn transitional_states_share_a_class() {
        assert_eq!(class("paused"), "state-transitional");
        assert_eq!(class("restarting"), "state-transitional");
    }

    #[test]
    fn dead_has_its_own_class() {
        assert_eq!(class("dead"), "state-dead");
    }

    #[test]
    fn an_unknown_state_falls_back_to_stopped() {
        assert_eq!(class("something-new"), "state-stopped");
    }

    #[test]
    fn every_class_used_can_be_cleared_from_a_recycled_row() {
        for state in [
            "running",
            "paused",
            "restarting",
            "dead",
            "exited",
            "created",
        ] {
            let applied = class(state);
            assert!(
                STATE_CLASSES.contains(&applied),
                "{applied} is missing from STATE_CLASSES, so recycled rows would keep it"
            );
        }
    }
}
