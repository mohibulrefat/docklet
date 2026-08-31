//! The containers page.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;
use gtk::{
    AlertDialog, Align, Box as GtkBox, Button, ColumnView, ColumnViewColumn, CssProvider, Label,
    ListItem, Orientation, PolicyType, ScrolledWindow, SignalListItemFactory, SingleSelection,
    Widget, Window,
};

use super::object::ContainerObject;
use crate::docker::{Container, Docker, DockerError};

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
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    /// Guards against a second refresh starting while one is in flight.
    refreshing: Rc<Cell<bool>>,
}

impl ContainersPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<ContainerObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        install_style();

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&state_column());
        view.append_column(&text_column("Name", ContainerObject::name));
        view.append_column(&text_column("Image", ContainerObject::image));
        view.append_column(&mono_column("ID", ContainerObject::short_id));
        view.append_column(&text_column("Status", ContainerObject::status));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = Label::builder()
            .label("No containers")
            .vexpand(true)
            .visible(false)
            .build();
        empty.add_css_class("dim-label");

        let actions = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&actions);
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(ContainersPage {
            root,
            scrolled,
            empty,
            store,
            selection,
            refreshing: Rc::new(Cell::new(false)),
        });

        actions.append(&page.action_button("Start", Docker::start_container));
        actions.append(&page.action_button("Stop", Docker::stop_container));
        actions.append(&page.action_button("Restart", Docker::restart_container));

        let remove = Button::with_label("Remove");
        remove.add_css_class("destructive-action");
        remove.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.remove_selected();
                }
            }
        });
        actions.append(&remove);

        page.refresh();
        page
    }

    /// The container the user has selected, if any.
    fn selected(&self) -> Option<ContainerObject> {
        self.selection
            .selected_item()
            .and_downcast::<ContainerObject>()
    }

    /// A button that runs one lifecycle action on the selected container.
    fn action_button(
        self: &Rc<Self>,
        label: &'static str,
        action: fn(&Docker, &str) -> Result<(), DockerError>,
    ) -> Button {
        let button = Button::with_label(label);
        let page = Rc::downgrade(self);

        button.connect_clicked(move |_| {
            let Some(page) = page.upgrade() else {
                return;
            };
            page.act(label, action);
        });
        button
    }

    /// Remove the selected container, after confirming.
    ///
    /// Docker refuses to remove a running container with a 409; rather than
    /// reporting that as a failure, we offer to stop it first.
    fn remove_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let (id, name) = (row.id(), row.name());
        let parent = self.root.root().and_downcast::<Window>();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let confirmed = confirm(
                parent.as_ref(),
                &format!("Remove {name}?"),
                "The container and its writable layer are deleted. This cannot be undone.",
                "Remove",
            )
            .await;
            if !confirmed {
                return;
            }

            match remove(&id, false).await {
                Ok(()) => {}
                Err(DockerError::Api { status: 409, .. }) => {
                    let forced = confirm(
                        parent.as_ref(),
                        &format!("{name} is still running."),
                        "Removing it will stop the container first.",
                        "Force remove",
                    )
                    .await;
                    if !forced {
                        return;
                    }
                    if let Err(e) = remove(&id, true).await {
                        glib::g_warning!(LOG_DOMAIN, "force remove failed: {e}");
                        return;
                    }
                }
                Err(e) => {
                    glib::g_warning!(LOG_DOMAIN, "remove failed: {e}");
                    return;
                }
            }

            if let Some(page) = page.upgrade() {
                page.refresh();
            }
        });
    }

    /// Run an action against the selected container, then reload the list.
    fn act(
        self: &Rc<Self>,
        label: &'static str,
        action: fn(&Docker, &str) -> Result<(), DockerError>,
    ) {
        let Some(row) = self.selected() else {
            return;
        };
        let id = row.id();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || action(&Docker::connect()?, &id)).await;

            match outcome {
                Ok(Ok(())) => {
                    if let Some(page) = page.upgrade() {
                        page.refresh();
                    }
                }
                Ok(Err(e)) => glib::g_warning!(LOG_DOMAIN, "{label} failed: {e}"),
                Err(_) => glib::g_warning!(LOG_DOMAIN, "{label} panicked"),
            }
        });
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    /// Reload the list from Docker.
    ///
    /// Does nothing if a refresh is already running, so holding the button down
    /// cannot pile up requests.
    pub fn refresh(&self) {
        if self.refreshing.replace(true) {
            return;
        }

        let store = self.store.clone();
        let refreshing = self.refreshing.clone();
        let scrolled = self.scrolled.clone();
        let empty = self.empty.clone();

        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.containers(true)).await;

            match listed {
                Ok(Ok(containers)) => {
                    apply(&store, &containers);
                    // Only after a completed load, so an empty grid during the
                    // first fetch is never mistaken for "no containers".
                    let is_empty = store.n_items() == 0;
                    empty.set_visible(is_empty);
                    scrolled.set_visible(!is_empty);
                }
                Ok(Err(e)) => glib::g_warning!(LOG_DOMAIN, "could not list containers: {e}"),
                Err(_) => glib::g_warning!(LOG_DOMAIN, "listing containers panicked"),
            }
            refreshing.set(false);
        });
    }
}

/// Ask the user to confirm a destructive action.
///
/// Cancel is both the default and the cancel button, so a stray Return or
/// Escape never deletes anything.
async fn confirm(parent: Option<&Window>, message: &str, detail: &str, action: &str) -> bool {
    let dialog = AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .buttons(["Cancel", action])
        .cancel_button(0)
        .default_button(0)
        .build();

    // An error means the dialog was dismissed, which is a "no".
    dialog.choose_future(parent).await == Ok(1)
}

/// Remove a container off the main thread.
async fn remove(id: &str, force: bool) -> Result<(), DockerError> {
    let id = id.to_string();
    gio::spawn_blocking(move || Docker::connect()?.remove_container(&id, force))
        .await
        .unwrap_or_else(|_| Err(DockerError::Protocol("the remove task panicked".into())))
}

/// Bring the store into line with a freshly fetched list.
///
/// Rows are matched by container id and updated in place rather than the store
/// being cleared and refilled: that keeps the selection, avoids flicker, and
/// leaves rows whose values did not change completely untouched.
fn apply(store: &gio::ListStore, containers: &[Container]) {
    for (index, container) in containers.iter().enumerate() {
        let index = index as u32;

        match find(store, &container.id, index) {
            Some(found) if found == index => {
                let row = row_at(store, index);
                if !row.matches(container) {
                    row.set(container);
                    // No properties to notify, so ask the view to rebind.
                    store.items_changed(index, 1, 1);
                }
            }
            Some(found) => {
                // Same container, different position: move it rather than
                // rebuilding, so its row keeps its identity.
                let row = row_at(store, found);
                store.remove(found);
                row.set(container);
                store.insert(index, &row);
            }
            None => store.insert(index, &ContainerObject::new(container)),
        }
    }

    // Anything past the new length is gone from Docker.
    while store.n_items() > containers.len() as u32 {
        store.remove(store.n_items() - 1);
    }
}

/// Position of the row for `id`, searching from `from` onwards.
fn find(store: &gio::ListStore, id: &str, from: u32) -> Option<u32> {
    (from..store.n_items()).find(|&i| row_at(store, i).id() == id)
}

fn row_at(store: &gio::ListStore, index: u32) -> ContainerObject {
    store
        .item(index)
        .and_downcast::<ContainerObject>()
        .expect("the store only ever holds ContainerObjects")
}

/// A column of text that shares the remaining width and truncates when narrow.
fn text_column(title: &str, field: fn(&ContainerObject) -> String) -> ColumnViewColumn {
    column(title, field, false, true)
}

/// A fixed-width column of monospace text, for ids.
fn mono_column(title: &str, field: fn(&ContainerObject) -> String) -> ColumnViewColumn {
    column(title, field, true, false)
}

fn column(
    title: &str,
    field: fn(&ContainerObject) -> String,
    monospace: bool,
    expand: bool,
) -> ColumnViewColumn {
    let factory = SignalListItemFactory::new();

    factory.connect_setup(move |_, item| {
        let label = Label::builder()
            .halign(Align::Start)
            .ellipsize(EllipsizeMode::End)
            .build();
        if monospace {
            label.add_css_class("monospace");
        }
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
        .expand(expand)
        .resizable(true)
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

    /// Build a container with just the fields the store compares.
    fn container(id: &str, name: &str, state: &str) -> Container {
        serde_json::from_str(&format!(
            r#"{{"Id":"{id}","Names":["/{name}"],"Image":"img","State":"{state}",
                 "Status":"status-{state}"}}"#
        ))
        .expect("fixture should parse")
    }

    fn store_of(containers: &[Container]) -> gio::ListStore {
        let store = gio::ListStore::new::<ContainerObject>();
        apply(&store, containers);
        store
    }

    fn ids(store: &gio::ListStore) -> Vec<String> {
        (0..store.n_items())
            .map(|i| row_at(store, i).id())
            .collect()
    }

    #[test]
    fn fills_an_empty_store() {
        let store = store_of(&[container("a", "one", "running")]);
        assert_eq!(ids(&store), ["a"]);
        assert_eq!(row_at(&store, 0).name(), "one");
    }

    #[test]
    fn appends_a_new_container() {
        let store = store_of(&[container("a", "one", "running")]);
        apply(
            &store,
            &[
                container("a", "one", "running"),
                container("b", "two", "exited"),
            ],
        );
        assert_eq!(ids(&store), ["a", "b"]);
    }

    #[test]
    fn removes_a_departed_container() {
        let store = store_of(&[
            container("a", "one", "running"),
            container("b", "two", "exited"),
        ]);
        apply(&store, &[container("b", "two", "exited")]);
        assert_eq!(ids(&store), ["b"]);
    }

    #[test]
    fn removes_every_container() {
        let store = store_of(&[container("a", "one", "running")]);
        apply(&store, &[]);
        assert_eq!(store.n_items(), 0);
    }

    #[test]
    fn updates_a_changed_container_in_place() {
        let store = store_of(&[container("a", "one", "running")]);
        let before = row_at(&store, 0);

        apply(&store, &[container("a", "one", "exited")]);

        let after = row_at(&store, 0);
        assert_eq!(after.state(), "exited");
        // Same GObject, not a replacement: the row kept its identity.
        assert_eq!(before, after);
    }

    #[test]
    fn keeps_the_same_row_object_when_nothing_changed() {
        let store = store_of(&[container("a", "one", "running")]);
        let before = row_at(&store, 0);

        apply(&store, &[container("a", "one", "running")]);

        assert_eq!(before, row_at(&store, 0));
        assert!(before.matches(&container("a", "one", "running")));
    }

    #[test]
    fn reorders_without_rebuilding_rows() {
        let a = container("a", "one", "running");
        let b = container("b", "two", "exited");
        let store = store_of(&[a.clone(), b.clone()]);
        let row_a = row_at(&store, 0);

        apply(&store, &[b, a]);

        assert_eq!(ids(&store), ["b", "a"]);
        // "a" moved rather than being recreated.
        assert_eq!(row_a, row_at(&store, 1));
    }

    #[test]
    fn handles_a_wholesale_replacement() {
        let store = store_of(&[container("a", "one", "running")]);
        apply(
            &store,
            &[
                container("x", "nine", "exited"),
                container("y", "ten", "dead"),
            ],
        );
        assert_eq!(ids(&store), ["x", "y"]);
    }

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
