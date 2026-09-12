//! A GObject wrapper so containers can live in a `gio::ListStore`.
//!
//! GTK's list widgets only hold GObjects, so each row needs one. These carry
//! plain fields rather than GObject properties: nothing binds to them, and the
//! property machinery would be weight for no gain.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::subclass::prelude::*;

use super::list::Row;
use crate::docker::{
    ComposeProject, ComposeService, Container, Image, Network, ProjectState, Volume,
};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ContainerObject {
        pub id: RefCell<String>,
        pub name: RefCell<String>,
        pub image: RefCell<String>,
        pub state: RefCell<String>,
        pub status: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContainerObject {
        const NAME: &'static str = "DockletContainerObject";
        type Type = super::ContainerObject;
    }

    impl ObjectImpl for ContainerObject {}
}

glib::wrapper! {
    pub struct ContainerObject(ObjectSubclass<imp::ContainerObject>);
}

impl ContainerObject {
    pub fn new(container: &Container) -> Self {
        let object: Self = glib::Object::new();
        object.set(container);
        object
    }

    /// Copy a container's fields into this row.
    pub fn set(&self, container: &Container) {
        let imp = self.imp();
        imp.id.replace(container.id.clone());
        imp.name.replace(container.name().to_string());
        imp.image.replace(container.image.clone());
        imp.state.replace(container.state.clone());
        imp.status.replace(container.status.clone());
    }

    /// Whether this row already shows exactly this container.
    ///
    /// Refreshes are frequent and usually change nothing; comparing first means
    /// an unchanged row is never rebound, so it neither flickers nor disturbs
    /// the selection.
    pub fn matches(&self, container: &Container) -> bool {
        let imp = self.imp();
        *imp.id.borrow() == container.id
            && *imp.name.borrow() == container.name()
            && *imp.image.borrow() == container.image
            && *imp.state.borrow() == container.state
            && *imp.status.borrow() == container.status
    }

    pub fn id(&self) -> String {
        self.imp().id.borrow().clone()
    }

    /// The 12-character id Docker displays.
    pub fn short_id(&self) -> String {
        crate::docker::short_id(&self.imp().id.borrow()).to_string()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn image(&self) -> String {
        self.imp().image.borrow().clone()
    }

    pub fn state(&self) -> String {
        self.imp().state.borrow().clone()
    }

    pub fn status(&self) -> String {
        self.imp().status.borrow().clone()
    }
}

impl Row for ContainerObject {
    type Data = Container;

    fn build(container: &Container) -> Self {
        ContainerObject::new(container)
    }

    fn key(&self) -> String {
        self.id()
    }

    fn key_of(container: &Container) -> &str {
        &container.id
    }

    fn matches(&self, container: &Container) -> bool {
        ContainerObject::matches(self, container)
    }

    fn update(&self, container: &Container) {
        self.set(container);
    }
}

mod image_imp {
    use super::*;

    #[derive(Default)]
    pub struct ImageObject {
        pub id: RefCell<String>,
        pub repository: RefCell<String>,
        pub tag: RefCell<String>,
        pub short_id: RefCell<String>,
        pub size: RefCell<String>,
        pub created: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageObject {
        const NAME: &'static str = "DockletImageObject";
        type Type = super::ImageObject;
    }

    impl ObjectImpl for ImageObject {}
}

glib::wrapper! {
    pub struct ImageObject(ObjectSubclass<image_imp::ImageObject>);
}

impl ImageObject {
    /// Ages are relative, so the row is built against a fixed "now" — one per
    /// refresh, so every row in a list agrees on the time.
    pub fn new(image: &Image, now: i64) -> Self {
        let object: Self = glib::Object::new();
        object.set(image, now);
        object
    }

    pub fn set(&self, image: &Image, now: i64) {
        let imp = self.imp();
        imp.id.replace(image.id.clone());
        imp.repository.replace(image.repository().to_string());
        imp.tag.replace(image.tag().to_string());
        imp.short_id.replace(image.short_id().to_string());
        imp.size.replace(image.size_display());
        imp.created.replace(image.created_display(now));
    }

    pub fn id(&self) -> String {
        self.imp().id.borrow().clone()
    }

    pub fn repository(&self) -> String {
        self.imp().repository.borrow().clone()
    }

    pub fn tag(&self) -> String {
        self.imp().tag.borrow().clone()
    }

    pub fn short_id(&self) -> String {
        self.imp().short_id.borrow().clone()
    }

    pub fn size(&self) -> String {
        self.imp().size.borrow().clone()
    }

    pub fn created(&self) -> String {
        self.imp().created.borrow().clone()
    }
}

mod volume_imp {
    use super::*;

    #[derive(Default)]
    pub struct VolumeObject {
        pub name: RefCell<String>,
        pub driver: RefCell<String>,
        pub mountpoint: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VolumeObject {
        const NAME: &'static str = "DockletVolumeObject";
        type Type = super::VolumeObject;
    }

    impl ObjectImpl for VolumeObject {}
}

glib::wrapper! {
    pub struct VolumeObject(ObjectSubclass<volume_imp::VolumeObject>);
}

impl VolumeObject {
    pub fn new(volume: &Volume) -> Self {
        let object: Self = glib::Object::new();
        object.set(volume);
        object
    }

    pub fn set(&self, volume: &Volume) {
        let imp = self.imp();
        imp.name.replace(volume.name.clone());
        imp.driver.replace(volume.driver.clone());
        imp.mountpoint.replace(volume.mountpoint.clone());
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn driver(&self) -> String {
        self.imp().driver.borrow().clone()
    }

    pub fn mountpoint(&self) -> String {
        self.imp().mountpoint.borrow().clone()
    }
}

mod network_imp {
    use super::*;

    #[derive(Default)]
    pub struct NetworkObject {
        pub id: RefCell<String>,
        pub name: RefCell<String>,
        pub driver: RefCell<String>,
        pub scope: RefCell<String>,
        pub predefined: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NetworkObject {
        const NAME: &'static str = "DockletNetworkObject";
        type Type = super::NetworkObject;
    }

    impl ObjectImpl for NetworkObject {}
}

glib::wrapper! {
    pub struct NetworkObject(ObjectSubclass<network_imp::NetworkObject>);
}

impl NetworkObject {
    pub fn new(network: &Network) -> Self {
        let object: Self = glib::Object::new();
        object.set(network);
        object
    }

    pub fn set(&self, network: &Network) {
        let imp = self.imp();
        imp.id.replace(network.id.clone());
        imp.name.replace(network.name.clone());
        imp.driver.replace(network.driver.clone());
        imp.scope.replace(network.scope.clone());
        imp.predefined.set(network.is_predefined());
    }

    pub fn id(&self) -> String {
        self.imp().id.borrow().clone()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn driver(&self) -> String {
        self.imp().driver.borrow().clone()
    }

    pub fn scope(&self) -> String {
        self.imp().scope.borrow().clone()
    }

    /// Docker owns this network and will not let it be removed.
    pub fn is_predefined(&self) -> bool {
        self.imp().predefined.get()
    }
}

/// One row of the flattened Compose list: a project, or one of its services
/// shown underneath it while the project is expanded.
///
/// This is a presentation concept, not Docker data — `ComposeProject` and
/// `ComposeService` (in `docker::compose`) know nothing about expansion or
/// row flattening, only about what Compose's labels say. Building this from
/// them is `ui::compose`'s job; it lives here because it is the `Data` type
/// [`ComposeRowObject`] is built and diffed from, matching every other row
/// object in this module.
pub enum ComposeRow {
    Project {
        key: String,
        project: ComposeProject,
        expanded: bool,
    },
    Service {
        key: String,
        service: ComposeService,
    },
}

impl ComposeRow {
    pub fn project(project: ComposeProject, expanded: bool) -> Self {
        ComposeRow::Project {
            key: format!("p:{}", project.name),
            project,
            expanded,
        }
    }

    pub fn service(service: ComposeService) -> Self {
        ComposeRow::Service {
            key: format!("s:{}", service.container_id),
            service,
        }
    }

    /// The identity used to match this row across refreshes.
    pub fn key(&self) -> &str {
        match self {
            ComposeRow::Project { key, .. } | ComposeRow::Service { key, .. } => key,
        }
    }
}

/// A project's aggregate state, or a service's own state, as the sentinel
/// `"partial"` or the raw Docker state string — the shared vocabulary
/// `state_indicator` (see `ui::containers`) already knows how to draw.
fn state_kind_of(project: &ComposeProject) -> String {
    match project.state() {
        ProjectState::Uniform(state) => state,
        ProjectState::Partial => "partial".to_string(),
    }
}

mod compose_row_imp {
    use super::*;

    #[derive(Default)]
    pub struct ComposeRowObject {
        pub key: RefCell<String>,
        pub is_project: Cell<bool>,
        pub name: RefCell<String>,
        pub detail: RefCell<String>,
        pub state_kind: RefCell<String>,
        pub working_dir: RefCell<String>,
        pub expanded: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComposeRowObject {
        const NAME: &'static str = "DockletComposeRowObject";
        type Type = super::ComposeRowObject;
    }

    impl ObjectImpl for ComposeRowObject {}
}

glib::wrapper! {
    pub struct ComposeRowObject(ObjectSubclass<compose_row_imp::ComposeRowObject>);
}

impl ComposeRowObject {
    pub fn new(row: &ComposeRow) -> Self {
        let object: Self = glib::Object::new();
        object.set(row);
        object
    }

    /// Copy a row's fields into this object.
    pub fn set(&self, row: &ComposeRow) {
        let imp = self.imp();
        imp.key.replace(row.key().to_string());
        match row {
            ComposeRow::Project {
                project, expanded, ..
            } => {
                imp.is_project.set(true);
                imp.name.replace(project.name.clone());
                imp.detail.replace(format!(
                    "{} service{}",
                    project.service_count(),
                    if project.service_count() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ));
                imp.state_kind.replace(state_kind_of(project));
                imp.working_dir.replace(project.working_dir.clone());
                imp.expanded.set(*expanded);
            }
            ComposeRow::Service { service, .. } => {
                imp.is_project.set(false);
                // Indented so it visually nests under its project without a
                // second widget/column just to draw a hierarchy.
                imp.name.replace(format!("    {}", service.name));
                imp.detail.replace(service.container_name.clone());
                imp.state_kind.replace(service.state.clone());
                imp.working_dir.replace(String::new());
                imp.expanded.set(false);
            }
        }
    }

    /// Whether this row already shows exactly this data.
    pub fn matches(&self, row: &ComposeRow) -> bool {
        let imp = self.imp();
        if *imp.key.borrow() != row.key() {
            return false;
        }
        match row {
            ComposeRow::Project {
                project, expanded, ..
            } => {
                imp.is_project.get()
                    && *imp.name.borrow() == project.name
                    && *imp.detail.borrow()
                        == format!(
                            "{} service{}",
                            project.service_count(),
                            if project.service_count() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        )
                    && *imp.state_kind.borrow() == state_kind_of(project)
                    && *imp.working_dir.borrow() == project.working_dir
                    && imp.expanded.get() == *expanded
            }
            ComposeRow::Service { service, .. } => {
                !imp.is_project.get()
                    && *imp.name.borrow() == format!("    {}", service.name)
                    && *imp.detail.borrow() == service.container_name
                    && *imp.state_kind.borrow() == service.state
            }
        }
    }

    pub fn key(&self) -> String {
        self.imp().key.borrow().clone()
    }

    pub fn is_project(&self) -> bool {
        self.imp().is_project.get()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    /// "N services" for a project, or the container name for a service.
    pub fn detail(&self) -> String {
        self.imp().detail.borrow().clone()
    }

    /// The raw Docker state, or `"partial"` for a project whose services
    /// disagree — see [`state_kind_of`].
    pub fn state_kind(&self) -> String {
        self.imp().state_kind.borrow().clone()
    }

    /// A human label for `state_kind`: the exact state, title-cased, or
    /// "Partial".
    pub fn state_label(&self) -> String {
        let kind = self.state_kind();
        if kind.is_empty() {
            return String::new();
        }
        if kind == "partial" {
            return "Partial".to_string();
        }
        let mut chars = kind.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }

    pub fn working_dir(&self) -> String {
        self.imp().working_dir.borrow().clone()
    }

    /// Only meaningful for a project row: whether its services are shown.
    pub fn expanded(&self) -> bool {
        self.imp().expanded.get()
    }
}
