//! The volumes page.

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
use super::list::{self, mono_column, text_column, Row};
use super::object::VolumeObject;
use crate::docker::{Docker, DockerError, Volume};

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
    name_entry: Entry,
    driver_entry: Entry,
    remove_button: Button,
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

        let name_entry = Entry::builder()
            .placeholder_text("volume name")
            .hexpand(true)
            .build();
        let driver_entry = Entry::builder()
            .placeholder_text("driver (local)")
            .width_chars(14)
            .build();
        let create_button = Button::with_label("Create");
        create_button.add_css_class("suggested-action");

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

        let remove_button = Button::with_label("Remove");
        remove_button.add_css_class("destructive-action");
        remove_button.set_sensitive(false);
        actions.append(&remove_button);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
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
            name_entry,
            driver_entry,
            remove_button,
        });

        create_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.create_volume();
                }
            }
        });
        page.name_entry.connect_activate({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.create_volume();
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
                    let ready = page.selected().is_some();
                    page.remove_button.set_sensitive(ready);
                }
            }
        });

        page.refresh();
        page
    }

    /// Remove the selected volume, after confirming.
    fn remove_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let name = row.name();
        let parent = self.root.root().and_downcast::<Window>();
        self.banner.clear();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let confirmed = confirm(
                parent.as_ref(),
                &format!("Remove volume {name}?"),
                "Everything stored in the volume is deleted. This cannot be undone.",
                "Remove",
            )
            .await;
            if !confirmed {
                return;
            }

            match remove(&name, false).await {
                Ok(()) => {}
                Err(DockerError::Api { status: 409, .. }) => {
                    let forced = confirm(
                        parent.as_ref(),
                        &format!("{name} is still in use."),
                        "A container still mounts this volume.",
                        "Force remove",
                    )
                    .await;
                    if !forced {
                        return;
                    }
                    if let Err(e) = remove(&name, true).await {
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

    /// Create a volume from the entries.
    fn create_volume(self: &Rc<Self>) {
        let name = self.name_entry.text().trim().to_string();
        if name.is_empty() {
            self.show_error("Enter a name for the volume.");
            return;
        }
        let driver = self.driver_entry.text().trim().to_string();

        self.banner.clear();
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let created = {
                let (name, driver) = (name.clone(), driver.clone());
                gio::spawn_blocking(move || Docker::connect()?.create_volume(&name, &driver)).await
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

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

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

/// Remove a volume off the main thread.
async fn remove(name: &str, force: bool) -> Result<(), DockerError> {
    let name = name.to_string();
    gio::spawn_blocking(move || Docker::connect()?.remove_volume(&name, force))
        .await
        .unwrap_or_else(|_| Err(DockerError::Protocol("the remove task panicked".into())))
}
