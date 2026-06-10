//! `gpui-driver` — drive a running, instrumented GPUI app from the command line.
//!
//! Exit codes (stable, scripting/agent contract):
//!   0  success
//!   2  element or window not found (also: element not visible/occluded)
//!   3  timeout (`wait-idle`)
//!   4  no instrumented app found / ambiguous selection / stale discovery
//!   5  protocol, transport, or auth error

mod client;
mod output;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gpui_driver_protocol::{ErrorKind, TreeResult};
use serde_json::json;

use client::CliError;

#[derive(Parser)]
#[command(
    name = "gpui-driver",
    version,
    about = "Drive a running GPUI app: inspect, click, screenshot"
)]
struct Cli {
    /// Select the app by name (defaults to the only running instrumented app).
    #[arg(long, global = true)]
    app: Option<String>,

    /// Select the app by process id.
    #[arg(long, global = true)]
    pid: Option<u32>,

    /// Emit raw JSON (default when stdout is not a terminal).
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List discovered instrumented apps and their liveness.
    List,
    /// Show app + driver version info.
    Info,
    /// List the app's windows.
    Windows,
    /// Dump the element tree of a window.
    Tree {
        #[arg(long, default_value_t = 0)]
        window: u64,
        /// Only show interactive elements (and their ancestors).
        #[arg(long)]
        interactive_only: bool,
    },
    /// Click an element by driver id.
    Click {
        id: String,
        #[arg(long, default_value_t = 0)]
        window: u64,
        #[arg(long, default_value = "left", value_parser = ["left", "right", "middle"])]
        button: String,
        /// Modifiers to hold, e.g. --modifier ctrl --modifier shift.
        #[arg(long = "modifier", value_parser = ["ctrl", "alt", "shift", "cmd", "fn"])]
        modifiers: Vec<String>,
    },
    /// Capture a screenshot of a window (works while occluded or session-locked).
    Screenshot {
        /// Output PNG path.
        #[arg(short, long, default_value = "shot.png")]
        output: PathBuf,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Wait until the window's rendered output stops changing.
    WaitIdle {
        #[arg(long, default_value_t = 5000)]
        timeout: u64,
        #[arg(long, default_value_t = 150)]
        quiet: u64,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Type text into the focused element.
    Type {
        text: String,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Send a keystroke combo, e.g. `ctrl-s` or `enter`.
    Key {
        combo: String,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Scroll an element by driver id.
    Scroll {
        id: String,
        #[arg(long, default_value_t = 0.0)]
        delta_x: f32,
        #[arg(long, default_value_t = -120.0, allow_hyphen_values = true)]
        delta_y: f32,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Focus an element by driver id.
    Focus {
        id: String,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
    /// Find elements without dumping the whole tree.
    Query {
        /// Match elements whose text contains this string.
        #[arg(long)]
        text_contains: Option<String>,
        /// Match elements whose driver id contains this string.
        #[arg(long)]
        id_contains: Option<String>,
        #[arg(long, default_value_t = 0)]
        window: u64,
    },
}

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json || !std::io::stdout().is_terminal();
    std::process::exit(run(cli, json_mode));
}

const EXIT_OK: i32 = 0;
const EXIT_NOT_FOUND: i32 = 2;
const EXIT_TIMEOUT: i32 = 3;
const EXIT_NO_APP: i32 = 4;
const EXIT_PROTOCOL: i32 = 5;

fn exit_code(err: &CliError) -> i32 {
    match err {
        CliError::NoApp(_) => EXIT_NO_APP,
        CliError::Protocol(_) => EXIT_PROTOCOL,
        CliError::Rpc { kind, .. } => match kind {
            Some(ErrorKind::ElementNotFound)
            | Some(ErrorKind::ElementNotVisible)
            | Some(ErrorKind::WindowNotFound) => EXIT_NOT_FOUND,
            Some(ErrorKind::Timeout) => EXIT_TIMEOUT,
            _ => EXIT_PROTOCOL,
        },
    }
}

fn run(cli: Cli, json_mode: bool) -> i32 {
    match execute(&cli, json_mode) {
        Ok(code) => code,
        Err(err) => {
            if json_mode {
                let kind = match &err {
                    CliError::Rpc { kind, .. } => {
                        kind.map(|k| serde_json::to_value(k).unwrap_or_default())
                    }
                    CliError::NoApp(_) => Some(json!("no_app")),
                    CliError::Protocol(_) => Some(json!("protocol")),
                };
                eprintln!("{}", json!({ "error": err.to_string(), "kind": kind }));
            } else {
                eprintln!("error: {err}");
            }
            exit_code(&err)
        }
    }
}

fn execute(cli: &Cli, json_mode: bool) -> Result<i32, CliError> {
    let dir = client::discovery_dir();

    if let Command::List = cli.command {
        let apps = client::scan_with_liveness(&dir);
        if json_mode {
            let list: Vec<_> = apps
                .iter()
                .map(|(d, alive)| {
                    json!({
                        "app_name": d.file.app_name,
                        "pid": d.file.pid,
                        "port": d.file.port,
                        "started_at": d.file.started_at,
                        "alive": alive,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "apps": list })).unwrap()
            );
        } else if apps.is_empty() {
            println!("no instrumented apps discovered (dir: {})", dir.display());
        } else {
            for (d, alive) in &apps {
                println!(
                    "{}\tpid {}\tport {}\tstarted {}\t{}",
                    d.file.app_name,
                    d.file.pid,
                    d.file.port,
                    d.file.started_at,
                    if *alive { "alive" } else { "stale (removed)" },
                );
            }
        }
        return Ok(EXIT_OK);
    }

    let (_discovered, mut client) = client::select_app(&dir, cli.app.as_deref(), cli.pid)?;

    let (method, params): (&str, serde_json::Value) = match &cli.command {
        Command::List => unreachable!("handled above"),
        Command::Info => ("info", json!({})),
        Command::Windows => ("windows", json!({})),
        Command::Tree { window, .. } => ("tree", json!({ "window_id": window })),
        Command::Click {
            id,
            window,
            button,
            modifiers,
        } => (
            "click",
            json!({
                "window_id": window,
                "id": id,
                "button": button,
                "modifiers": modifiers,
            }),
        ),
        Command::Screenshot { window, .. } => ("screenshot", json!({ "window_id": window })),
        Command::WaitIdle {
            timeout,
            quiet,
            window,
        } => (
            "wait_idle",
            json!({ "window_id": window, "timeout_ms": timeout, "quiet_ms": quiet }),
        ),
        Command::Type { text, window } => {
            ("type_text", json!({ "window_id": window, "text": text }))
        }
        Command::Key { combo, window } => ("key", json!({ "window_id": window, "combo": combo })),
        Command::Scroll {
            id,
            delta_x,
            delta_y,
            window,
        } => (
            "scroll",
            json!({ "window_id": window, "id": id, "delta_x": delta_x, "delta_y": delta_y }),
        ),
        Command::Focus { id, window } => ("focus", json!({ "window_id": window, "id": id })),
        Command::Query {
            text_contains,
            id_contains,
            window,
        } => (
            "query",
            json!({
                "window_id": window,
                "text_contains": text_contains,
                "id_contains": id_contains,
            }),
        ),
    };

    let result = client.call(method, params)?;
    render(cli, json_mode, result)
}

fn render(cli: &Cli, json_mode: bool, result: serde_json::Value) -> Result<i32, CliError> {
    // Screenshot writes the PNG regardless of output mode.
    if let Command::Screenshot { output, .. } = &cli.command {
        let data = result
            .get("data_base64")
            .and_then(|d| d.as_str())
            .ok_or_else(|| CliError::Protocol("screenshot response missing data".into()))?;
        use base64::Engine as _;
        let png = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CliError::Protocol(format!("invalid base64 image data: {e}")))?;
        std::fs::write(output, &png).map_err(|e| {
            CliError::Protocol(format!("failed to write {}: {e}", output.display()))
        })?;
        // Older servers omit `method`; treat that as the renderer path.
        let method = result["method"].as_str().unwrap_or("renderer");
        if method == "printwindow" {
            eprintln!(
                "warning: captured via the PrintWindow fallback — the image may be stale or \
                 black while the window is occluded, minimized, or the session is locked. \
                 Apply the vendored gpui_windows patch for reliable capture (see README)."
            );
        }
        if json_mode {
            println!(
                "{}",
                json!({
                    "path": output.display().to_string(),
                    "width": result["width"],
                    "height": result["height"],
                    "scale": result["scale"],
                    "method": method,
                })
            );
        } else {
            println!(
                "wrote {} ({}x{}, scale {}, method {})",
                output.display(),
                result["width"],
                result["height"],
                result["scale"],
                method,
            );
        }
        return Ok(EXIT_OK);
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        return Ok(EXIT_OK);
    }

    match &cli.command {
        Command::Tree {
            interactive_only, ..
        } => {
            let tree: TreeResult = serde_json::from_value(result)
                .map_err(|e| CliError::Protocol(format!("unexpected tree response: {e}")))?;
            print!("{}", output::render_tree(&tree.tree, *interactive_only));
        }
        Command::Windows => {
            for w in result["windows"].as_array().into_iter().flatten() {
                println!(
                    "window {}\t{:?}\t[{},{} {}x{}]{}",
                    w["window_id"],
                    w["title"].as_str().unwrap_or(""),
                    w["bounds"]["x"],
                    w["bounds"]["y"],
                    w["bounds"]["w"],
                    w["bounds"]["h"],
                    if w["active"].as_bool() == Some(true) {
                        " (active)"
                    } else {
                        ""
                    },
                );
            }
        }
        Command::Info => {
            println!(
                "{} {} (protocol v{}, gpui-driver {})",
                result["app_name"].as_str().unwrap_or("?"),
                result["app_version"].as_str().unwrap_or("?"),
                result["protocol_version"],
                result["gpui_driver_version"].as_str().unwrap_or("?"),
            );
        }
        Command::Click { id, .. } => {
            let b = &result["resolved_bounds"];
            println!(
                "clicked {id} at center of [{},{} {}x{}]",
                b["x"], b["y"], b["w"], b["h"]
            );
        }
        Command::WaitIdle { .. } => {
            println!("idle after {} ms", result["waited_ms"]);
        }
        Command::Type { text, .. } => println!("typed {text:?}"),
        Command::Key { combo, .. } => println!("sent {combo}"),
        Command::Scroll { id, .. } => println!("scrolled {id}"),
        Command::Focus { id, .. } => println!("focused {id}"),
        Command::Query { .. } => {
            for m in result["matches"].as_array().into_iter().flatten() {
                println!(
                    "{} <{}>{} [{},{} {}x{}]",
                    m["id"].as_str().unwrap_or("(anonymous)"),
                    m["kind"].as_str().unwrap_or("?"),
                    m["text"]
                        .as_str()
                        .map(|t| format!(" {t:?}"))
                        .unwrap_or_default(),
                    m["bounds"]["x"],
                    m["bounds"]["y"],
                    m["bounds"]["w"],
                    m["bounds"]["h"],
                );
            }
        }
        Command::List | Command::Screenshot { .. } => unreachable!("handled above"),
    }
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_design_contract() {
        let rpc = |kind| CliError::Rpc {
            kind: Some(kind),
            message: String::new(),
        };
        assert_eq!(exit_code(&rpc(ErrorKind::ElementNotFound)), 2);
        assert_eq!(exit_code(&rpc(ErrorKind::ElementNotVisible)), 2);
        assert_eq!(exit_code(&rpc(ErrorKind::WindowNotFound)), 2);
        assert_eq!(exit_code(&rpc(ErrorKind::Timeout)), 3);
        assert_eq!(exit_code(&CliError::NoApp(String::new())), 4);
        assert_eq!(exit_code(&rpc(ErrorKind::AuthFailed)), 5);
        assert_eq!(exit_code(&rpc(ErrorKind::Unsupported)), 5);
        assert_eq!(exit_code(&rpc(ErrorKind::Internal)), 5);
        assert_eq!(exit_code(&CliError::Protocol(String::new())), 5);
    }

    #[test]
    fn cli_parses_typical_agent_invocations() {
        Cli::try_parse_from(["gpui-driver", "list"]).unwrap();
        Cli::try_parse_from(["gpui-driver", "tree", "--app", "demo", "--interactive-only"])
            .unwrap();
        Cli::try_parse_from(["gpui-driver", "click", "save_button", "--app", "demo"]).unwrap();
        Cli::try_parse_from([
            "gpui-driver",
            "screenshot",
            "--app",
            "demo",
            "-o",
            "out.png",
        ])
        .unwrap();
        Cli::try_parse_from(["gpui-driver", "wait-idle", "--timeout", "2000"]).unwrap();
        Cli::try_parse_from(["gpui-driver", "type", "hello world"]).unwrap();
        Cli::try_parse_from(["gpui-driver", "key", "ctrl-s"]).unwrap();
        Cli::try_parse_from(["gpui-driver", "query", "--text-contains", "Save"]).unwrap();
    }
}
