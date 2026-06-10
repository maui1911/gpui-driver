//! In-process runtime automation server for GPUI applications.
//!
//! Add `gpui_driver::init(cx)` behind a cargo feature in your app and annotate
//! interactive elements with [`DriverExt::driver_id`]; the `gpui-driver` CLI can then
//! inspect, click and screenshot the running app — without window focus, and even when
//! the session is locked.

mod element;
mod registry;

pub use element::{DriverExt, DriverNode};
