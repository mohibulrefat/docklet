//! The volumes page.

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
use super::list::{self, mono_column, text_column, Row};
use super::object::VolumeObject;
use crate::docker::{Docker, Volume};

const LOG_DOMAIN: &str = "docklet";

impl Row for VolumeObject {
    type Data = Volume;

    fn build(volume: &Volume) -> Self {
        VolumeObject::new(volume)
    }

    fn key(&self) -> String {
        self.name()
    }

    fn key_of(volume: &Volume) -> &str {
        &volume.name
    }

    fn matches(&self, volume: &Volume) -> bool {
        self.name() == volume.name
            && self.driver() == volume.driver
            && self.mountpoint() == volume.mountpoint
    }

    fn update(&self, volume: &Volume) {
        self.set(volume);
    }
}

/// The volume list.
pub struct VolumesPage {
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    banner: Banner,
    refreshing: Rc<Cell<bool>>,
}

impl VolumesPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<VolumeObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&text_column::<VolumeObject>("Name", VolumeObject::name));
        view.append_column(&text_column::<VolumeObject>("Driver", VolumeObject::driver));
        view.append_column(&mono_column::<VolumeObject>(
            "Mountpoint",
            VolumeObject::mountpoint,
        ));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = Label::builder()
            .label("No volumes")
            .vexpand(true)
            .visible(false)
            .build();
        empty.add_css_class("dim-label");

        let banner = Banner::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(VolumesPage {
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
    fn selected(&self) -> Option<VolumeObject> {
        self.selection
            .selected_item()
            .and_downcast::<VolumeObject>()
    }

    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    /// Reload the list from Docker.
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            return;
        }

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.volumes()).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match listed {
                Ok(Ok(volumes)) => {
                    list::apply::<VolumeObject>(&page.store, &volumes);
                    let is_empty = page.store.n_items() == 0;
                    page.empty.set_visible(is_empty);
                    page.scrolled.set_visible(!is_empty);
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list volumes. {e}")),
                Err(_) => page.show_error("Could not list volumes."),
            }
            page.refreshing.set(false);
        });
    }
}
