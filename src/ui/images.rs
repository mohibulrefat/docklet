//! The images page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, ColumnView, Entry, Grid, Label, Orientation, PolicyType,
    ProgressBar, Revealer, ScrolledWindow, Separator, SingleSelection, Widget, Window,
};

use super::banner::Banner;
use super::detail::field;
use super::dialog::confirm;
use super::list::{self, mono_column, text_column, Row};
use super::object::ImageObject;
use crate::docker::{
    now_seconds, Docker, DockerError, Image, ImageInspect, PullEvent, StreamHandle,
};

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
    reference: Entry,
    pull_button: Button,
    progress: Revealer,
    progress_bar: ProgressBar,
    progress_label: Label,
    remove_button: Button,
    detail: ImageDetail,
    /// The running pull. Dropping it cancels the stream.
    pull: RefCell<Option<StreamHandle>>,
}

/// A read-only view of one image.
struct ImageDetail {
    root: GtkBox,
    back: Button,
    title: Label,
    platform: Label,
    created: Label,
    layers: Label,
    entrypoint: Label,
    command: Label,
    ports: Label,
    environment: Label,
}

impl ImageDetail {
    fn new() -> Self {
        let back = Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Back to images"));

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

        let platform = field(&grid, 0, "Platform", false);
        let created = field(&grid, 1, "Created", false);
        let layers = field(&grid, 2, "Layers", false);
        let entrypoint = field(&grid, 3, "Entrypoint", true);
        let command = field(&grid, 4, "Command", true);
        let ports = field(&grid, 5, "Ports", false);
        let environment = field(&grid, 6, "Environment", true);

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

        ImageDetail {
            root,
            back,
            title,
            platform,
            created,
            layers,
            entrypoint,
            command,
            ports,
            environment,
        }
    }

    /// Show the name immediately; inspect fields are blanked until they load.
    fn show(&self, name: &str) {
        self.title.set_text(name);
        for label in [
            &self.platform,
            &self.created,
            &self.layers,
            &self.entrypoint,
            &self.command,
            &self.ports,
            &self.environment,
        ] {
            label.set_text("");
        }
    }

    fn set_inspect(&self, inspect: &ImageInspect) {
        self.platform.set_text(&inspect.platform());
        self.created.set_text(&inspect.created);
        self.layers.set_text(&inspect.layer_count().to_string());
        self.entrypoint.set_text(&inspect.entrypoint());
        self.command.set_text(&inspect.command());
        self.ports.set_text(&inspect.ports());
        self.environment.set_text(&inspect.environment());
    }
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

        let reference = Entry::builder()
            .placeholder_text("alpine:latest")
            .hexpand(true)
            .build();
        let pull_button = Button::with_label("Pull");
        pull_button.add_css_class("suggested-action");

        let actions = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let remove_button = Button::with_label("Remove");
        remove_button.add_css_class("destructive-action");
        remove_button.set_sensitive(false);

        actions.append(&reference);
        actions.append(&pull_button);
        actions.append(&remove_button);

        let progress_label = Label::builder()
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let progress_bar = ProgressBar::builder().hexpand(true).build();
        let cancel_pull = Button::with_label("Cancel");

        let progress_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_bottom(6)
            .build();
        progress_row.append(&progress_bar);
        progress_row.append(&cancel_pull);

        let progress_box = GtkBox::new(Orientation::Vertical, 2);
        progress_box.set_margin_start(6);
        progress_box.append(&progress_label);
        progress_box.append(&progress_row);

        let progress = Revealer::builder()
            .child(&progress_box)
            .reveal_child(false)
            .build();

        let detail = ImageDetail::new();

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(banner.widget());
        root.append(&actions);
        root.append(&progress);
        root.append(&scrolled);
        root.append(&empty);
        root.append(&detail.root);

        let page = Rc::new(ImagesPage {
            root,
            scrolled,
            empty,
            store,
            selection,
            banner,
            refreshing: Rc::new(Cell::new(false)),
            reference,
            pull_button,
            progress,
            progress_bar,
            progress_label,
            remove_button,
            detail,
            pull: RefCell::new(None),
        });

        page.pull_button.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.start_pull();
                }
            }
        });
        // Enter in the entry pulls, as it would in a terminal.
        page.reference.connect_activate({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.start_pull();
                }
            }
        });
        cancel_pull.connect_clicked({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.finish_pull(Some("Pull cancelled."));
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
        // Removing needs a selected image.
        page.selection.connect_selected_item_notify({
            let page = Rc::downgrade(&page);
            move |_| {
                if let Some(page) = page.upgrade() {
                    let ready = page.selected().is_some();
                    page.remove_button.set_sensitive(ready);
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

    /// Show the detail pane for the selected image.
    fn open_detail(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let name = match row.repository().as_str() {
            "<none>" => row.short_id(),
            repository => format!("{repository}:{}", row.tag()),
        };
        self.detail.show(&name);

        self.scrolled.set_visible(false);
        self.empty.set_visible(false);
        self.detail.root.set_visible(true);

        let id = row.id();
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let inspected =
                gio::spawn_blocking(move || Docker::connect()?.inspect_image(&id)).await;

            let Some(page) = page.upgrade() else {
                return;
            };
            match inspected {
                Ok(Ok(inspect)) => page.detail.set_inspect(&inspect),
                Ok(Err(e)) => page.show_error(&format!("Could not inspect image. {e}")),
                Err(_) => page.show_error("Could not inspect image."),
            }
        });
    }

    /// Return to the list.
    fn close_detail(self: &Rc<Self>) {
        self.detail.root.set_visible(false);
        let is_empty = self.store.n_items() == 0;
        self.empty.set_visible(is_empty);
        self.scrolled.set_visible(!is_empty);
    }

    /// Whether the detail pane is showing.
    fn showing_detail(&self) -> bool {
        self.detail.root.is_visible()
    }

    /// Remove the selected image, after confirming.
    fn remove_selected(self: &Rc<Self>) {
        let Some(row) = self.selected() else {
            return;
        };
        let id = row.id();
        let name = match row.repository().as_str() {
            "<none>" => row.short_id(),
            repository => format!("{repository}:{}", row.tag()),
        };
        let parent = self.root.root().and_downcast::<Window>();
        self.banner.clear();
        let page = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let confirmed = confirm(
                parent.as_ref(),
                &format!("Remove {name}?"),
                "The image is deleted from this machine. Containers using it are unaffected \
                 until they are recreated.",
                "Remove",
            )
            .await;
            if !confirmed {
                return;
            }

            match remove(&id, false).await {
                Ok(()) => {}
                Err(DockerError::Api { status: 409, .. }) => {
                    let forced = confirm(
                        parent.as_ref(),
                        &format!("{name} is still in use."),
                        "A container still references this image. Removing it anyway can leave \
                         those containers unable to start.",
                        "Force remove",
                    )
                    .await;
                    if !forced {
                        return;
                    }
                    if let Err(e) = remove(&id, true).await {
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

    /// Begin pulling whatever the entry names.
    fn start_pull(self: &Rc<Self>) {
        let reference = self.reference.text().trim().to_string();
        if reference.is_empty() {
            return;
        }
        if self.pull.borrow().is_some() {
            // One pull at a time; the button is insensitive, but Enter is not.
            return;
        }

        self.banner.clear();
        self.progress_label
            .set_text(&format!("Pulling {reference}…"));
        self.progress_bar.set_fraction(0.0);
        self.progress.set_reveal_child(true);
        self.pull_button.set_sensitive(false);
        self.reference.set_sensitive(false);

        let (sender, receiver) = async_channel::bounded::<PullEvent>(64);
        let handle = match Docker::connect() {
            Ok(docker) => docker.pull_image(&reference, sender),
            Err(e) => {
                self.finish_pull(Some(&format!("Could not pull {reference}. {e}")));
                return;
            }
        };
        self.pull.replace(Some(handle));

        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut failure = None;

            while let Ok(event) = receiver.recv().await {
                let Some(page) = page.upgrade() else {
                    return;
                };
                match event {
                    PullEvent::Progress { message, fraction } => {
                        page.progress_label.set_text(&message);
                        match fraction {
                            Some(fraction) => page.progress_bar.set_fraction(fraction),
                            // Docker gives no counts for this step; show motion
                            // rather than a bar frozen at its last value.
                            None => page.progress_bar.pulse(),
                        }
                    }
                    PullEvent::Failed(message) => failure = Some(message),
                }
            }

            // The channel closing is how a finished stream reports itself.
            let Some(page) = page.upgrade() else {
                return;
            };
            match failure {
                Some(message) => page.finish_pull(Some(&format!("Pull failed. {message}"))),
                None => {
                    page.finish_pull(None);
                    page.reference.set_text("");
                    page.refresh();
                }
            }
        });
    }

    /// Tear down the pull UI, reporting a failure if there was one.
    fn finish_pull(&self, error: Option<&str>) {
        // Dropping the handle stops the worker if it is still running.
        self.pull.replace(None);
        self.progress.set_reveal_child(false);
        self.pull_button.set_sensitive(true);
        self.reference.set_sensitive(true);

        if let Some(message) = error {
            self.show_error(message);
        }
    }

    pub fn widget(&self) -> &Widget {
        self.root.upcast_ref()
    }

    /// The image the user has selected, if any.
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
                    if !page.showing_detail() {
                        let is_empty = page.store.n_items() == 0;
                        page.empty.set_visible(is_empty);
                        page.scrolled.set_visible(!is_empty);
                    }
                }
                Ok(Err(e)) => page.show_error(&format!("Could not list images. {e}")),
                Err(_) => page.show_error("Could not list images."),
            }

            page.refreshing.set(false);
        });
    }
}

/// Remove an image off the main thread.
async fn remove(id: &str, force: bool) -> Result<(), DockerError> {
    let id = id.to_string();
    gio::spawn_blocking(move || Docker::connect()?.remove_image(&id, force))
        .await
        .unwrap_or_else(|_| Err(DockerError::Protocol("the remove task panicked".into())))
}
