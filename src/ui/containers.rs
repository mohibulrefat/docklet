//! The containers page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, ColumnView, ColumnViewColumn, CssProvider, Label, ListItem,
    Orientation, PolicyType, ScrolledWindow, SignalListItemFactory, SingleSelection, Widget,
    Window,
};

use super::banner::Banner;
use super::detail::DetailView;
use super::dialog::confirm;
use super::list::{self, mono_column, text_column};
use super::object::ContainerObject;
use crate::docker::{Docker, DockerError, LogEvent, StatsEvent, StreamHandle};

const LOG_DOMAIN: &str = "docklet";

/// How many log lines to fetch. Bounded so a chatty container cannot fill
/// memory just by being opened.
const LOG_TAIL: usize = 500;

/// Colours for the state bullet. `alpha(currentColor, …)` follows the theme,
/// so the stopped bullet stays legible in both light and dark.
const STYLE: &str = "
.state-running     { color: #33d17a; }
.state-transitional{ color: #e5a50a; }
.state-dead        { color: #e01b24; }
.state-stopped     { color: alpha(currentColor, 0.45); }
.docklet-banner    { background: alpha(#e01b24, 0.15); padding: 6px; }
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
    /// The action bar; made insensitive while an action runs.
    actions: GtkBox,
    detail: DetailView,
    banner: Banner,
    /// Guards against a second refresh starting while one is in flight.
    refreshing: Rc<Cell<bool>>,
    /// A refresh asked for while one was already running.
    refresh_again: Rc<Cell<bool>>,
    /// True while a lifecycle action is in flight.
    busy: Rc<Cell<bool>>,
    /// The container the detail pane is showing, and whether it has a TTY.
    /// Remembered so logs can be re-fetched without another inspect.
    open_container: RefCell<Option<(String, bool)>>,
    /// The live log stream, if Follow is on. Dropping it stops the worker,
    /// so replacing or clearing this is what tears the stream down.
    follow: RefCell<Option<StreamHandle>>,
    /// The stats stream for the open detail pane. Exactly one at a time,
    /// structurally: opening a detail pane replaces this, and closing one
    /// clears it — there is no path that leaves two running.
    stats: RefCell<Option<StreamHandle>>,
}

impl ContainersPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<ContainerObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        install_style();

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&state_column());
        view.append_column(&text_column::<ContainerObject>(
            "Name",
            ContainerObject::name,
        ));
        view.append_column(&text_column::<ContainerObject>(
            "Image",
            ContainerObject::image,
        ));
        view.append_column(&mono_column::<ContainerObject>(
            "ID",
            ContainerObject::short_id,
        ));
        view.append_column(&text_column::<ContainerObject>(
            "Status",
            ContainerObject::status,
        ));

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

        let banner = Banner::new();

        let detail = DetailView::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
        root.append(&scrolled);
        root.append(&empty);
        root.append(detail.widget());

        let page = Rc::new(ContainersPage {
            root,
            scrolled,
            empty,
            store,
            selection,
            actions: actions.clone(),
            detail,
            banner,
            refreshing: Rc::new(Cell::new(false)),
            refresh_again: Rc::new(Cell::new(false)),
            busy: Rc::new(Cell::new(false)),
            open_container: RefCell::new(None),
            follow: RefCell::new(None),
            stats: RefCell::new(None),
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

        // Actions need a selected container, so follow the selection.
        page.selection.connect_selected_item_notify({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.sync_actions();
                }
            }
        });

        // Activating a row (double-click or Enter) opens its detail pane.
        view.connect_activate({
            let page = Rc::downgrade(&page);
            move |_, _| {
                if let Some(page) = page.upgrade() {
                    page.open_detail();
                }
            }
        });

        page.detail.connect_logs_refresh({
            let page = Rc::downgrade(&page);
            move || {
                if let Some(page) = page.upgrade() {
                    page.reload_logs();
                }
            }
        });

        page.detail.connect_follow({
            let page = Rc::downgrade(&page);
            move |following| {
                if let Some(page) = page.upgrade() {
                    if following {
                        page.start_follow();
                    } else {
                        page.stop_follow();
                    }
                }
            }
        });

        page.detail.connect_back({
            let page = Rc::downgrade(&page);
            move || {
                if let Some(page) = page.upgrade() {
                    page.close_detail();
                }
            }
        });

        page.sync_actions();
        page.refresh();
        page
    }

    /// Show the detail pane for the selected container.
    fn open_detail(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        self.detail.show(&row);

        self.actions.set_visible(false);
        self.scrolled.set_visible(false);
        self.empty.set_visible(false);
        self.detail.set_visible(true);

        self.load_inspect(&row.id());
        self.start_stats(&row.id());
    }

    /// Begin streaming CPU and memory for the open container.
    ///
    /// One stream only, for whichever container is currently open; replacing
    /// `self.stats` drops and stops any previous one.
    fn start_stats(self: &Rc<Self>, id: &str) {
        let (sender, receiver) = async_channel::bounded::<StatsEvent>(16);

        let handle = match Docker::connect() {
            Ok(docker) => docker.follow_stats(id, sender),
            Err(_) => return, // Already reported by the status footer.
        };
        self.stats.replace(Some(handle));

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // The previous sample's network totals and timestamp, for turning
            // cumulative counters into a rate. `None` until a second sample
            // arrives, which is also why the first reading shows "—".
            let mut previous: Option<(f64, u64, u64)> = None;

            // Docker does not close this stream when a container merely
            // stops — verified directly against the daemon, and it matches
            // `docker stats` itself: the numbers just settle at zero, which
            // the per-sample update below already renders correctly with no
            // special case needed. The stream *does* end — via a clean EOF,
            // not an error — when the container is removed, which is what
            // the loop exiting without a `Failed` event below is for.
            while let Ok(event) = receiver.recv().await {
                let Some(page) = page.upgrade() else {
                    return;
                };
                match event {
                    StatsEvent::Sample(sample) => {
                        page.detail.set_stats(
                            sample.cpu_percent(),
                            sample.memory_stats.working_set(),
                            sample.memory_stats.limit,
                        );

                        let (rx, tx) = sample.network_totals();
                        let rate = match (previous, sample.read_seconds()) {
                            (Some((prev_t, prev_rx, prev_tx)), Some(now_t)) if now_t > prev_t => {
                                let elapsed = now_t - prev_t;
                                Some((
                                    (rx.saturating_sub(prev_rx)) as f64 / elapsed,
                                    (tx.saturating_sub(prev_tx)) as f64 / elapsed,
                                ))
                            }
                            _ => None,
                        };
                        page.detail.set_network_rate(rate);

                        if let Some(now_t) = sample.read_seconds() {
                            previous = Some((now_t, rx, tx));
                        }
                    }
                    StatsEvent::Failed(message) => {
                        glib::g_warning!(LOG_DOMAIN, "stats stream failed: {message}");
                        page.detail.clear_stats();
                        return;
                    }
                }
            }

            // The channel closed with no `Failed` event: a clean EOF, which
            // is what a removed container looks like. Blanking the fields
            // here is what stops them from showing a frozen last reading for
            // a container that no longer exists.
            if let Some(page) = page.upgrade() {
                page.detail.clear_stats();
            }
        });
    }

    /// Fetch the fields only a full inspect provides.
    fn load_inspect(self: &Rc<Self>, id: &str) {
        let id = id.to_string();
        let id_for_logs = id.clone();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let inspected =
                gio::spawn_blocking(move || Docker::connect()?.inspect_container(&id)).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match inspected {
                Ok(Ok(inspect)) => {
                    page.detail.set_inspect(&inspect);
                    let tty = inspect.config.tty;
                    page.open_container
                        .replace(Some((id_for_logs.clone(), tty)));
                    // Framing depends on the TTY flag, so logs wait for inspect.
                    page.load_logs(&id_for_logs, tty);
                }
                Ok(Err(e)) => page.show_error(&format!("Could not inspect container. {e}")),
                Err(_) => page.show_error("Could not inspect container."),
            }
        });
    }

    /// Fetch the tail of the container's logs.
    fn load_logs(self: &Rc<Self>, id: &str, tty: bool) {
        let id = id.to_string();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let fetched =
                gio::spawn_blocking(move || Docker::connect()?.container_logs(&id, tty, LOG_TAIL))
                    .await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match fetched {
                Ok(Ok(text)) => page.detail.set_logs(&text),
                Ok(Err(e)) => page.show_error(&format!("Could not read logs. {e}")),
                Err(_) => page.show_error("Could not read logs."),
            }
        });
    }

    /// Re-fetch the open container's logs.
    ///
    /// A manual refresh is a one-shot read, so it turns Follow off rather than
    /// leaving a stream appending underneath it.
    fn reload_logs(self: &Rc<Self>) {
        self.stop_follow();
        self.detail.set_following(false);
        let open = self.open_container.borrow().clone();
        if let Some((id, tty)) = open {
            self.load_logs(&id, tty);
        }
    }

    /// Begin streaming the open container's logs.
    fn start_follow(self: &Rc<Self>) {
        let open = self.open_container.borrow().clone();
        let Some((id, tty)) = open else {
            return;
        };

        let (sender, receiver) = async_channel::bounded::<LogEvent>(64);

        let handle = match Docker::connect() {
            Ok(docker) => docker.follow_logs(&id, tty, LOG_TAIL, sender),
            Err(e) => {
                self.show_error(&format!("Could not follow logs. {e}"));
                self.detail.set_following(false);
                return;
            }
        };
        // Replacing any previous handle drops it, stopping that stream.
        self.follow.replace(Some(handle));

        // Following restarts from the tail, so clear what the one-shot fetch left.
        self.detail.set_logs("");

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = receiver.recv().await {
                let Some(page) = page.upgrade() else {
                    return;
                };
                match event {
                    LogEvent::Text(text) => page.detail.append_logs(&text),
                    LogEvent::Failed(message) => {
                        page.show_error(&format!("Log stream ended. {message}"));
                        page.stop_follow();
                        page.detail.set_following(false);
                        return;
                    }
                }
            }
        });
    }

    /// Stop streaming, if we are.
    fn stop_follow(&self) {
        // Dropping the handle shuts the socket down, which unblocks the worker.
        self.follow.replace(None);
    }

    /// Return to the list.
    fn close_detail(self: &Rc<Self>) {
        self.stop_follow();
        self.detail.set_following(false);
        // Dropping the handle stops the worker; nothing keeps sampling once
        // the pane that showed the numbers is gone.
        self.stats.replace(None);
        self.detail.clear_stats();
        self.open_container.replace(None);
        self.detail.set_visible(false);
        self.actions.set_visible(true);
        self.sync_list_visibility();
    }

    /// Show either the list or the empty state, whichever fits the store.
    fn sync_list_visibility(&self) {
        let is_empty = self.store.n_items() == 0;
        self.empty.set_visible(is_empty);
        self.scrolled.set_visible(!is_empty);
    }

    /// Whether the detail pane is currently showing.
    fn showing_detail(&self) -> bool {
        self.detail.widget().is_visible()
    }

    /// Enable the action bar only when it can actually do something.
    fn sync_actions(&self) {
        let ready = !self.busy.get() && self.selected().is_some();
        self.actions.set_sensitive(ready);
    }

    /// Show a failure to the user, and log it.
    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    fn clear_error(&self) {
        self.banner.clear();
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
        self.clear_error();
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
                        if let Some(page) = page.upgrade() {
                            page.show_error(&format!("Could not force remove {name}. {e}"));
                        }
                        return;
                    }
                }
                Err(e) => {
                    if let Some(page) = page.upgrade() {
                        page.show_error(&format!("Could not remove {name}. {e}"));
                    }
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
        let (id, name) = (row.id(), row.name());

        self.clear_error();
        self.busy.set(true);
        self.sync_actions();

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || action(&Docker::connect()?, &id)).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            page.busy.set(false);
            page.sync_actions();

            let verb = label.to_lowercase();
            match outcome {
                Ok(Ok(())) => page.refresh(),
                Ok(Err(e)) => page.show_error(&format!("Could not {verb} {name}. {e}")),
                Err(_) => page.show_error(&format!("Could not {verb} {name}.")),
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
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            // A refresh is already running, and it may have read Docker before
            // whatever prompted this one. Queue a second pass rather than
            // dropping the request and showing stale state.
            self.refresh_again.set(true);
            return;
        }

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.containers(true)).await;

            let Some(page) = page.upgrade() else {
                return;
            };

            match listed {
                Ok(Ok(containers)) => {
                    list::apply::<ContainerObject>(&page.store, &containers);
                    // Only after a completed load, so an empty grid during the
                    // first fetch is never mistaken for "no containers".
                    if !page.showing_detail() {
                        page.sync_list_visibility();
                    }
                    page.sync_actions();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list containers. {e}")),
                Err(_) => page.show_error("Could not list containers."),
            }

            page.refreshing.set(false);
            if page.refresh_again.replace(false) {
                page.refresh();
            }
        });
    }
}

/// Remove a container off the main thread.
async fn remove(id: &str, force: bool) -> Result<(), DockerError> {
    let id = id.to_string();
    gio::spawn_blocking(move || Docker::connect()?.remove_container(&id, force))
        .await
        .unwrap_or_else(|_| Err(DockerError::Protocol("the remove task panicked".into())))
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
    use crate::docker::Container;

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
        list::apply::<ContainerObject>(&store, containers);
        store
    }

    fn ids(store: &gio::ListStore) -> Vec<String> {
        (0..store.n_items())
            .map(|i| list::row_at::<ContainerObject>(store, i).id())
            .collect()
    }

    #[test]
    fn fills_an_empty_store() {
        let store = store_of(&[container("a", "one", "running")]);
        assert_eq!(ids(&store), ["a"]);
        assert_eq!(list::row_at::<ContainerObject>(&store, 0).name(), "one");
    }

    #[test]
    fn appends_a_new_container() {
        let store = store_of(&[container("a", "one", "running")]);
        list::apply::<ContainerObject>(
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
        list::apply::<ContainerObject>(&store, &[container("b", "two", "exited")]);
        assert_eq!(ids(&store), ["b"]);
    }

    #[test]
    fn removes_every_container() {
        let store = store_of(&[container("a", "one", "running")]);
        list::apply::<ContainerObject>(&store, &[]);
        assert_eq!(store.n_items(), 0);
    }

    #[test]
    fn updates_a_changed_container_in_place() {
        let store = store_of(&[container("a", "one", "running")]);
        let before = list::row_at::<ContainerObject>(&store, 0);

        list::apply::<ContainerObject>(&store, &[container("a", "one", "exited")]);

        let after = list::row_at::<ContainerObject>(&store, 0);
        assert_eq!(after.state(), "exited");
        // Same GObject, not a replacement: the row kept its identity.
        assert_eq!(before, after);
    }

    #[test]
    fn keeps_the_same_row_object_when_nothing_changed() {
        let store = store_of(&[container("a", "one", "running")]);
        let before = list::row_at::<ContainerObject>(&store, 0);

        list::apply::<ContainerObject>(&store, &[container("a", "one", "running")]);

        assert_eq!(before, list::row_at::<ContainerObject>(&store, 0));
        assert!(before.matches(&container("a", "one", "running")));
    }

    #[test]
    fn reorders_without_rebuilding_rows() {
        let a = container("a", "one", "running");
        let b = container("b", "two", "exited");
        let store = store_of(&[a.clone(), b.clone()]);
        let row_a = list::row_at::<ContainerObject>(&store, 0);

        list::apply::<ContainerObject>(&store, &[b, a]);

        assert_eq!(ids(&store), ["b", "a"]);
        // "a" moved rather than being recreated.
        assert_eq!(row_a, list::row_at::<ContainerObject>(&store, 1));
    }

    #[test]
    fn handles_a_wholesale_replacement() {
        let store = store_of(&[container("a", "one", "running")]);
        list::apply::<ContainerObject>(
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
