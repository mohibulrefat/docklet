//! The networks page.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, ColumnView, Entry, Label, Orientation, PolicyType, ScrolledWindow,
    SingleSelection, Widget, Window,
};

use super::banner::Banner;
use super::dialog::confirm;
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
    name_entry: Entry,
    driver_entry: Entry,
    remove_button: Button,
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

        let name_entry = Entry::builder()
            .placeholder_text("network name")
            .hexpand(true)
            .build();
        let driver_entry = Entry::builder()
            .placeholder_text("driver (bridge)")
            .width_chars(14)
            .build();
        let create_button = Button::with_label("Create");
        create_button.add_css_class("suggested-action");

        let remove_button = Button::with_label("Remove");
        remove_button.add_css_class("destructive-action");
        remove_button.set_sensitive(false);

        let actions = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        actions.append(&name_entry);
        actions.append(&driver_entry);
        actions.append(&create_button);
        actions.append(&remove_button);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
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
            name_entry,
            driver_entry,
            remove_button,
        });

        create_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.create_network();
                }
            }
        });
        page.name_entry.connect_activate({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.create_network();
                }
            }
        });
        page.remove_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.remove_selected();
                }
            }
        });
        page.selection.connect_selected_item_notify({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.sync_remove();
                }
            }
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

    /// Removing needs a selection, and Docker's own networks cannot go.
    fn sync_remove(&self) {
        let removable = self
            .selected()
            .is_some_and(|network| !network.is_predefined());
        self.remove_button.set_sensitive(removable);
    }

    fn show_error(&self, message: &str) {
        glib::g_warning!(LOG_DOMAIN, "{message}");
        self.banner.show(message);
    }

    /// Create a network from the entries.
    fn create_network(self: &Rc<Self>) {
        let name = self.name_entry.text().trim().to_string();
        if name.is_empty() {
            self.show_error("Enter a name for the network.");
            return;
        }
        let driver = self.driver_entry.text().trim().to_string();

        self.banner.clear();
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let created = {
                let (name, driver) = (name.clone(), driver.clone());
                gio::spawn_blocking(move || Docker::connect()?.create_network(&name, &driver)).await
            };

            let Some(page) = page.upgrade() else {
                return;
            };
            match created {
                Ok(Ok(())) => {
                    page.name_entry.set_text("");
                    page.driver_entry.set_text("");
                    page.refresh();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not create {name}. {e}")),
                Err(_) => page.show_error(&format!("Could not create {name}.")),
            }
        });
    }

    /// Remove the selected network, after confirming.
    fn remove_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        if row.is_predefined() {
            return;
        }
        let (id, name) = (row.id(), row.name());
        let parent = self.root.root().and_downcast::<Window>();
        self.banner.clear();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let confirmed = confirm(
                parent.as_ref(),
                &format!("Remove network {name}?"),
                "Containers still attached to it will lose this network.",
                "Remove",
            )
            .await;
            if !confirmed {
                return;
            }

            let removed = gio::spawn_blocking(move || Docker::connect()?.remove_network(&id)).await;
            let Some(page) = page.upgrade() else {
                return;
            };
            match removed {
                Ok(Ok(())) => page.refresh(),
                Ok(Err(e)) => page.show_error(&format!("Could not remove {name}. {e}")),
                Err(_) => page.show_error(&format!("Could not remove {name}.")),
            }
        });
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
                    page.sync_remove();
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list networks. {e}")),
                Err(_) => page.show_error("Could not list networks."),
            }
            page.refreshing.set(false);
        });
    }
}
