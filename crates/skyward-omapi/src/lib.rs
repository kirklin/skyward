//! Android's secure element service, which the phone's own programs reach
//! a secure element through: the weaver of a phone that keeps its slots in
//! one asks this service for a channel to the chip, and the service asks
//! the secure element's own. In the system an app serves it; a recovery
//! runs no app, so the recovery serves it itself.

pub mod apdu;
pub mod element;

#[cfg(target_os = "android")]
mod service;

#[cfg(target_os = "android")]
pub use service::start;

/// The name the service registers under, which is the one a phone's
/// description gives where its programs expect the service.
pub const SERVICE: &str = "android.se.omapi.ISecureElementService/default";
