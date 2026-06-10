//! In-process runtime automation server for GPUI applications.
//!
//! Add `gpui_driver::init(cx)` behind a cargo feature in your app and annotate
//! interactive elements with [`DriverExt::driver_id`]; the `gpui-driver` CLI can then
//! inspect, click and screenshot the running app — without window focus, and even when
//! the session is locked.
//!
//! ```ignore
//! fn main() {
//!     gpui_platform::application().run(|cx| {
//!         #[cfg(feature = "driver")]
//!         gpui_driver::init(cx);
//!         // ... normal app setup
//!     });
//! }
//! ```
//!
//! **Never enable this in release builds.** The server accepts JSON-RPC on localhost
//! authenticated only by a token in the user's temp directory.

mod discovery;
mod element;
mod handlers;
mod registry;
mod server;

pub use element::{DriverExt, DriverNode};
use gpui_driver_protocol::{DiscoveryFile, PROTOCOL_VERSION};

/// Optional metadata for [`init_with_options`].
#[derive(Default)]
pub struct DriverOptions {
    /// Name the CLI selects the app by (`--app <name>`). Defaults to the executable
    /// file stem.
    pub app_name: Option<String>,
    /// Reported by the `info` method. Defaults to `"unknown"`; pass
    /// `env!("CARGO_PKG_VERSION")` from your app.
    pub app_version: Option<String>,
}

/// Starts the driver server and writes the discovery file. Call once from your app's
/// `run` callback.
pub fn init(cx: &mut gpui::App) {
    init_with_options(cx, DriverOptions::default());
}

/// [`init`] with explicit app metadata.
pub fn init_with_options(cx: &mut gpui::App, options: DriverOptions) {
    let app_name = options.app_name.unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "unknown".to_string())
    });
    let app_version = options.app_version.unwrap_or_else(|| "unknown".to_string());

    let token = discovery::generate_token();
    let (port, rx) = match server::start(token.clone()) {
        Ok(started) => started,
        Err(e) => {
            log::error!("gpui-driver: failed to start server: {e:#}");
            return;
        }
    };

    let file = DiscoveryFile {
        app_name: app_name.clone(),
        pid: std::process::id(),
        port,
        token,
        protocol_version: PROTOCOL_VERSION,
        started_at: discovery::now_iso8601(),
    };
    match discovery::write(&file) {
        Ok(path) => log::info!(
            "gpui-driver: listening on 127.0.0.1:{port}, discovery file {}",
            path.display()
        ),
        Err(e) => log::error!("gpui-driver: failed to write discovery file: {e:#}"),
    }

    cx.on_app_quit(|_| {
        discovery::remove_own();
        std::future::ready(())
    })
    .detach();

    handlers::spawn(
        cx,
        handlers::DriverMeta {
            app_name,
            app_version,
        },
        rx,
    );
}
