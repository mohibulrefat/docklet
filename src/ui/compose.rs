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
    Box as GtkBox, ColumnView, Label, Orientation, PolicyType, ScrolledWindow, SingleSelection,
    Widget,
};

use super::banner::Banner;
use super::list::{self, text_column, Row};
use super::object::ComposeObject;
use crate::docker::{ComposeProject, Docker};

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

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
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
        });

        page.refresh();
        page
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    #[allow(dead_code)]
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
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list Compose projects. {e}")),
                Err(_) => page.show_error("Could not list Compose projects."),
            }
            page.refreshing.set(false);
        });
    }
}
