//! Background TCP server speaking newline-delimited JSON-RPC 2.0.
//!
//! Connection threads never touch UI state: each request is forwarded as a [`Job`]
//! over a channel to the foreground handler loop (see `handlers.rs`) and the thread
//! blocks until the response comes back.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use futures::channel::{mpsc, oneshot};
use gpui_driver_protocol::{ErrorKind, RpcRequest, RpcResponse};

pub(crate) type Job = (RpcRequest, oneshot::Sender<RpcResponse>);

/// Binds an ephemeral localhost port and spawns the accept thread.
/// Returns the port and the receiving end for the foreground handler loop.
pub(crate) fn start(token: String) -> anyhow::Result<(u16, mpsc::UnboundedReceiver<Job>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = mpsc::unbounded();

    std::thread::Builder::new()
        .name("gpui-driver-accept".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let tx = tx.clone();
                let token = token.clone();
                let _ = std::thread::Builder::new()
                    .name("gpui-driver-conn".into())
                    .spawn(move || {
                        if let Err(e) = serve_connection(stream, &tx, &token) {
                            log::debug!("gpui-driver connection ended: {e}");
                        }
                    });
            }
        })?;

    Ok((port, rx))
}

fn serve_connection(
    stream: TcpStream,
    tx: &mpsc::UnboundedSender<Job>,
    token: &str,
) -> std::io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = process_line(&line, tx, token);
        let mut out = serde_json::to_string(&response).unwrap_or_else(|e| {
            format!(
                r#"{{"jsonrpc":"2.0","id":0,"error":{{"code":-32099,"message":"response serialization failed: {e}"}}}}"#
            )
        });
        out.push('\n');
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Parses one request line, checks the auth token, forwards to the foreground loop,
/// and blocks until the response arrives.
pub(crate) fn process_line(
    line: &str,
    tx: &mpsc::UnboundedSender<Job>,
    token: &str,
) -> RpcResponse {
    let req: RpcRequest = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(e) => {
            return RpcResponse::error(0, ErrorKind::Internal, format!("malformed request: {e}"));
        }
    };
    let id = req.id;

    if req.params.get("token").and_then(|t| t.as_str()) != Some(token) {
        return RpcResponse::error(id, ErrorKind::AuthFailed, "missing or invalid token");
    }

    let (reply_tx, reply_rx) = oneshot::channel();
    if tx.unbounded_send((req, reply_tx)).is_err() {
        return RpcResponse::error(id, ErrorKind::Internal, "driver handler loop is gone");
    }
    match futures::executor::block_on(reply_rx) {
        Ok(response) => response,
        Err(_) => RpcResponse::error(id, ErrorKind::Internal, "request dropped by handler"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;

    /// Spins up a mock "foreground loop" thread that answers every job with its id.
    fn mock_handler() -> (mpsc::UnboundedSender<Job>, std::thread::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::unbounded::<Job>();
        let handle = std::thread::spawn(move || {
            futures::executor::block_on(async move {
                while let Some((req, reply)) = rx.next().await {
                    let _ = reply.send(RpcResponse::success(req.id, json!({"ok": req.method})));
                }
            });
        });
        (tx, handle)
    }

    #[test]
    fn round_trips_a_valid_request() {
        let (tx, _h) = mock_handler();
        let resp = process_line(
            r#"{"jsonrpc":"2.0","id":42,"method":"info","params":{"token":"secret"}}"#,
            &tx,
            "secret",
        );
        assert_eq!(resp.id, 42);
        assert_eq!(resp.result.unwrap()["ok"], "info");
    }

    #[test]
    fn rejects_bad_token() {
        let (tx, _h) = mock_handler();
        let resp = process_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"info","params":{"token":"wrong"}}"#,
            &tx,
            "secret",
        );
        assert_eq!(resp.error.unwrap().data.unwrap().kind, ErrorKind::AuthFailed);
    }

    #[test]
    fn rejects_missing_token() {
        let (tx, _h) = mock_handler();
        let resp = process_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"info","params":{}}"#,
            &tx,
            "secret",
        );
        assert_eq!(resp.error.unwrap().data.unwrap().kind, ErrorKind::AuthFailed);
    }

    #[test]
    fn reports_malformed_json() {
        let (tx, _h) = mock_handler();
        let resp = process_line("{not json", &tx, "secret");
        let err = resp.error.unwrap();
        assert_eq!(err.data.unwrap().kind, ErrorKind::Internal);
        assert!(err.message.contains("malformed"));
    }
}
