//! Discovery-file scanning, app selection, and the JSON-RPC TCP client.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_driver_protocol::{DISCOVERY_DIR_NAME, DiscoveryFile, ErrorKind, RpcRequest, RpcResponse};

/// Failures that map onto the CLI's documented exit codes (see `main.rs`).
#[derive(Debug)]
pub enum CliError {
    /// The app responded with a JSON-RPC error.
    Rpc {
        kind: Option<ErrorKind>,
        message: String,
    },
    /// No (matching) live instrumented app was found.
    NoApp(String),
    /// Transport/serialization trouble.
    Protocol(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Rpc { kind, message } => match kind {
                Some(kind) => write!(f, "{message} ({kind:?})"),
                None => write!(f, "{message}"),
            },
            CliError::NoApp(message) => write!(f, "{message}"),
            CliError::Protocol(message) => write!(f, "{message}"),
        }
    }
}

pub fn discovery_dir() -> PathBuf {
    // Overridable for tests.
    if let Ok(dir) = std::env::var("GPUI_DRIVER_DISCOVERY_DIR") {
        return PathBuf::from(dir);
    }
    std::env::temp_dir().join(DISCOVERY_DIR_NAME)
}

pub struct Discovered {
    pub file: DiscoveryFile,
    pub path: PathBuf,
}

/// All parseable discovery files, without liveness checking.
pub fn scan(dir: &Path) -> Vec<Discovered> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Discovered> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().is_none_or(|e| e != "json") {
                return None;
            }
            let contents = std::fs::read_to_string(&path).ok()?;
            let file: DiscoveryFile = serde_json::from_str(&contents).ok()?;
            Some(Discovered { file, path })
        })
        .collect();
    found.sort_by_key(|d| d.file.pid);
    found
}

/// Filters by `--app`/`--pid`, then liveness-probes candidates in order. Stale entries
/// (connection refused) are deleted. Errors unless exactly one live candidate matches —
/// except that with no filters, the first live app wins only if it is the *only* one.
pub fn select_app(
    dir: &Path,
    app: Option<&str>,
    pid: Option<u32>,
) -> Result<(Discovered, Client), CliError> {
    let discovered = scan(dir);
    let candidates: Vec<Discovered> = discovered
        .into_iter()
        .filter(|d| app.is_none_or(|a| d.file.app_name == a))
        .filter(|d| pid.is_none_or(|p| d.file.pid == p))
        .collect();

    let mut live: Vec<(Discovered, Client)> = Vec::new();
    for candidate in candidates {
        match Client::connect(&candidate.file) {
            Ok(client) => live.push((candidate, client)),
            Err(_) => {
                // Dead pid / closed port: stale discovery file.
                let _ = std::fs::remove_file(&candidate.path);
            }
        }
    }

    match live.len() {
        0 => Err(CliError::NoApp(format!(
            "no running instrumented app found{}{} (discovery dir: {})",
            app.map(|a| format!(" with name {a:?}")).unwrap_or_default(),
            pid.map(|p| format!(" with pid {p}")).unwrap_or_default(),
            dir.display(),
        ))),
        1 => Ok(live.remove(0)),
        n => Err(CliError::NoApp(format!(
            "{n} instrumented apps are running; disambiguate with --app <name> or --pid <pid>: {}",
            live.iter()
                .map(|(d, _)| format!("{} (pid {})", d.file.app_name, d.file.pid))
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

/// Probe liveness of every discovery entry; deletes stale files.
/// Returns `(discovered, alive)` pairs.
pub fn scan_with_liveness(dir: &Path) -> Vec<(Discovered, bool)> {
    scan(dir)
        .into_iter()
        .map(|d| {
            let alive = Client::connect(&d.file).is_ok();
            if !alive {
                let _ = std::fs::remove_file(&d.path);
            }
            (d, alive)
        })
        .collect()
}

pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    token: String,
    next_id: u64,
}

impl Client {
    pub fn connect(file: &DiscoveryFile) -> std::io::Result<Client> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], file.port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
        // Element/window resolution and forced draws are fast; wait_idle can
        // legitimately take its full timeout. Give RPCs a generous ceiling.
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        Ok(Client {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            token: file.token.clone(),
            next_id: 1,
        })
    }

    /// Sends one request (token injected into `params`) and waits for the response.
    /// Returns the `result` value, or a [`CliError::Rpc`] carrying the structured kind.
    pub fn call(
        &mut self,
        method: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, CliError> {
        if !params.is_object() {
            params = serde_json::json!({});
        }
        params["token"] = serde_json::Value::String(self.token.clone());

        let id = self.next_id;
        self.next_id += 1;
        let request = RpcRequest::new(id, method, params);
        let mut line = serde_json::to_string(&request)
            .map_err(|e| CliError::Protocol(format!("failed to encode request: {e}")))?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| CliError::Protocol(format!("failed to send request: {e}")))?;

        let mut response_line = String::new();
        self.reader
            .read_line(&mut response_line)
            .map_err(|e| CliError::Protocol(format!("failed to read response: {e}")))?;
        if response_line.is_empty() {
            return Err(CliError::Protocol("connection closed by app".into()));
        }

        let response: RpcResponse = serde_json::from_str(&response_line)
            .map_err(|e| CliError::Protocol(format!("malformed response: {e}")))?;
        if let Some(error) = response.error {
            return Err(CliError::Rpc {
                kind: error.data.map(|d| d.kind),
                message: error.message,
            });
        }
        response
            .result
            .ok_or_else(|| CliError::Protocol("response had neither result nor error".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;

    /// Minimal fake app: accepts connections and answers every request with `{"ok":true}`
    /// if the token matches, else a JSON-RPC auth error.
    fn fake_app(token: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut writer = stream;
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                        let req: RpcRequest = serde_json::from_str(&line).unwrap();
                        let ok = req.params.get("token").and_then(|t| t.as_str())
                            == Some(token);
                        let resp = if ok {
                            RpcResponse::success(req.id, json!({"ok": true}))
                        } else {
                            RpcResponse::error(req.id, ErrorKind::AuthFailed, "bad token")
                        };
                        let mut out = serde_json::to_string(&resp).unwrap();
                        out.push('\n');
                        writer.write_all(out.as_bytes()).unwrap();
                        line.clear();
                    }
                });
            }
        });
        port
    }

    fn discovery(name: &str, pid: u32, port: u16, token: &str) -> DiscoveryFile {
        DiscoveryFile {
            app_name: name.into(),
            pid,
            port,
            token: token.into(),
            protocol_version: 1,
            started_at: "2026-06-10T00:00:00Z".into(),
        }
    }

    fn write_discovery(dir: &Path, file: &DiscoveryFile) -> PathBuf {
        let path = dir.join(format!("{}.json", file.pid));
        std::fs::write(&path, serde_json::to_string(file).unwrap()).unwrap();
        path
    }

    #[test]
    fn call_round_trip_with_token() {
        let port = fake_app("tok");
        let mut client = Client::connect(&discovery("a", 1, port, "tok")).unwrap();
        let result = client.call("info", json!({})).unwrap();
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn rpc_error_carries_kind() {
        let port = fake_app("tok");
        let mut client = Client::connect(&discovery("a", 1, port, "WRONG")).unwrap();
        let err = client.call("info", json!({})).unwrap_err();
        match err {
            CliError::Rpc { kind, .. } => assert_eq!(kind, Some(ErrorKind::AuthFailed)),
            other => panic!("expected rpc error, got {other:?}"),
        }
    }

    #[test]
    fn select_app_picks_single_live_app_and_prunes_stale() {
        let dir = tempfile::tempdir().unwrap();
        let port = fake_app("tok");
        write_discovery(dir.path(), &discovery("alive", 10, port, "tok"));
        // Port 1 is essentially guaranteed closed.
        let stale_path = write_discovery(dir.path(), &discovery("dead", 11, 1, "tok"));

        let (selected, _client) = select_app(dir.path(), None, None).unwrap();
        assert_eq!(selected.file.app_name, "alive");
        assert!(!stale_path.exists(), "stale discovery file should be deleted");
    }

    #[test]
    fn select_app_filters_by_name_and_pid() {
        let dir = tempfile::tempdir().unwrap();
        let port_a = fake_app("tok");
        let port_b = fake_app("tok");
        write_discovery(dir.path(), &discovery("appa", 21, port_a, "tok"));
        write_discovery(dir.path(), &discovery("appb", 22, port_b, "tok"));

        // Ambiguous without filters.
        assert!(matches!(
            select_app(dir.path(), None, None),
            Err(CliError::NoApp(_))
        ));
        let (by_name, _) = select_app(dir.path(), Some("appb"), None).unwrap();
        assert_eq!(by_name.file.pid, 22);
        let (by_pid, _) = select_app(dir.path(), None, Some(21)).unwrap();
        assert_eq!(by_pid.file.app_name, "appa");
    }

    #[test]
    fn select_app_errors_when_nothing_matches() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            select_app(dir.path(), Some("ghost"), None),
            Err(CliError::NoApp(_))
        ));
    }
}
