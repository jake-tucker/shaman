mod ansi;
pub mod approval;
mod attachments;
mod broker;
pub mod build_identity;
mod change_count;
pub mod cli;
mod clock;
mod errands;
mod icon_catalog;
pub mod logging;
pub mod login_start;
pub mod managed_client;
mod memories;
mod model_catalog;
pub mod open_files;
pub mod paths;
pub mod pricing;
mod process_tree;
pub mod protocol;
pub mod provider;
mod relay_said;
mod relays;
mod runtime;
pub mod server;
mod serving;
mod session_projection;
mod sessions;
pub mod settings;
mod shown_text;
mod sidekick;
mod skill_catalog;
pub mod source_control;
mod storage;
mod terminal;
mod theme;
pub mod tui;

pub use runtime::{LastStop, RuntimeConfig};
pub use serving::IdentityStoreChoice;

pub mod questionnaire;

/// The Icon Catalog's names, in listing order — the same list the Title
/// Errand's reply schema enumerates an Icon over. The Catalog itself stays
/// crate-private; this is a deliberate, narrow seam so integration tests (a
/// separate crate) can assert a schema's `icon` enum matches it without
/// reaching into `icon_catalog` itself.
pub fn icon_catalog_names() -> &'static [&'static str] {
    icon_catalog::names()
}
