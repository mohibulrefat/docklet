//! Shared machinery for the list pages.
//!
//! Containers, images, volumes and networks are all "a Docker list rendered in
//! a `ColumnView`". The diffing is subtle enough that copying it per page would
//! mean copying its bugs too, so it lives here once.

use gtk::gio;
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;
use gtk::{Align, ColumnViewColumn, Label, ListItem, SignalListItemFactory};

/// A row object backed by one Docker value.
pub trait Row: IsA<gtk::glib::Object> {
    /// The Docker value this row displays.
    type Data;

    /// Build a row for a value.
    fn build(data: &Self::Data) -> Self;

    /// The identity Docker gives this row, used to match across refreshes.
    fn key(&self) -> String;

    /// The identity of a value, for the same purpose.
    fn key_of(data: &Self::Data) -> &str;

    /// Whether this row already shows exactly this value.
    fn matches(&self, data: &Self::Data) -> bool;

    /// Copy a value's fields into this row.
    fn update(&self, data: &Self::Data);
}

/// Bring a store into line with a freshly fetched list.
///
/// Rows are matched by Docker's own identity and updated in place rather than
/// the store being cleared and refilled: that keeps the selection, avoids
/// flicker, and leaves rows whose values did not change completely untouched.
pub fn apply<R: Row>(store: &gio::ListStore, items: &[R::Data]) {
    for (index, item) in items.iter().enumerate() {
        let index = index as u32;

        match find::<R>(store, R::key_of(item), index) {
            Some(found) if found == index => {
                let row = row_at::<R>(store, index);
                if !row.matches(item) {
                    row.update(item);
                    // No properties to notify, so ask the view to rebind.
                    store.items_changed(index, 1, 1);
                }
            }
            Some(found) => {
                // Same value, different position: move the row rather than
                // rebuilding it, so it keeps its identity.
                let row = row_at::<R>(store, found);
                store.remove(found);
                row.update(item);
                store.insert(index, &row);
            }
            None => store.insert(index, &R::build(item)),
        }
    }

    // Anything past the new length is gone from Docker.
    while store.n_items() > items.len() as u32 {
        store.remove(store.n_items() - 1);
    }
}

/// Position of the row for `key`, searching from `from` onwards.
fn find<R: Row>(store: &gio::ListStore, key: &str, from: u32) -> Option<u32> {
    (from..store.n_items()).find(|&i| row_at::<R>(store, i).key() == key)
}

/// The row at an index.
pub fn row_at<R: Row>(store: &gio::ListStore, index: u32) -> R {
    store
        .item(index)
        .and_downcast::<R>()
        .expect("the store only ever holds its own row type")
}

/// A column of text that shares the remaining width and truncates when narrow.
pub fn text_column<R: Row>(title: &str, field: fn(&R) -> String) -> ColumnViewColumn {
    column(title, field, false, true)
}

/// A fixed-width column of monospace text, for ids.
pub fn mono_column<R: Row>(title: &str, field: fn(&R) -> String) -> ColumnViewColumn {
    column(title, field, true, false)
}

/// A column rendering one text field of a row.
pub fn column<R: Row>(
    title: &str,
    field: fn(&R) -> String,
    monospace: bool,
    expand: bool,
) -> ColumnViewColumn {
    let factory = SignalListItemFactory::new();

    factory.connect_setup(move |_, item| {
        let label = Label::builder()
            .halign(Align::Start)
            .ellipsize(EllipsizeMode::End)
            .build();
        if monospace {
            label.add_css_class("monospace");
        }
        item.downcast_ref::<ListItem>()
            .expect("list item")
            .set_child(Some(&label));
    });

    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<ListItem>().expect("list item");
        let Some(object) = item.item().and_downcast::<R>() else {
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
        .expand(expand)
        .resizable(true)
        .build()
}
