//! The Compose page.
//!
//! A thin, read-only-by-default view over what M9's label grouping finds.
//! There is no "create project" here and never will be: Docklet reads
//! Compose's own bookkeeping, it does not replace `docker compose up`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, ColumnView, DropDown, Label, Orientation, PolicyType, ScrolledWindow,
    SingleSelection, StringList, Widget,
};

use super::banner::Banner;
use super::list::{self, text_column, Loading, Row};
use super::logpane::LogPane;
use super::object::ComposeObject;
use crate::docker::{
    ComposeProject, Docker, DockerError, LogEvent, ProjectActionResult, StreamHandle,
};

const LOG_DOMAIN: &str = "docklet";

impl Row for ComposeObject {
    type Data = ComposeProject;

    fn build(project: &ComposeProject) -> Self {
        ComposeObject::new(project)
    }

    fn key(&self) -> String {
        self.name()
    }

    fn key_of(project: &ComposeProject) -> &str {
        &project.name
    }

    fn matches(&self, project: &ComposeProject) -> bool {
        self.name() == project.name
            && self.services()
                == format!(
                    "{} service{}",
                    project.service_count(),
                    if project.service_count() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            && self.working_dir() == project.working_dir
    }

    fn update(&self, project: &ComposeProject) {
        self.set(project);
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
    refreshing: Rc<Cell<bool>>,
    start_button: Button,
    stop_button: Button,
    logs_button: Button,
    log_view: Rc<ComposeLogView>,
}

impl ComposePage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<ComposeObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&text_column::<ComposeObject>(
            "Project",
            ComposeObject::name,
        ));
        view.append_column(&text_column::<ComposeObject>(
            "Services",
            ComposeObject::services,
        ));
        view.append_column(&text_column::<ComposeObject>("State", ComposeObject::state));
        view.append_column(&text_column::<ComposeObject>(
            "Working Directory",
            ComposeObject::working_dir,
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
            refreshing: Rc::new(Cell::new(false)),
            start_button,
            stop_button,
            logs_button,
            log_view: ComposeLogView::new(),
        });

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

    /// Open the log view for the selected project's services.
    fn open_logs(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
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

    /// Enable Start/Stop/Logs only when a project is selected.
    fn sync_action_buttons(&self) {
        let ready = self.selected().is_some();
        self.start_button.set_sensitive(ready);
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

    fn selected(&self) -> Option<ComposeObject> {
        self.selection
            .selected_item()
            .and_downcast::<ComposeObject>()
    }

    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    /// Reload the project list from Docker.
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
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
                    list::apply::<ComposeObject>(&page.store, &projects);
                    let is_empty = page.store.n_items() == 0;
                    page.empty.set_visible(is_empty);
                    page.scrolled.set_visible(!is_empty);
                    page.sync_action_buttons();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list Compose projects. {e}")),
                Err(_) => page.show_error("Could not list Compose projects."),
            }
            page.refreshing.set(false);
        });
    }
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
