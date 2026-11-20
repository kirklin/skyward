//! The phone's own programs, as its own partitions describe them: how its
//! init scripts say each is started, and which services each registers.

pub mod description;
pub mod naming;
pub mod phone;
pub mod script;
pub mod secure;
