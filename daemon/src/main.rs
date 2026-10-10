//! coucou-daemon — the bridge between coucou-hook and the Noctalia plugin.
//!
//! ```text
//! coucou-daemon                       serve both sockets (default)
//! coucou-daemon --follow              print one JSON line per event (plugin stream)
//! coucou-daemon --status-json         print the status snapshot
//! coucou-daemon ack <id>              the card is on screen
//! coucou-daemon decline <id>          nobody can act on it, hand back to the terminal
//! coucou-daemon decide <id> <allow|deny|answers-json>
//! coucou-daemon install-hooks [--agent <id>]... [--remove] [--yes]
//!               [--status | --dry-run] [--json] [--home <dir>] [--hook-path <path>]
//!               [--expect-sha256 <path>=<sha256>]...
//! coucou-daemon secret has|set|delete <provider|key> [--bound-url <url>] [--from-file <path>]   (set reads stdin)
//! coucou-daemon chat --request <file>  one chat turn, streamed as JSON lines (chat.rs)
//! coucou-daemon chat-models <provider> [--base-url <url>]
//! coucou-daemon open-terminal <session-id> [--cwd <path>]   focus the session's terminal (terminal.rs)
//! ```
//!
//! `COUCOU_ACK_TIMEOUT_MS` and `COUCOU_DECISION_TIMEOUT_MS` override the
//! 800 ms / 108 s waits (for testing).

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

mod chat;
mod hub;
mod install;
mod net;
mod paths;
mod protocol;
mod secrets;
mod server;
mod terminal;

const USAGE: &str = "usage: coucou-daemon [--follow | --status-json | ack <id> | decline <id> | decide <id> <allow|deny|answers-json>]
       coucou-daemon install-hooks [--agent <id>]... [--remove] [--yes] [--status | --dry-run] [--json] [--home <dir>] [--hook-path <path>] [--expect-sha256 <path>=<sha256>]...
       coucou-daemon secret has|set|delete <provider|key> [--bound-url <url>] [--from-file <path>]
       coucou-daemon chat --request <file> | chat-models <provider> [--base-url <url>]
       coucou-daemon open-terminal <session-id> [--cwd <path>]
       agents: claude, codex, copilot, muse, gemini, antigravity, cursor, opencode, amp, hermes";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = paths::runtime_dir();
    let control = paths::control_socket(&dir);
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(&dir),
        Some("--follow") => follow(&control),
        Some("--status-json") => {
            println!("{}", server::status_or_offline(&control));
            ExitCode::SUCCESS
        }
        Some(verb @ ("ack" | "decline" | "decide")) => send(&control, &format!("{verb} {}", args[1..].join(" "))),
        Some("install-hooks") => install_hooks(&args[1..]),
        Some("secret") => secret(&args[1..]),
        Some("chat") => exit(chat::run(&args[1..], &mut std::io::stdout())),
        Some("chat-models") => exit(chat::run_models(&args[1..], &mut std::io::stdout())),
        Some("open-terminal") => open_terminal(&control, &args[1..]),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown argument: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn install_hooks(args: &[String]) -> ExitCode {
    let opts = match install::parse_args(args) {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("coucou-daemon: {err}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let stdin = std::io::stdin();
    match install::run(&opts, &mut stdin.lock(), &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("coucou-daemon: {err}");
            ExitCode::FAILURE
        }
    }
}

/// `open-terminal <session-id> [--cwd <path>]`: asks the daemon where the
/// session's relay ran, then focuses that terminal (kitty tab, Hyprland
/// window). Runs here, in the caller's session (the shell has Hyprland's
/// environment, a systemd service may not). Nothing found: the folder opens in
/// VS Code or the file manager, as the original. Exit 0 when something opened.
fn open_terminal(control: &Path, args: &[String]) -> ExitCode {
    let mut session = None;
    let mut cwd = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--cwd" => cwd = it.next().cloned(),
            s if session.is_none() && !s.starts_with('-') => session = Some(s.to_string()),
            other => {
                eprintln!("coucou-daemon: unexpected argument: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(session) = session else {
        eprintln!("coucou-daemon: open-terminal needs a session id\n{USAGE}");
        return ExitCode::from(2);
    };
    let folder = cwd.as_deref().map(terminal::folder_name).unwrap_or_default().to_string();
    let owners = match server::request(control, &format!("window {session}")) {
        Ok(answer) => answer.strip_prefix("ok ").map(terminal::decode_pids).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    if !owners.is_empty() && terminal::focus_session_window(&owners, &folder) {
        return ExitCode::SUCCESS;
    }
    exit(cwd.as_deref().is_some_and(terminal::open_folder))
}

fn exit(ok: bool) -> ExitCode {
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn secret(args: &[String]) -> ExitCode {
    let stdin = std::io::stdin();
    match secrets::run(args, &mut stdin.lock(), &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("coucou-daemon: {err}");
            ExitCode::FAILURE
        }
    }
}

fn env_ms(key: &str) -> Option<Duration> {
    std::env::var(key).ok()?.parse::<u64>().ok().map(Duration::from_millis)
}

fn serve(dir: &Path) -> ExitCode {
    if !paths::is_private_dir(dir) {
        eprintln!("coucou-daemon: {} is not a private runtime directory (0700, owned by you)", dir.display());
        return ExitCode::FAILURE;
    }
    let defaults = hub::Timeouts::default();
    let timeouts = hub::Timeouts {
        ack: env_ms("COUCOU_ACK_TIMEOUT_MS").unwrap_or(defaults.ack),
        decision: env_ms("COUCOU_DECISION_TIMEOUT_MS").unwrap_or(defaults.decision),
    };
    let relay = paths::relay_socket(dir);
    let control = paths::control_socket(dir);
    if let Err(err) = server::start(&relay, &control, timeouts) {
        eprintln!("coucou-daemon: {err}");
        return ExitCode::FAILURE;
    }
    eprintln!("coucou-daemon: listening on {} and {}", relay.display(), control.display());
    loop {
        std::thread::park();
    }
}

/// The plugin's stream. Never exits on its own while stdout is open: while the
/// daemon is away it says so every few seconds (which doubles as a heartbeat)
/// and retries, so the plugin keeps one long-lived process.
fn follow(control: &Path) -> ExitCode {
    use std::io::Write;
    let mut out = std::io::stdout();
    loop {
        if let Err(err) = server::follow(control, &mut out) {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
        }
        if writeln!(out, r#"{{"type":"daemon","running":false}}"#).and_then(|_| out.flush()).is_err() {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn send(control: &Path, command: &str) -> ExitCode {
    match server::request(control, command) {
        Ok(answer) if answer == "ok" => ExitCode::SUCCESS,
        Ok(answer) => {
            eprintln!("coucou-daemon: {answer}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("coucou-daemon: daemon not reachable: {err}");
            ExitCode::FAILURE
        }
    }
}
