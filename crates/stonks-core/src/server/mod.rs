//! Stoat deployment discovery, validation and persistence. The host app
//! chooses the location of the JSON selection store.

pub mod endpoints;
pub mod models;
mod probe;
mod store;

pub use endpoints::{normalize_input, validate_selection};
pub use models::{
    CaptchaCapability, ServerCapabilities, ServerEndpoints, ServerSelection,
    DEFAULT_ATTACHMENT_SIZE_LIMIT,
};
pub use probe::probe_server;
pub use store::{read_selection, save_selection};

pub const OFFICIAL_DEPLOYMENT: &str = "https://stoat.chat";
pub const OFFICIAL_API: &str = "https://api.stoat.chat";
pub const CONFIG_FILE: &str = "server.json";

#[cfg(test)]
mod tests;
