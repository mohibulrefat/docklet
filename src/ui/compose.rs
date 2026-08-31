//! The Compose page.
//!
//! A thin, read-only-by-default view over what M9's label grouping finds.
//! There is no "create project" here and never will be: Docklet reads
//! Compose's own bookkeeping, it does not replace `docker compose up`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, ColumnView, Label, Orientation, PolicyType, ScrolledWindow,
    SingleSelection, Widget,
};

use super::banner::Banner;
use super::list::{self, text_column, Row};
use super::object::ComposeObject;
use crate::docker::{ComposeProject, Docker, ProjectActionResult};

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
    banner: Banner,
    refreshing: Rc<Cell<bool>>,
    start_button: Button,
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
        actions.append(&start_button);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(ComposePage {
            root,
            scrolled,
            empty,
            store,
            selection,
            banner,
            refreshing: Rc::new(Cell::new(false)),
            start_button,
        });

        page.start_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.start_selected();
                }
            }
        });
        page.selection.connect_selected_item_notify({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.start_button.set_sensitive(page.selected().is_some());
                }
            }
        });

        page.refresh();
        page
    }

    /// Start the selected project's existing, stopped containers.
    fn start_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let name = row.name();
        self.banner.clear();
        self.start_button.set_sensitive(false);

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
        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.compose_projects()).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match listed {
                Ok(Ok(projects)) => {
                    list::apply::<ComposeObject>(&page.store, &projects);
                    let is_empty = page.store.n_items() == 0;
                    page.empty.set_visible(is_empty);
                    page.scrolled.set_visible(!is_empty);
                    page.start_button.set_sensitive(page.selected().is_some());
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list Compose projects. {e}")),
                Err(_) => page.show_error("Could not list Compose projects."),
            }
            page.refreshing.set(false);
        });
    }
}
