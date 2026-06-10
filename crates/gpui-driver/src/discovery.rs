//! Discovery files: `<temp>/gpui-driver/<pid>.json`, written on init so the CLI can
//! find running instrumented apps. Removed on clean shutdown; the CLI treats files
//! whose process no longer answers as stale and deletes them.

use std::path::PathBuf;

use gpui_driver_protocol::{DISCOVERY_DIR_NAME, DiscoveryFile};
use rand::RngCore;

pub(crate) fn discovery_dir() -> PathBuf {
    std::env::temp_dir().join(DISCOVERY_DIR_NAME)
}

fn discovery_path(pid: u32) -> PathBuf {
    discovery_dir().join(format!("{pid}.json"))
}

pub(crate) fn write(file: &DiscoveryFile) -> anyhow::Result<PathBuf> {
    let dir = discovery_dir();
    std::fs::create_dir_all(&dir)?;
    let path = discovery_path(file.pid);
    std::fs::write(&path, serde_json::to_string_pretty(file)?)?;
    Ok(path)
}

pub(crate) fn remove_own() {
    let _ = std::fs::remove_file(discovery_path(std::process::id()));
}

pub(crate) fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars_and_random() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn discovery_round_trip_on_disk() {
        let file = DiscoveryFile {
            app_name: "test-app".into(),
            pid: std::process::id(),
            port: 12345,
            token: generate_token(),
            protocol_version: 1,
            started_at: now_iso8601(),
        };
        let path = write(&file).unwrap();
        let read: DiscoveryFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read.port, 12345);
        assert_eq!(read.app_name, "test-app");
        remove_own();
        assert!(!path.exists());
    }
}
