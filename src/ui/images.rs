//! The images page.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, ColumnView, Label, Orientation, PolicyType, ScrolledWindow,
    SingleSelection, Widget,
};

use super::banner::Banner;
use super::list::{self, mono_column, text_column, Row};
use super::object::ImageObject;
use crate::docker::{now_seconds, Docker, Image};

const LOG_DOMAIN: &str = "docklet";

impl Row for ImageObject {
    type Data = Image;

    fn build(image: &Image) -> Self {
        // Rebuilt rows share the refresh's clock closely enough; ages are
        // relative to the minute at coarsest.
        ImageObject::new(image, now_seconds())
    }

    fn key(&self) -> String {
        self.id()
    }

    fn key_of(image: &Image) -> &str {
        &image.id
    }

    fn matches(&self, image: &Image) -> bool {
        self.id() == image.id
            && self.repository() == image.repository()
            && self.tag() == image.tag()
            && self.size() == image.size_display()
    }

    fn update(&self, image: &Image) {
        self.set(image, now_seconds());
    }
}

/// The image list.
pub struct ImagesPage {
    root: GtkBox,
    scrolled: ScrolledWindow,
    empty: Label,
    store: gio::ListStore,
    selection: SingleSelection,
    banner: Banner,
    refreshing: Rc<Cell<bool>>,
}

impl ImagesPage {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<ImageObject>();
        let selection = SingleSelection::new(Some(store.clone()));

        let view = ColumnView::builder().model(&selection).build();
        view.append_column(&text_column::<ImageObject>(
            "Repository",
            ImageObject::repository,
        ));
        view.append_column(&text_column::<ImageObject>("Tag", ImageObject::tag));
        view.append_column(&mono_column::<ImageObject>("ID", ImageObject::short_id));
        view.append_column(&text_column::<ImageObject>("Size", ImageObject::size));
        view.append_column(&text_column::<ImageObject>("Created", ImageObject::created));

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = Label::builder()
            .label("No images")
            .vexpand(true)
            .visible(false)
            .build();
        empty.add_css_class("dim-label");

        let banner = Banner::new();

        let actions = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .halign(Align::Start)
            .build();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
        root.append(&scrolled);
        root.append(&empty);

        let page = Rc::new(ImagesPage {
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

    /// The image the user has selected, if any.
    #[allow(dead_code)]
    fn selected(&self) -> Option<ImageObject> {
        self.selection.selected_item().and_downcast::<ImageObject>()
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
            let listed = gio::spawn_blocking(|| Docker::connect()?.images()).await;

            let Some(page) = page.upgrade() else {
                return;
            };

            match listed {
                Ok(Ok(images)) => {
                    list::apply::<ImageObject>(&page.store, &images);
                    let is_empty = page.store.n_items() == 0;
                    page.empty.set_visible(is_empty);
                    page.scrolled.set_visible(!is_empty);
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list images. {e}")),
                Err(_) => page.show_error("Could not list images."),
            }

            page.refreshing.set(false);
        });
    }
}
