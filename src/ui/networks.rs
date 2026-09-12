//! The networks page.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, ColumnView, Entry, Grid, Label, Orientation, PolicyType,
    ScrolledWindow, Separator, SingleSelection, Widget, Window,
};

use super::banner::{Banner, StaleBanner};
use super::connection::ConnectionStatus;
use super::detail::field;
use super::dialog::confirm;
use super::list::{self, text_column, Loading, Row};
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

/// A read-only view of one network.
struct NetworkDetail {
    root: GtkBox,
    back: Button,
    title: Label,
    driver: Label,
    scope: Label,
    internal: Label,
    subnet: Label,
    gateway: Label,
    created: Label,
    containers: Label,
}

impl NetworkDetail {
    fn new() -> Self {
        let back = Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Back to networks"));

        let title = Label::builder().halign(Align::Start).build();
        title.add_css_class("heading");

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

        let grid = Grid::builder()
            .row_spacing(6)
            .column_spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let driver = field(&grid, 0, "Driver", false);
        let scope = field(&grid, 1, "Scope", false);
        let internal = field(&grid, 2, "Internal", false);
        let subnet = field(&grid, 3, "Subnet", true);
        let gateway = field(&grid, 4, "Gateway", true);
        let created = field(&grid, 5, "Created", false);
        let containers = field(&grid, 6, "Containers", false);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&grid)
            .build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&header);
        root.append(&Separator::new(Orientation::Horizontal));
        root.append(&scrolled);
        root.set_visible(false);

        NetworkDetail {
            root,
            back,
            title,
            driver,
            scope,
            internal,
            subnet,
            gateway,
            created,
            containers,
        }
    }

    fn show(&self, name: &str) {
        self.title.set_text(name);
        for label in [
            &self.driver,
            &self.scope,
            &self.internal,
            &self.subnet,
            &self.gateway,
            &self.created,
            &self.containers,
        ] {
            label.set_text("");
        }
    }

    fn set_network(&self, network: &Network) {
        self.driver.set_text(&network.driver);
        self.scope.set_text(&network.scope);
        self.internal
            .set_text(if network.internal { "Yes" } else { "No" });
        self.subnet.set_text(&network.subnets());
        self.gateway.set_text(&network.gateways());
        self.created.set_text(&network.created);

        let connected = network.connected();
        self.containers.set_text(if connected.is_empty() {
            "No containers"
        } else {
            &connected
        });
    }
}

/// The network list.
pub struct NetworksPage {
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    loading: Loading,
    banner: Banner,
    stale: StaleBanner,
    connection: Rc<ConnectionStatus>,
    refreshing: Rc<Cell<bool>>,
    name_entry: Entry,
    driver_entry: Entry,
    create_button: Button,
    remove_button: Button,
    detail: NetworkDetail,
}

impl NetworksPage {
    pub fn new(connection: Rc<ConnectionStatus>) -> Rc<Self> {
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
        let stale = StaleBanner::new();

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

        let detail = NetworkDetail::new();

        let loading = Loading::new();
        loading.widget().set_halign(Align::Center);
        loading.widget().set_valign(Align::Center);
        loading.widget().set_vexpand(true);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(stale.widget());
        root.append(&actions);
        root.append(loading.widget());
        root.append(&scrolled);
        root.append(&empty);
        root.append(&detail.root);

        let page = Rc::new(NetworksPage {
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
            name_entry,
            driver_entry,
            create_button,
            remove_button,
            detail,
        });

        page.create_button.connect_clicked({
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
        view.connect_activate({
            let page = Rc::downgrade(&page);
            move |_, _| {
                if let Some(page) = page.upgrade() {
                    page.open_detail();
                }
            }
        });
        page.detail.back.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.close_detail();
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
        self.create_button.set_sensitive(false);
        self.name_entry.set_sensitive(false);
        self.driver_entry.set_sensitive(false);

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let created = {
                let (name, driver) = (name.clone(), driver.clone());
                gio::spawn_blocking(move || Docker::connect()?.create_network(&name, &driver)).await
            };

            let Some(page) = page.upgrade() else {
                return;
            };
            page.create_button.set_sensitive(true);
            page.name_entry.set_sensitive(true);
            page.driver_entry.set_sensitive(true);

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

    /// Show the detail pane for the selected network.
    fn open_detail(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let name = row.name();
        self.detail.show(&name);
        self.scrolled.set_visible(false);
        self.empty.set_visible(false);
        self.detail.root.set_visible(true);

        let id = row.id();
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // Only the single-network endpoint reports connected containers.
            let inspected =
                gio::spawn_blocking(move || Docker::connect()?.inspect_network(&id)).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match inspected {
                Ok(Ok(network)) => page.detail.set_network(&network),
                Ok(Err(e)) => page.show_error(&format!("Could not inspect {name}. {e}")),
                Err(_) => page.show_error(&format!("Could not inspect {name}.")),
            }
        });
    }

    /// Return to the list.
    fn close_detail(self: &Rc<Self>) {
        self.detail.root.set_visible(false);
        self.sync_list_visibility();
    }

    fn showing_detail(&self) -> bool {
        self.detail.root.is_visible()
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
        if self.loading.start() {
            self.scrolled.set_visible(false);
            self.empty.set_visible(false);
        }

        glib::spawn_future_local(async move {
            let listed = gio::spawn_blocking(|| Docker::connect()?.networks()).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            page.loading.finish();
            match listed {
                Ok(Ok(networks)) => {
                    list::apply::<NetworkObject>(&page.store, &networks);
                    if !page.showing_detail() {
                        page.sync_list_visibility();
                    }
                    page.sync_remove();
                    page.stale.clear();
                    page.connection.report_ok();
                }
                Ok(Err(e)) => {
                    page.show_error(&format!("Could not list networks. {e}"));
                    if e.is_connection_error() {
                        page.connection.report_error(&e);
                        if page.store.n_items() > 0 {
                            page.stale.mark();
                        }
                    }
                }
                Err(_) => page.show_error("Could not list networks."),
            }
            page.refreshing.set(false);
        });
    }
}
