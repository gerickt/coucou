use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MochiStatus {
    pub state: String,
    pub agent: String,
    pub tool: Option<String>,
    pub active: bool,
    pub last_updated: u64,
}

impl Default for MochiStatus {
    fn default() -> Self {
        Self {
            state: "idle".to_string(),
            agent: "Claude Code".to_string(),
            tool: None,
            active: false,
            last_updated: 0,
        }
    }
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    dir.join("coucou.sock")
}

fn state_file_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    dir.join("coucou-state.json")
}

fn print_status() {
    let file = state_file_path();
    if let Ok(content) = fs::read_to_string(&file) {
        if let Ok(status) = serde_json::from_str::<MochiStatus>(&content) {
            println!("{}", serde_json::to_string(&status).unwrap());
            return;
        }
    }
    println!("{}", serde_json::to_string(&MochiStatus::default()).unwrap());
}

fn handle_connection(stream: UnixStream, status: Arc<Mutex<MochiStatus>>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_ok() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
            let event = json.get("event").and_then(|v| v.as_str()).unwrap_or("");
            let tool = json.get("tool_name").and_then(|v| v.as_str()).map(|s| s.to_string());
            let agent = json.get("agent").and_then(|v| v.as_str()).unwrap_or("Claude Code").to_string();

            let mut st = status.lock().unwrap();
            st.agent = agent;
            st.last_updated = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            match event {
                "PreToolUse" => {
                    st.state = "running".to_string();
                    st.tool = tool;
                    st.active = true;
                }
                "PermissionRequest" => {
                    st.state = "permission".to_string();
                    st.tool = tool;
                    st.active = true;
                }
                "UserPromptSubmit" => {
                    st.state = "thinking".to_string();
                    st.tool = None;
                    st.active = true;
                }
                "Stop" | "PostToolUse" => {
                    st.state = "idle".to_string();
                    st.tool = None;
                    st.active = false;
                }
                _ => {}
            }

            let _ = fs::write(state_file_path(), serde_json::to_string(&*st).unwrap_or_default());
        }
    }
}

fn run_daemon() {
    let path = socket_path();
    let _ = fs::remove_file(&path);

    let listener = UnixListener::bind(&path).expect("Could not bind Unix socket");
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o700));

    let status = Arc::new(Mutex::new(MochiStatus::default()));
    let _ = fs::write(state_file_path(), serde_json::to_string(&MochiStatus::default()).unwrap_or_default());

    println!("coucou-daemon listening on {:?}", path);

    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let status_clone = Arc::clone(&status);
            std::thread::spawn(move || {
                handle_connection(stream, status_clone);
            });
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--status-json") {
        print_status();
    } else {
        run_daemon();
    }
}
