//! Confirmation prompts for destructive actions.

use gtk::{AlertDialog, Window};

/// Ask the user to confirm, returning whether they agreed.
///
/// Cancel is both the default and the cancel button, so a stray Return or
/// Escape never destroys anything.
pub async fn confirm(parent: Option<&Window>, message: &str, detail: &str, action: &str) -> bool {
    let dialog = AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .buttons(["Cancel", action])
        .cancel_button(0)
        .default_button(0)
        .build();

    // An error means the dialog was dismissed, which is a "no".
    dialog.choose_future(parent).await == Ok(1)
}
