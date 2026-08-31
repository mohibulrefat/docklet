//! The containers page.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, ColumnView, ColumnViewColumn, Label, ListItem, PolicyType, ScrolledWindow,
    SignalListItemFactory, SingleSelection, Widget,
};

use super::object::ContainerObject;
use crate::docker::Docker;

const LOG_DOMAIN: &str = "docklet";

/// The containers list, and the store backing it.
pub struct ContainersPage {
    root: ScrolledWindow,
    store: gio::ListStore,
}

impl ContainersPage {
    pub fn new() -> Self {
        let store = gio::ListStore::new::<ContainerObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
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
