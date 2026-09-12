//! The Compose page.
//!
//! A thin, read-only-by-default view over what M9's label grouping finds.
//! There is no "create project" here and never will be: Docklet reads
//! Compose's own bookkeeping, it does not replace `docker compose up`.
//!
//! Projects and their services share one flat `ColumnView`/`ListStore`: a
//! project's own row is always present, and its services' rows are spliced
//! in right after it while the project is expanded. Expansion is tracked
//! here (by project name) rather than through GTK's `TreeListModel`, because
//! every refresh already rebuilds rows from scratch by diffing against the
//! previous store — a plain `HashSet` surviving that diff is far simpler
//! than reconciling expansion state with a tree model on every refresh.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, ColumnView, ColumnViewColumn, DropDown, Label, ListItem,
    Orientation, PolicyType, ScrolledWindow, SignalListItemFactory, SingleSelection, StringList,
    Widget,
};

use super::banner::{Banner, StaleBanner};
use super::connection::ConnectionStatus;
use super::containers::{state_indicator, STATE_CLASSES};
use super::list::{self, text_column, Loading, Row};
use super::logpane::LogPane;
use super::object::{ComposeRow, ComposeRowObject};
use crate::docker::{
    ComposeProject, Docker, DockerError, LogEvent, ProjectActionResult, StreamHandle,
};

const LOG_DOMAIN: &str = "docklet";

impl Row for ComposeRowObject {
    type Data = ComposeRow;

    fn build(row: &ComposeRow) -> Self {
        ComposeRowObject::new(row)
    }

    fn key(&self) -> String {
        ComposeRowObject::key(self)
    }

    fn key_of(row: &ComposeRow) -> &str {
        row.key()
    }

    fn matches(&self, row: &ComposeRow) -> bool {
        ComposeRowObject::matches(self, row)
    }

    fn update(&self, row: &ComposeRow) {
        self.set(row);
    }
}

/// The Compose project list.
pub struct ComposePage {
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    loading: Loading,
    banner: Banner,
    /// A persistent notice, distinct from `banner`, shown while the list is
    /// known to be stale because the last refresh could not reach Docker.
    stale: StaleBanner,
    /// The app-wide Docker connection indicator, shared with every page.
    connection: Rc<ConnectionStatus>,
    refreshing: Rc<Cell<bool>>,
    /// A refresh asked for while one was already running — set when, say, a
    /// post-Start/Stop refresh lands while an auto-refresh tick is still in
    /// flight, so that request is not silently dropped: seeing this true
    /// once the in-flight refresh finishes is what triggers the follow-up
    /// that actually picks up the just-completed action's new state.
    refresh_again: Rc<Cell<bool>>,
    start_button: Button,
    stop_button: Button,
    logs_button: Button,
    log_view: Rc<ComposeLogView>,
    /// The last successful fetch, kept so toggling a project's expansion can
    /// rebuild the row list locally instead of re-reading Docker.
    projects: RefCell<Vec<ComposeProject>>,
    /// Names of the projects currently expanded, surviving across refreshes.
    expanded: RefCell<HashSet<String>>,
}

impl ComposePage {
    pub fn new(connection: Rc<ConnectionStatus>) -> Rc<Self> {
        let store = gio::ListStore::new::<ComposeRowObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&state_column());
        view.append_column(&text_column::<ComposeRowObject>(
            "Project / Service",
            ComposeRowObject::name,
        ));
        view.append_column(&text_column::<ComposeRowObject>(
            "Services / Container",
            ComposeRowObject::detail,
        ));
        view.append_column(&text_column::<ComposeRowObject>(
            "State",
            ComposeRowObject::state_label,
        ));
        view.append_column(&text_column::<ComposeRowObject>(
            "Working Directory",
            ComposeRowObject::working_dir,
        ));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = Label::builder()
            .label("No Compose projects found")
            .vexpand(true)
            .visible(false)
            .build();
        empty.add_css_class("dim-label");

        let banner = Banner::new();
        let stale = StaleBanner::new();

        let start_button = Button::with_label("Start");
        start_button.add_css_class("suggested-action");
        start_button.set_sensitive(false);
        start_button.set_tooltip_text(Some(
            "Starts this project's existing containers. Does not create missing ones.",
        ));

        let actions = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let stop_button = Button::with_label("Stop");
        stop_button.set_sensitive(false);
        let logs_button = Button::with_label("Logs");
        logs_button.set_sensitive(false);
        actions.append(&start_button);
        actions.append(&stop_button);
        actions.append(&logs_button);

        let loading = Loading::new();
        loading.widget().set_halign(gtk::Align::Center);
        loading.widget().set_valign(gtk::Align::Center);
        loading.widget().set_vexpand(true);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(stale.widget());
        root.append(&actions);
        root.append(loading.widget());
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(ComposePage {
            root,
            scrolled,
            empty,
            store,
            selection,
            loading,
            banner,
            stale,
            connection,
            refreshing: Rc::new(Cell::new(false)),
            refresh_again: Rc::new(Cell::new(false)),
            start_button,
            stop_button,
            logs_button,
            log_view: ComposeLogView::new(),
            projects: RefCell::new(Vec::new()),
            expanded: RefCell::new(HashSet::new()),
        });

        // Inserted after construction, since toggling a row needs a handle
        // back to the page itself.
        view.insert_column(0, &toggle_column(&page));

        page.start_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.start_selected();
                }
            }
        });
        page.stop_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.stop_selected();
                }
            }
        });
        page.selection.connect_selected_item_notify({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.sync_action_buttons();
                }
            }
        });

        page.root.append(page.log_view.widget());

        page.logs_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.open_logs();
                }
            }
        });
        page.log_view.connect_back({
            let page = Rc::downgrade(&page);
            move || {
                if let Some(page) = page.upgrade() {
                    page.close_logs();
                }
            }
        });

        page.refresh();
        page
    }

    /// Flip a project's expansion and rebuild the visible rows from the last
    /// fetch — no Docker round trip needed just to show what is already known.
    fn toggle_expanded(self: &Rc<Self>, name: &str) {
        {
            let mut expanded = self.expanded.borrow_mut();
            if !expanded.remove(name) {
                expanded.insert(name.to_string());
            }
        }
        self.rebuild_rows();
    }

    /// Flatten `self.projects` into rows — each project, followed by its
    /// services if expanded — and diff that into the store.
    fn rebuild_rows(&self) {
        let rows = flatten(&self.projects.borrow(), &self.expanded.borrow());

        list::apply::<ComposeRowObject>(&self.store, &rows);
        let is_empty = self.store.n_items() == 0;
        self.empty.set_visible(is_empty);
        self.scrolled.set_visible(!is_empty);
        self.sync_action_buttons();
    }

    /// Open the log view for the selected project's services.
    fn open_logs(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        if !row.is_project() {
            return;
        }
        let name = row.name();
        self.banner.clear();

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let looked_up = gio::spawn_blocking(move || -> Result<ComposeProject, DockerError> {
                let docker = Docker::connect()?;
                docker
                    .compose_projects()?
                    .into_iter()
                    .find(|p| p.name == name)
                    .ok_or_else(|| {
                        DockerError::Protocol("project is no longer present".to_string())
                    })
            })
            .await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match looked_up {
                Ok(Ok(project)) => {
                    page.scrolled.set_visible(false);
                    page.empty.set_visible(false);
                    page.log_view.show(&project);
                }
                Ok(Err(e)) => page.show_error(&format!("Could not open logs. {e}")),
                Err(_) => page.show_error("Could not open logs."),
            }
        });
    }

    fn close_logs(self: &Rc<Self>) {
        self.log_view.hide();
        let is_empty = self.store.n_items() == 0;
        self.empty.set_visible(is_empty);
        self.scrolled.set_visible(!is_empty);
    }

    /// Enable Start/Stop/Logs only when a project (not a service) is selected.
    fn sync_action_buttons(&self) {
        let row = self.selected();
        let ready = row.as_ref().is_some_and(|row| row.is_project());
        // A project already uniformly running has nothing left to start —
        // Partial (or any other state) still does.
        let already_running = row
            .as_ref()
            .is_some_and(|row| row.state_kind() == "running");

        self.start_button.set_sensitive(ready && !already_running);
        self.stop_button.set_sensitive(ready);
        self.logs_button.set_sensitive(ready);
    }

    /// Neither button while an action is running.
    fn sync_action_buttons_busy(&self) {
        self.start_button.set_sensitive(false);
        self.stop_button.set_sensitive(false);
    }

    /// Stop the selected project's running containers.
    fn stop_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        if !row.is_project() {
            return;
        }
        let name = row.name();
        self.banner.clear();
        self.sync_action_buttons_busy();

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let stopped = gio::spawn_blocking(
                move || -> Result<ProjectActionResult, crate::docker::DockerError> {
                    let docker = Docker::connect()?;
                    let projects = docker.compose_projects()?;
                    let project =
                        projects
                            .into_iter()
                            .find(|p| p.name == name)
                            .ok_or_else(|| {
                                crate::docker::DockerError::Protocol(
                                    "project is no longer present".to_string(),
                                )
                            })?;
                    Ok(docker.compose_stop(&project))
                },
            )
            .await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match stopped {
                Ok(Ok(result)) if result.is_success() => page.refresh(),
                Ok(Ok(result)) => {
                    let details: Vec<String> = result
                        .failed
                        .iter()
                        .map(|(service, error)| format!("{service}: {error}"))
                        .collect();
                    page.show_error(&format!(
                        "Some services could not be stopped. {}",
                        details.join("; ")
                    ));
                    page.refresh();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not stop the project. {e}")),
                Err(_) => page.show_error("Could not stop the project."),
            }
        });
    }

    /// Start the selected project's existing, stopped containers.
    fn start_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        if !row.is_project() {
            return;
        }
        let name = row.name();
        self.banner.clear();
        self.sync_action_buttons_busy();

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let started = gio::spawn_blocking(
                move || -> Result<ProjectActionResult, crate::docker::DockerError> {
                    let docker = Docker::connect()?;
                    let projects = docker.compose_projects()?;
                    let project =
                        projects
                            .into_iter()
                            .find(|p| p.name == name)
                            .ok_or_else(|| {
                                crate::docker::DockerError::Protocol(
                                    "project is no longer present".to_string(),
                                )
                            })?;
                    Ok(docker.compose_start(&project))
                },
            )
            .await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match started {
                Ok(Ok(result)) if result.is_success() => page.refresh(),
                Ok(Ok(result)) => {
                    let details: Vec<String> = result
                        .failed
                        .iter()
                        .map(|(service, error)| format!("{service}: {error}"))
                        .collect();
                    page.show_error(&format!(
                        "Some services could not be started. {}",
                        details.join("; ")
                    ));
                    page.refresh();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not start the project. {e}")),
                Err(_) => page.show_error("Could not start the project."),
            }
        });
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    fn selected(&self) -> Option<ComposeRowObject> {
        self.selection
            .selected_item()
            .and_downcast::<ComposeRowObject>()
    }

    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    /// Reload the project list from Docker.
    ///
    /// If a refresh is already running, this one is queued rather than
    /// dropped: it may have been asked for by something (a Start/Stop that
    /// just completed) which happened after whatever the in-flight refresh
    /// read, and skipping it would leave the list showing that older state
    /// indefinitely — not just until the next refresh, but never, if nothing
    /// else prompts one.
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            self.refresh_again.set(true);
            return;
        }

        let page = Rc::downgrade(self);
        if self.loading.start() {
            self.scrolled.set_visible(false);
            self.empty.set_visible(false);
        }

        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.compose_projects()).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            page.loading.finish();
            match listed {
                Ok(Ok(projects)) => {
                    page.projects.replace(projects);
                    page.rebuild_rows();
                    page.stale.clear();
                    page.connection.report_ok();
                }
                Ok(Err(e)) => {
                    page.show_error(&format!("Could not list Compose projects. {e}"));
                    if e.is_connection_error() {
                        page.connection.report_error(&e);
                        if page.store.n_items() > 0 {
                            page.stale.mark();
                        }
                    }
                }
                Err(_) => page.show_error("Could not list Compose projects."),
            }
            page.refreshing.set(false);
            if page.refresh_again.replace(false) {
                page.refresh();
            }
        });
    }
}

/// Flatten a set of projects into display rows: each project, immediately
/// followed by its services if it is in `expanded`.
///
/// A pure function so the tree-flattening logic is testable without a
/// display — the same reasoning that keeps `group_projects` in
/// `docker::compose` independently testable.
fn flatten(projects: &[ComposeProject], expanded: &HashSet<String>) -> Vec<ComposeRow> {
    let mut rows = Vec::new();
    for project in projects {
        let is_expanded = expanded.contains(&project.name);
        rows.push(ComposeRow::project(project.clone(), is_expanded));
        if is_expanded {
            for service in &project.services {
                rows.push(ComposeRow::service(service.clone()));
            }
        }
    }
    rows
}

/// The leading column: an expand/collapse toggle for project rows, blank for
/// service rows.
///
/// A fresh button is built on every bind rather than one reused across
/// recycled rows: Compose lists are small, and rebuilding here avoids having
/// to track and disconnect a stale click handler from whatever row a
/// recycled widget last belonged to.
fn toggle_column(page: &Rc<ComposePage>) -> ColumnViewColumn {
    let factory = SignalListItemFactory::new();

    factory.connect_bind({
        let page = Rc::downgrade(page);
        move |_, item| {
            let item = item.downcast_ref::<ListItem>().expect("list item");
            let Some(object) = item.item().and_downcast::<ComposeRowObject>() else {
                return;
            };

            if !object.is_project() {
                item.set_child(None::<&Widget>);
                return;
            }

            let icon = if object.expanded() {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            };
            let button = Button::from_icon_name(icon);
            button.add_css_class("flat");

            let name = object.name();
            button.connect_clicked({
                let page = page.clone();
                move |_| {
                    if let Some(page) = page.upgrade() {
                        page.toggle_expanded(&name);
                    }
                }
            });
            item.set_child(Some(&button));
        }
    });

    ColumnViewColumn::builder().factory(&factory).build()
}

/// The state column: a bullet reusing the container page's own state colours,
/// plus `state-partial` for a project whose services disagree.
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
        let Some(object) = item.item().and_downcast::<ComposeRowObject>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<Label>() else {
            return;
        };

        let kind = object.state_kind();
        let (bullet, class) = if kind == "partial" {
            ("\u{25d0}", "state-partial")
        } else {
            state_indicator(&kind)
        };
        label.set_text(bullet);
        label.set_tooltip_text(Some(&object.state_label()));

        // Rows are recycled, so drop whatever the previous row left.
        for stale in STATE_CLASSES {
            label.remove_css_class(stale);
        }
        label.remove_css_class("state-partial");
        label.add_css_class(class);
    });

    ColumnViewColumn::builder().factory(&factory).build()
}

/// A per-service log view for one Compose project.
///
/// Only one container is ever followed at a time — picking a different
/// service tears down the previous stream before starting the next one,
/// the same demand-driven rule the container detail pane follows.
struct ComposeLogView {
    root: GtkBox,
    back: Button,
    title: Label,
    picker: DropDown,
    log_pane: LogPane,
    services: RefCell<Vec<(String, String, bool)>>, // (name, container_id, tty)
    follow: RefCell<Option<StreamHandle>>,
}

impl ComposeLogView {
    fn new() -> Rc<Self> {
        let back = Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Back to projects"));

        let title = Label::builder()
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        title.add_css_class("heading");

        let picker = DropDown::builder().model(&StringList::new(&[])).build();

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
        header.append(&picker);

        let log_pane = LogPane::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(log_pane.widget());
        root.set_visible(false);

        let view = Rc::new(ComposeLogView {
            root,
            back,
            title,
            picker,
            log_pane,
            services: RefCell::new(Vec::new()),
            follow: RefCell::new(None),
        });

        view.picker.connect_selected_notify({
            let view = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = view.upgrade() {
                    view.load_selected();
                }
            }
        });
        view.log_pane.connect_refresh({
            let view = Rc::downgrade(&view);
            move || {
                if let Some(view) = view.upgrade() {
                    view.load_selected();
                }
            }
        });
        view.log_pane.connect_follow({
            let view = Rc::downgrade(&view);
            move |following| {
                if let Some(view) = view.upgrade() {
                    if following {
                        view.start_follow();
                    } else {
                        view.stop_follow();
                    }
                }
            }
        });

        view
    }

    fn widget(&self) -> &gtk::Widget {
        self.root.upcast_ref()
    }

    fn connect_back(&self, handler: impl Fn() + 'static) {
        self.back.connect_clicked(move |_| handler());
    }

    fn hide(&self) {
        self.stop_follow();
        self.root.set_visible(false);
    }

    /// Populate the picker for a project and load the first service's logs.
    ///
    /// A container's TTY flag decides how its logs are framed; rather than an
    /// inspect call per service up front, `tty` is looked up lazily and
    /// cached the first time each service is actually selected.
    fn show(self: &Rc<Self>, project: &ComposeProject) {
        self.title.set_text(&project.name);
        self.log_pane.set_following(false);
        self.stop_follow();

        let names: Vec<&str> = project.services.iter().map(|s| s.name.as_str()).collect();
        self.picker.set_model(Some(&StringList::new(&names)));
        *self.services.borrow_mut() = project
            .services
            .iter()
            .map(|s| (s.name.clone(), s.container_id.clone(), false))
            .collect();

        self.root.set_visible(true);
        if !project.services.is_empty() {
            self.picker.set_selected(0);
            // `set_selected(0)` does not notify when the picker is already at
            // index 0 (its default), so the first load is kicked off directly.
            self.load_selected();
        }
    }

    fn selected_container(&self) -> Option<(String, String)> {
        let index = self.picker.selected() as usize;
        self.services
            .borrow()
            .get(index)
            .map(|(_, id, _)| (id.clone(), id.clone()))
    }

    /// Fetch the tail of the selected service's logs.
    fn load_selected(self: &Rc<Self>) {
        self.stop_follow();
        self.log_pane.set_following(false);

        let Some((id, _)) = self.selected_container() else {
            return;
        };
        let view = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let id_for_tty = id.clone();
            let fetched = gio::spawn_blocking(move || -> Result<(bool, String), DockerError> {
                let docker = Docker::connect()?;
                let tty = docker.inspect_container(&id_for_tty)?.config.tty;
                let text =
                    docker.container_logs(&id_for_tty, tty, crate::docker::DEFAULT_LOG_TAIL)?;
                Ok((tty, text))
            })
            .await;

            let Some(view) = view.upgrade() else {
                return;
            };
            match fetched {
                Ok(Ok((tty, text))) => {
                    view.remember_tty(&id, tty);
                    view.log_pane.set_logs(&text);
                }
                Ok(Err(e)) => {
                    glib::g_warning!(LOG_DOMAIN, "could not read service logs: {e}");
                    view.log_pane.set_logs(&format!("Could not read logs. {e}"));
                }
                Err(_) => view.log_pane.set_logs("Could not read logs."),
            }
        });
    }

    fn remember_tty(&self, id: &str, tty: bool) {
        if let Some(entry) = self
            .services
            .borrow_mut()
            .iter_mut()
            .find(|(_, cid, _)| cid == id)
        {
            entry.2 = tty;
        }
    }

    fn start_follow(self: &Rc<Self>) {
        let Some((id, _)) = self.selected_container() else {
            return;
        };
        let tty = self
            .services
            .borrow()
            .iter()
            .find(|(_, cid, _)| *cid == id)
            .map(|(_, _, tty)| *tty)
            .unwrap_or(false);

        let (sender, receiver) = async_channel::bounded::<LogEvent>(64);
        let handle = match Docker::connect() {
            Ok(docker) => docker.follow_logs(&id, tty, crate::docker::DEFAULT_LOG_TAIL, sender),
            Err(_) => return,
        };
        self.follow.replace(Some(handle));
        self.log_pane.set_logs("");

        let view = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = receiver.recv().await {
                let Some(view) = view.upgrade() else {
                    return;
                };
                match event {
                    LogEvent::Text(text) => view.log_pane.append_logs(&text),
                    LogEvent::Failed(message) => {
                        glib::g_warning!(LOG_DOMAIN, "log stream ended: {message}");
                        view.stop_follow();
                        view.log_pane.set_following(false);
                        return;
                    }
                }
            }
        });
    }

    fn stop_follow(&self) {
        self.follow.replace(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::ComposeService;

    fn service(name: &str, id: &str, state: &str) -> ComposeService {
        ComposeService {
            name: name.to_string(),
            container_id: id.to_string(),
            container_name: format!("app-{name}-1"),
            state: state.to_string(),
        }
    }

    fn project(name: &str, services: Vec<ComposeService>) -> ComposeProject {
        ComposeProject {
            name: name.to_string(),
            services,
            working_dir: format!("/home/user/{name}"),
            config_files: String::new(),
        }
    }

    fn expanded_of(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn collapsed_projects_show_only_their_own_row() {
        let projects = vec![project(
            "app",
            vec![
                service("web", "a", "running"),
                service("db", "b", "running"),
            ],
        )];
        let rows = flatten(&projects, &HashSet::new());
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn expanding_a_project_inserts_its_services_right_after_it() {
        let projects = vec![
            project("app", vec![service("web", "a", "running")]),
            project("other", vec![service("cache", "c", "running")]),
        ];
        let rows = flatten(&projects, &expanded_of(&["app"]));

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].key(), "p:app");
        assert_eq!(rows[1].key(), "s:a");
        assert_eq!(rows[2].key(), "p:other");
    }

    #[test]
    fn expanding_every_project_shows_every_service() {
        let projects = vec![
            project("app", vec![service("web", "a", "running")]),
            project("other", vec![service("cache", "c", "running")]),
        ];
        let rows = flatten(&projects, &expanded_of(&["app", "other"]));
        assert_eq!(rows.len(), 4);
    }

    fn store_of(rows: &[ComposeRow]) -> gio::ListStore {
        let store = gio::ListStore::new::<ComposeRowObject>();
        list::apply::<ComposeRowObject>(&store, rows);
        store
    }

    #[test]
    fn a_project_row_reports_its_uniform_state() {
        let projects = vec![project("app", vec![service("web", "a", "running")])];
        let store = store_of(&flatten(&projects, &HashSet::new()));

        let row = list::row_at::<ComposeRowObject>(&store, 0);
        assert!(row.is_project());
        assert_eq!(row.state_kind(), "running");
        assert_eq!(row.state_label(), "Running");
    }

    #[test]
    fn a_project_with_disagreeing_services_reports_partial() {
        let projects = vec![project(
            "app",
            vec![service("web", "a", "running"), service("db", "b", "exited")],
        )];
        let store = store_of(&flatten(&projects, &HashSet::new()));

        let row = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(row.state_kind(), "partial");
        assert_eq!(row.state_label(), "Partial");
    }

    #[test]
    fn a_service_row_shows_its_own_container_state_and_name() {
        let projects = vec![project("app", vec![service("web", "a", "restarting")])];
        let store = store_of(&flatten(&projects, &expanded_of(&["app"])));

        let row = list::row_at::<ComposeRowObject>(&store, 1);
        assert!(!row.is_project());
        assert_eq!(row.state_kind(), "restarting");
        assert_eq!(row.detail(), "app-web-1");
        // Indented so it reads as nested under its project.
        assert!(row.name().starts_with(' '));
        assert!(row.name().trim() == "web");
    }

    /// Regression test: the project's own row must be re-bound — not left
    /// showing an old aggregate state — whenever the underlying services'
    /// states change, even though the project's name, service count and
    /// working directory all stay the same. `matches()` used to compare only
    /// those three, so a project flipping from Running to Stopped kept
    /// displaying "Running" until something else about it happened to change.
    #[test]
    fn a_project_row_is_rebuilt_when_only_its_aggregate_state_changes() {
        let name = "app";
        let store = store_of(&flatten(
            &[project(name, vec![service("web", "a", "running")])],
            &HashSet::new(),
        ));
        let before = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(before.state_kind(), "running");

        list::apply::<ComposeRowObject>(
            &store,
            &flatten(
                &[project(name, vec![service("web", "a", "exited")])],
                &HashSet::new(),
            ),
        );

        let after = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(after.state_kind(), "exited");
        assert_eq!(after.state_label(), "Exited");
        // Same GObject, not a replacement: the row kept its identity.
        assert_eq!(before, after);
    }

    /// The same regression, but with the project expanded — its row and its
    /// services' rows are diffed together in one `list::apply` call, which
    /// is what a real Start/Stop's follow-up refresh does whenever the user
    /// happens to have the project open. Covers the exact "services updated
    /// to Exited but the project stayed Running" bug report: a project row
    /// bug that only the collapsed-only test above could not have caught if
    /// the two row kinds interfered with each other while diffing.
    #[test]
    fn an_expanded_projects_row_still_updates_when_stopped() {
        let name = "app";
        let expanded = expanded_of(&[name]);
        let store = store_of(&flatten(
            &[project(
                name,
                vec![
                    service("web", "a", "running"),
                    service("db", "b", "running"),
                ],
            )],
            &expanded,
        ));
        assert_eq!(store.n_items(), 3);
        assert_eq!(
            list::row_at::<ComposeRowObject>(&store, 0).state_label(),
            "Running"
        );

        // Both services stop — exactly what `compose_stop` plus a refresh
        // produces.
        list::apply::<ComposeRowObject>(
            &store,
            &flatten(
                &[project(
                    name,
                    vec![service("web", "a", "exited"), service("db", "b", "exited")],
                )],
                &expanded,
            ),
        );

        assert_eq!(store.n_items(), 3);
        let project_row = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(project_row.state_kind(), "exited");
        assert_eq!(project_row.state_label(), "Exited");
        assert_eq!(
            list::row_at::<ComposeRowObject>(&store, 1).state_kind(),
            "exited"
        );
        assert_eq!(
            list::row_at::<ComposeRowObject>(&store, 2).state_kind(),
            "exited"
        );
    }

    /// A refresh after only one of two services stops — the project must
    /// show Partial, not stay on whatever it showed before.
    #[test]
    fn a_project_shows_partial_after_an_external_stop_of_one_service() {
        let name = "app";
        let store = store_of(&flatten(
            &[project(
                name,
                vec![
                    service("web", "a", "running"),
                    service("db", "b", "running"),
                ],
            )],
            &HashSet::new(),
        ));
        assert_eq!(
            list::row_at::<ComposeRowObject>(&store, 0).state_label(),
            "Running"
        );

        list::apply::<ComposeRowObject>(
            &store,
            &flatten(
                &[project(
                    name,
                    vec![service("web", "a", "running"), service("db", "b", "exited")],
                )],
                &HashSet::new(),
            ),
        );

        let row = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(row.state_kind(), "partial");
        assert_eq!(row.state_label(), "Partial");
    }

    #[test]
    fn toggling_expansion_keeps_the_project_rows_identity() {
        let projects = vec![project("app", vec![service("web", "a", "running")])];
        let store = store_of(&flatten(&projects, &HashSet::new()));
        let collapsed = list::row_at::<ComposeRowObject>(&store, 0);

        list::apply::<ComposeRowObject>(&store, &flatten(&projects, &expanded_of(&["app"])));

        assert_eq!(store.n_items(), 2);
        let expanded = list::row_at::<ComposeRowObject>(&store, 0);
        assert_eq!(collapsed, expanded);
        assert!(expanded.expanded());
    }

    #[test]
    fn collapsing_again_removes_the_service_row() {
        let projects = vec![project("app", vec![service("web", "a", "running")])];
        let store = store_of(&flatten(&projects, &expanded_of(&["app"])));
        assert_eq!(store.n_items(), 2);

        list::apply::<ComposeRowObject>(&store, &flatten(&projects, &HashSet::new()));

        assert_eq!(store.n_items(), 1);
        assert!(list::row_at::<ComposeRowObject>(&store, 0).is_project());
    }
}
