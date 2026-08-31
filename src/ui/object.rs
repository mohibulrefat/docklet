//! A GObject wrapper so containers can live in a `gio::ListStore`.
//!
//! GTK's list widgets only hold GObjects, so each row needs one. These carry
//! plain fields rather than GObject properties: nothing binds to them, and the
//! property machinery would be weight for no gain.

use std::cell::RefCell;

use gtk::glib;
use gtk::subclass::prelude::*;

use crate::docker::Container;

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
