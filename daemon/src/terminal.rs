//! "Open terminal": brings the window a session runs in to the front
//! (port of the original session_window.rs + platform/linux_focus.rs).
//!
//! The relay (coucou-hook) is a grandchild of the agent, which runs in a
//! shell, which runs in the terminal or editor whose window we want. While the
//! relay is connected its process is alive, so walking up /proc from its pid
//! (SO_PEERCRED) finds that window's process. Every ancestor is kept per
//! session ID, nearest first, in memory only; which one owns a window is asked
//! when the button is clicked:
//!   * kitty, whatever the compositor, also selects the session's tab when its
//!     remote control listens on a socket (the original did this);
//!   * Hyprland (`hyprctl clients -j`) owns the windows here: the client whose
//!     pid is the nearest ancestor, the one titled after the session's folder
//!     if it has several, is focused with `hl.dsp.focus` (Lua config, 0.56+),
//!     or `focuswindow` on a hyprlang config.
//!
//! When nothing is found, the folder opens in VS Code (or the file manager),
//! as the original.
//!
//! Everything handed to a command is a number read from /proc of a process we
//! found ourselves, or an address hyprctl gave us and we checked; nothing from
//! a hook payload. Commands get their arguments one by one, never a shell.

use std::collections::HashMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// One process: its parent and its name from /proc (at most 15 bytes).
#[derive(Debug, Clone)]
pub struct Proc {
    pub parent: u32,
    pub exe: String,
}

/// Never the window of a session (the original's LINUX_TREE_TOPS): init and
/// the user's service manager, logins, session managers, and the compositors,
/// window managers and panels that own windows of their own. Reaching one
/// means the terminal was not an ancestor (tmux, screen, ssh).
const TREE_TOPS: &[&str] = &[
    "systemd", "init", "sshd", "sshd-session", "login", "su", "sudo", "doas",
    "gdm-session-wor", "gdm-x-session", "gdm-wayland-ses", "sddm", "sddm-helper",
    "lightdm", "lxdm-session", "greetd", "xinit", "xorg", "xwayland",
    "dbus-daemon", "dbus-broker", "dbus-broker-lau",
    "gnome-session-b", "gnome-session", "gnome-shell",
    "startplasma-x11", "startplasma-way", "plasma_session", "ksmserver",
    "plasmashell", "kwin_x11", "kwin_wayland",
    "xfce4-session", "xfce4-panel", "xfdesktop", "xfwm4",
    "cinnamon-sessio", "cinnamon", "cinnamon-launch", "mate-session", "mate-panel",
    "marco", "lxsession", "lxqt-session", "lxqt-panel", "lxpanel", "openbox",
    "budgie-wm", "budgie-panel", "cosmic-session", "cosmic-comp", "cosmic-panel",
    "sway", "hyprland", "niri", "labwc", "wayfire", "river", "i3", "awesome",
    "bspwm", "waybar", "noctalia", "uwsm", "start-hyprland",
];

/// How far up we look. A session sits a handful of levels below its window.
pub const MAX_DEPTH: usize = 16;

/// The ancestors of `start`, nearest first, stopping before the top of the
/// tree. `start` itself (the relay) is not included. A loop in the parent
/// links ends the walk.
pub fn ancestors(procs: &HashMap<u32, Proc>, start: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut seen = vec![start];
    let mut current = start;
    while out.len() < MAX_DEPTH {
        let Some(proc) = procs.get(&current) else { break };
        let parent = proc.parent;
        if parent == 0 || seen.contains(&parent) {
            break;
        }
        let Some(up) = procs.get(&parent) else { break };
        if TREE_TOPS.contains(&up.exe.to_ascii_lowercase().as_str()) {
            break;
        }
        out.push(parent);
        seen.push(parent);
        current = parent;
    }
    out
}

/// `/proc/<pid>/stat` → (name, parent). The name sits between the first `(`
/// and the LAST `)`: it may hold spaces and parentheses (`tmux: server`).
pub fn parse_stat(line: &str) -> Option<(String, u32)> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }
    let name = &line[open + 1..close];
    let mut rest = line[close + 1..].split_ascii_whitespace();
    let _state = rest.next()?;
    let parent = rest.next()?.parse().ok()?;
    Some((name.to_string(), parent))
}

/// One process of ours. PID 1 and processes of another user end any walk.
fn read_proc(pid: u32, uid: u32) -> Option<Proc> {
    if pid <= 1 {
        return None;
    }
    let dir = format!("/proc/{pid}");
    if std::fs::metadata(&dir).ok()?.uid() != uid {
        return None;
    }
    let stat = std::fs::read_to_string(format!("{dir}/stat")).ok()?;
    let (exe, parent) = parse_stat(&stat)?;
    Some(Proc { parent, exe })
}

/// The ancestors of a live process, nearest first, below the top of the tree.
pub fn process_ancestors(pid: u32) -> Vec<u32> {
    let uid = crate::paths::uid();
    let mut procs = HashMap::new();
    let mut current = pid;
    for _ in 0..MAX_DEPTH + 2 {
        let Some(p) = read_proc(current, uid) else { break };
        let parent = p.parent;
        procs.insert(current, p);
        if procs.contains_key(&parent) {
            break;
        }
        current = parent;
    }
    ancestors(&procs, pid)
}

/// `procs[i]` is what /proc says now of `owners[i]`. The nearest ones may have
/// exited (the `sh -c` that ran the relay does), so the chain starts at the
/// first one alive and still the child of the next, and ends where a process
/// is gone or its parent is no longer the next one: a closed terminal's pids
/// may since belong to someone else.
pub fn linked_chain(owners: &[u32], procs: &[Option<Proc>]) -> Vec<(u32, String)> {
    let linked = |i: usize| {
        procs[i].as_ref().is_some_and(|p| owners.get(i + 1).is_none_or(|next| p.parent == *next))
    };
    let n = owners.len().min(procs.len());
    let Some(start) = (0..n).find(|i| linked(*i)) else { return Vec::new() };
    let mut out = Vec::new();
    for i in start..n {
        let Some(p) = &procs[i] else { break };
        out.push((owners[i], p.exe.clone()));
        if !linked(i) {
            break;
        }
    }
    out
}

fn still_linked(owners: &[u32]) -> Vec<(u32, String)> {
    let uid = crate::paths::uid();
    let procs: Vec<Option<Proc>> = owners.iter().map(|pid| read_proc(*pid, uid)).collect();
    linked_chain(owners, &procs)
}

// ── Sessions seen so far ──────────────────────────────────────────────────────

/// Session ID → every ancestor of its relay, nearest first. Empty for a
/// session whose relay was already gone, so a later event tries again. Small:
/// the oldest session goes once there are more than this many.
const MAX_SESSIONS: usize = 64;

/// A session ID as agents send them (a UUID): at most 128 plain characters.
pub fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Default)]
pub struct Sessions {
    list: Mutex<Vec<(String, Vec<u32>)>>,
}

impl Sessions {
    pub fn remember(&self, session: &str, mut owners: Vec<u32>) {
        if !valid_session(session) || owners.is_empty() {
            return;
        }
        owners.truncate(MAX_DEPTH);
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        list.retain(|(s, _)| s != session);
        list.push((session.to_string(), owners));
        let excess = list.len().saturating_sub(MAX_SESSIONS);
        list.drain(..excess);
    }

    pub fn known(&self, session: &str) -> bool {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(s, _)| s == session)
    }

    pub fn lookup(&self, session: &str) -> Option<Vec<u32>> {
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        list.iter().find(|(s, _)| s == session).map(|(_, pids)| pids.clone())
    }

    pub fn forget(&self, session: &str) {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).retain(|(s, _)| s != session);
    }

    /// Called for every relay connection with its relay's ancestors: the
    /// first event of a session that found some settles where its window is.
    pub fn note(&self, payload: &Value, event: &str, ancestors: Vec<u32>) {
        let Some(session) = payload.get("session_id").and_then(Value::as_str) else { return };
        if event == "SessionEnd" {
            self.forget(session);
            return;
        }
        if !valid_session(session) || self.known(session) {
            return;
        }
        self.remember(session, ancestors);
    }
}

/// The control socket's answer to `window <session>`: the pids, comma-separated.
pub fn encode_pids(pids: &[u32]) -> String {
    pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

pub fn decode_pids(text: &str) -> Vec<u32> {
    text.split(',').filter_map(|p| p.trim().parse().ok()).filter(|p| *p > 1).take(MAX_DEPTH).collect()
}

// ── Hyprland ──────────────────────────────────────────────────────────────────

/// A window as `hyprctl clients -j` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    pub address: String,
    pub pid: u32,
    pub title: String,
}

/// An address hyprctl gave: `0x` and hex digits, nothing else.
fn valid_address(a: &str) -> bool {
    a.len() > 2 && a.len() <= 18 && a.starts_with("0x") && a[2..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn parse_clients(json: &str) -> Vec<Client> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter(|c| c.get("mapped").and_then(Value::as_bool) != Some(false))
        .filter_map(|c| {
            let address = c.get("address")?.as_str()?.to_string();
            let pid = u32::try_from(c.get("pid")?.as_i64()?).ok()?;
            let title = c.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
            valid_address(&address).then_some(Client { address, pid, title })
        })
        .collect()
}

/// The window to bring forward: the windows of the nearest ancestor that has
/// any, and among them the one titled after the session's folder, else the
/// first (the original's choose_window / pick_window).
pub fn choose_window<'a>(ancestors: &[u32], clients: &'a [Client], folder: &str) -> Option<&'a Client> {
    let folder = folder.to_lowercase();
    ancestors.iter().find_map(|pid| {
        let mine: Vec<&Client> = clients.iter().filter(|c| c.pid == *pid).collect();
        let named = (!folder.is_empty())
            .then(|| mine.iter().find(|c| c.title.to_lowercase().contains(&folder)).copied())
            .flatten();
        named.or_else(|| mine.first().copied())
    })
}

/// The Lua-config dispatcher (Hyprland 0.56+), then the hyprlang one.
pub fn focus_commands(address: &str) -> [Vec<String>; 2] {
    [
        vec!["dispatch".into(), format!("hl.dsp.focus({{window = \"address:{address}\"}})")],
        vec!["dispatch".into(), "focuswindow".into(), format!("address:{address}")],
    ]
}

/// hyprctl says `ok` when a dispatch worked; errors and warnings are text.
pub fn dispatch_ok(stdout: &str) -> bool {
    stdout.trim() == "ok"
}

fn hyprctl(args: &[String]) -> Option<String> {
    let out = run_capture(Command::new("hyprctl").args(args), Duration::from_secs(2))?;
    Some(out)
}

fn hyprland_focus(owners: &[u32], folder: &str) -> bool {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return false;
    }
    let Some(json) = hyprctl(&["clients".into(), "-j".into()]) else { return false };
    let clients = parse_clients(&json);
    let Some(target) = choose_window(owners, &clients, folder) else { return false };
    focus_commands(&target.address)
        .iter()
        .any(|args| hyprctl(args).is_some_and(|out| dispatch_ok(&out)))
}

// ── kitty ─────────────────────────────────────────────────────────────────────

/// `KITTY_LISTEN_ON` from a process environment (NUL-separated), when it is a
/// Unix socket address made of plain path characters.
pub fn kitty_listen_on(environ: &[u8]) -> Option<String> {
    let value = environ.split(|b| *b == 0).find_map(|entry| entry.strip_prefix(b"KITTY_LISTEN_ON="))?;
    let value = std::str::from_utf8(value).ok()?;
    let path = value.strip_prefix("unix:")?;
    let plain = |c: char| c.is_ascii_alphanumeric() || "/_.-@+:".contains(c);
    (!path.is_empty() && path.len() <= 107 && path.chars().all(plain)).then(|| value.to_string())
}

/// The shell kitty runs for the session: the process right below kitty.
pub fn kitty_shell(chain: &[(u32, String)]) -> Option<(u32, u32)> {
    let at = chain.iter().position(|(_, name)| name == "kitty")?;
    let (shell, _) = at.checked_sub(1).and_then(|i| chain.get(i))?;
    Some((*shell, chain[at].0))
}

fn executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| executable(p))
}

/// Focuses the session's kitty tab and window when kitty's remote control
/// listens on a socket. Quietly nothing otherwise.
fn kitty_focus_tab(chain: &[(u32, String)]) -> bool {
    let Some((shell, kitty_pid)) = kitty_shell(chain) else { return false };
    let Ok(environ) = std::fs::read(format!("/proc/{shell}/environ")) else { return false };
    let Some(to) = kitty_listen_on(&environ) else { return false };
    let beside = std::fs::read_link(format!("/proc/{kitty_pid}/exe"))
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("kitten")))
        .filter(|p| executable(p));
    let Some(kitten) = beside.or_else(|| find_on_path("kitten")) else { return false };
    run_quietly(
        Command::new(kitten).args(["@", "--to", &to, "focus-window", "--match", &format!("pid:{shell}")]),
        Duration::from_secs(2),
    )
}

// ── Running helpers ───────────────────────────────────────────────────────────

fn wait_with_limit(child: &mut std::process::Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Runs a helper with nothing attached and a time limit; true when it exits 0.
fn run_quietly(cmd: &mut Command, limit: Duration) -> bool {
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    wait_with_limit(&mut child, limit).is_some_and(|s| s.success())
}

/// Runs a helper and returns its stdout when it exits 0 in time.
fn run_capture(cmd: &mut Command, limit: Duration) -> Option<String> {
    use std::io::Read;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.by_ref().take(8 << 20).read_to_end(&mut buf);
        buf
    });
    let status = wait_with_limit(&mut child, limit)?;
    let buf = reader.join().ok()?;
    status.success().then(|| String::from_utf8_lossy(&buf).into_owned())
}

// ── The button ────────────────────────────────────────────────────────────────

/// The last folder name of a path.
pub fn folder_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

/// Brings forward the window a session remembered. False when no way worked.
pub fn focus_session_window(owners: &[u32], folder: &str) -> bool {
    let chain = still_linked(owners);
    if chain.is_empty() {
        return false;
    }
    let pids: Vec<u32> = chain.iter().map(|(pid, _)| *pid).collect();
    let tab = kitty_focus_tab(&chain);
    let raised = hyprland_focus(&pids, folder);
    tab || raised
}

/// The original's fallback: the session's folder in VS Code, else in the file
/// manager. Only an existing folder given by its full path goes any further
/// (`code` would read `--x` as an option, xdg-open would run a file).
pub fn open_folder(path: &str) -> bool {
    let p = Path::new(path);
    if !(p.is_absolute() && p.is_dir()) {
        return false;
    }
    let spawn = |cmd: &mut Command| cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();
    if let Some(code) = find_on_path("code") {
        if spawn(Command::new(code).arg(p)) {
            return true;
        }
    }
    find_on_path("xdg-open").is_some_and(|open| spawn(Command::new(open).arg(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[(u32, u32, &str)]) -> HashMap<u32, Proc> {
        entries.iter().map(|(pid, parent, exe)| (*pid, Proc { parent: *parent, exe: exe.to_string() })).collect()
    }

    #[test]
    fn kitty_is_found_above_the_shells_and_the_compositor_ends_the_walk() {
        // Hyprland → kitty → fish → claude → sh → coucou-hook
        let procs = tree(&[
            (50, 1, "Hyprland"),
            (200, 50, "kitty"),
            (300, 200, "fish"),
            (400, 300, "claude"),
            (500, 400, "sh"),
            (600, 500, "coucou-hook"),
        ]);
        assert_eq!(ancestors(&procs, 600), [500, 400, 300, 200]);
    }

    #[test]
    fn tmux_stops_at_the_service_manager() {
        let procs = tree(&[(10, 1, "systemd"), (100, 10, "tmux: server"), (300, 100, "bash"), (600, 300, "coucou-hook")]);
        assert_eq!(ancestors(&procs, 600), [300, 100]);
    }

    #[test]
    fn a_relay_already_gone_or_a_loop_ends_the_walk() {
        assert!(ancestors(&tree(&[]), 600).is_empty());
        let looped = tree(&[(1, 2, "a"), (2, 1, "b"), (3, 1, "coucou-hook")]);
        assert_eq!(ancestors(&looped, 3), [1, 2]);
        let deep: Vec<(u32, u32, &str)> = (1..100).map(|i| (i, i + 1, "x")).collect();
        assert_eq!(ancestors(&tree(&deep), 1).len(), MAX_DEPTH);
    }

    #[test]
    fn stat_names_with_spaces_and_parentheses() {
        assert_eq!(parse_stat("42 (tmux: server) S 7 42 42"), Some(("tmux: server".into(), 7)));
        assert_eq!(parse_stat("42 (a) b (c) R 9 1"), Some(("a) b (c".into(), 9)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn the_chain_starts_at_the_first_live_link_and_stops_at_a_reused_pid() {
        let p = |parent: u32, exe: &str| Some(Proc { parent, exe: exe.into() });
        // sh (500) exited; claude, fish, kitty still linked.
        let owners = [500, 400, 300, 200];
        let procs = [None, p(300, "claude"), p(200, "fish"), p(50, "kitty")];
        assert_eq!(linked_chain(&owners, &procs), [(400, "claude".into()), (300, "fish".into()), (200, "kitty".into())]);
        // fish's pid now belongs to something else: the chain ends there.
        let reused = [None, p(300, "claude"), p(77, "vim"), p(50, "kitty")];
        assert_eq!(linked_chain(&owners, &reused), [(400, "claude".into()), (300, "vim".into())]);
        assert!(linked_chain(&owners, &[None, None, None, None]).is_empty());
    }

    #[test]
    fn sessions_are_kept_valid_and_bounded() {
        let s = Sessions::default();
        s.remember("s-1", vec![42]);
        s.remember("s-1", vec![43, 44]);
        assert_eq!(s.lookup("s-1"), Some(vec![43, 44]));
        s.remember("not a session/../x", vec![7]);
        assert!(!s.known("not a session/../x"));
        s.remember("s-2", Vec::new());
        assert!(!s.known("s-2"), "an empty walk is tried again next event");
        for i in 0..70 {
            s.remember(&format!("x-{i}"), vec![9]);
        }
        assert!(!s.known("s-1") && s.known("x-69"));
        s.forget("x-69");
        assert_eq!(s.lookup("x-69"), None);
    }

    #[test]
    fn session_end_forgets_and_our_own_process_is_walked() {
        let s = Sessions::default();
        s.remember("s-end", vec![5]);
        s.note(&serde_json::json!({"session_id": "s-end"}), "SessionEnd", vec![5]);
        assert!(!s.known("s-end"));
        s.note(&serde_json::json!({"session_id": "s-new"}), "PreToolUse", vec![7, 8]);
        s.note(&serde_json::json!({"session_id": "s-new"}), "PreToolUse", vec![9]);
        assert_eq!(s.lookup("s-new"), Some(vec![7, 8]), "settled by the first event that found some");
        let mine = process_ancestors(std::process::id());
        assert!(mine.is_empty() || mine[0] == std::os::unix::process::parent_id());
    }

    #[test]
    fn pids_travel_as_a_comma_list() {
        assert_eq!(encode_pids(&[3, 20, 100]), "3,20,100");
        assert_eq!(decode_pids("3,20, 100,x,1,0"), [3, 20, 100]);
        assert!(decode_pids("").is_empty());
    }

    const CLIENTS: &str = r#"[
      {"address":"0x55ab9f611c50","mapped":true,"pid":228679,"title":"Firefox"},
      {"address":"0x55ab9ff16430","mapped":true,"pid":200,"title":"tuf: api"},
      {"address":"0x55ab9ff20000","mapped":true,"pid":200,"title":"tuf: coucou"},
      {"address":"0x1;rm","mapped":true,"pid":300,"title":"evil"},
      {"address":"0x55ab90000001","mapped":false,"pid":300,"title":"hidden"}
    ]"#;

    #[test]
    fn hyprland_clients_are_parsed_and_checked() {
        let clients = parse_clients(CLIENTS);
        assert_eq!(clients.len(), 3, "bad address and unmapped windows are left out");
        assert!(parse_clients("not json").is_empty());
    }

    #[test]
    fn the_nearest_ancestor_window_titled_after_the_folder_wins() {
        let clients = parse_clients(CLIENTS);
        assert_eq!(choose_window(&[400, 300, 200], &clients, "coucou").unwrap().address, "0x55ab9ff20000");
        assert_eq!(choose_window(&[400, 300, 200], &clients, "elsewhere").unwrap().address, "0x55ab9ff16430");
        assert_eq!(choose_window(&[400, 300, 200], &clients, "").unwrap().address, "0x55ab9ff16430");
        assert!(choose_window(&[1, 2], &clients, "coucou").is_none());
    }

    #[test]
    fn focus_goes_through_the_lua_dispatcher_first() {
        let [lua, legacy] = focus_commands("0xabc");
        assert_eq!(lua, ["dispatch", "hl.dsp.focus({window = \"address:0xabc\"})"]);
        assert_eq!(legacy, ["dispatch", "focuswindow", "address:0xabc"]);
        assert!(dispatch_ok("ok\n"));
        assert!(!dispatch_ok("warning: =[C]:-1: hl.focus: window not found"));
    }

    #[test]
    fn kitty_socket_and_shell() {
        assert_eq!(kitty_listen_on(b"A=1\0KITTY_LISTEN_ON=unix:/tmp/kitty-12\0"), Some("unix:/tmp/kitty-12".into()));
        assert_eq!(kitty_listen_on(b"KITTY_LISTEN_ON=unix:/tmp/a b\0"), None);
        assert_eq!(kitty_listen_on(b"KITTY_LISTEN_ON=tcp:localhost:1\0"), None);
        let chain = [(400, "claude".to_string()), (300, "fish".to_string()), (200, "kitty".to_string())];
        assert_eq!(kitty_shell(&chain), Some((300, 200)));
        assert_eq!(kitty_shell(&chain[2..]), None);
    }

    /// Live, on Hyprland: focuses the window that already has the focus (a
    /// no-op for the user) through the same path the button takes.
    /// `cargo test -- --ignored live_hyprland_focus`
    #[test]
    #[ignore]
    fn live_hyprland_focus_of_the_active_window() {
        let active = run_capture(Command::new("hyprctl").args(["activewindow", "-j"]), Duration::from_secs(2)).unwrap();
        let v: Value = serde_json::from_str(&active).unwrap();
        let (pid, address) = (v["pid"].as_u64().unwrap() as u32, v["address"].as_str().unwrap().to_string());
        assert!(hyprland_focus(&[pid], ""), "dispatch answered ok");
        let after = run_capture(Command::new("hyprctl").args(["activewindow", "-j"]), Duration::from_secs(2)).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["address"].as_str().unwrap(), address);
        assert!(!hyprland_focus(&[2], ""), "no window for a pid without one");
    }

    #[test]
    fn folders() {
        assert_eq!(folder_name("/home/u/coucou/"), "coucou");
        assert!(!open_folder("relative/path"));
        assert!(!open_folder("/definitely/not/here"));
    }
}
