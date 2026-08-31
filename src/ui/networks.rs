//! The networks page.

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
use super::object::NetworkObject;
use crate::docker::{Docker, Network};

const LOG_DOMAIN: &str = "docklet";

impl Row for NetworkObject {
    type Data = Network;

    fn build(network: &Network) -> Self {
        NetworkObject::new(network)
    }

    fn key(&self) -> String {
        self.id()
    }

    fn key_of(network: &Network) -> &str {
        &network.id
    }

    fn matches(&self, network: &Network) -> bool {
        self.id() == network.id
            && self.name() == network.name
            && self.driver() == network.driver
            && self.scope() == network.scope
    }

    fn update(&self, network: &Network) {
        self.set(network);
    }
}

/// The network list.
pub struct NetworksPage {
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    banner: Banner,
    refreshing: Rc<Cell<bool>>,
}

impl NetworksPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<NetworkObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&text_column::<NetworkObject>("Name", NetworkObject::name));
        view.append_column(&text_column::<NetworkObject>(
            "Driver",
            NetworkObject::driver,
        ));
        view.append_column(&text_column::<NetworkObject>("Scope", NetworkObject::scope));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = Label::builder()
            .label("No networks")
            .vexpand(true)
            .visible(false)
            .build();
        empty.add_css_class("dim-label");

        let banner = Banner::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(NetworksPage {
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

    fn selected(&self) -> Option<NetworkObject> {
        self.selection
            .selected_item()
            .and_downcast::<NetworkObject>()
    }

    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    fn sync_list_visibility(&self) {
        let is_empty = self.store.n_items() == 0;
        self.empty.set_visible(is_empty);
        self.scrolled.set_visible(!is_empty);
    }

    /// Reload the list from Docker.
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            return;
        }

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.networks()).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match listed {
                Ok(Ok(networks)) => {
                    list::apply::<NetworkObject>(&page.store, &networks);
                    page.sync_list_visibility();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list networks. {e}")),
                Err(_) => page.show_error("Could not list networks."),
            }
            page.refreshing.set(false);
        });
    }
}
