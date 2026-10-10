//! `coucou-daemon install-hooks`: routes each agent's hooks to coucou-hook.
//!
//! The rule is strict: read the agent's config, merge without touching anybody
//! else's entries, show the diff, take a dated backup, and write only after an
//! explicit "yes" (or `--yes`). `--remove` takes Coucou's entries out and
//! nothing else. A config whose `hooks` has an unexpected shape is refused,
//! never replaced.

use std::io::{self, BufRead, Write};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// Marker that identifies a Coucou entry: the relay's file name.
const MARKER: &str = "coucou-hook";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Agent {
    Claude,
    Gemini,
    Antigravity,
    Codex,
}

impl Agent {
    pub const ALL: &'static [Agent] = &[Agent::Claude, Agent::Gemini, Agent::Antigravity, Agent::Codex];

    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.iter().copied().find(|a| a.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Gemini => "gemini",
            Agent::Antigravity => "antigravity",
            Agent::Codex => "codex",
        }
    }

    /// The `--agent` tag the relay gets ("" for Claude Code, by contract).
    fn tag(self) -> &'static str {
        match self {
            Agent::Claude => "",
            other => other.id(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Gemini => "Gemini CLI",
            Agent::Antigravity => "Antigravity",
            Agent::Codex => "Codex",
        }
    }

    /// Whether Mochi can allow or deny this agent's permission requests. Must
    /// match `takes_decisions` in the relay (hook/src/reply.rs).
    pub fn approvals(self) -> bool {
        matches!(self, Agent::Claude | Agent::Codex)
    }

    /// The agent's own binaries and folders: any of them means it is installed.
    fn traces(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Agent::Claude => (&["claude"], &[".claude"]),
            Agent::Gemini => (&["gemini"], &[".gemini"]),
            Agent::Antigravity => (&["antigravity"], &[".antigravity", ".gemini/antigravity"]),
            Agent::Codex => (&["codex"], &[".codex"]),
        }
    }

    /// The config file Coucou edits (the first of `files`).
    pub fn file(self, home: &Path) -> PathBuf {
        self.files(home).remove(0)
    }

    /// Every file Coucou writes for this agent.
    pub fn files(self, home: &Path) -> Vec<PathBuf> {
        match self {
            Agent::Claude => vec![home.join(".claude").join("settings.json")],
            Agent::Gemini => vec![home.join(".gemini").join("settings.json")],
            Agent::Antigravity => vec![home.join(".gemini").join("config").join("hooks.json")],
            Agent::Codex => vec![home.join(".codex").join("hooks.json")],
        }
    }
}

/// Claude Code events with their hook timeout in seconds. PermissionRequest
/// waits for a human: the relay's 110 s plus a margin.
const CLAUDE_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

/// Gemini CLI: its event, the name the relay reports, timeout in ms.
/// AfterModel is left out: it fires on every chunk.
const GEMINI_EVENTS: &[(&str, &str, u64)] = &[
    ("SessionStart", "SessionStart", 10000),
    ("SessionEnd", "SessionEnd", 10000),
    ("BeforeTool", "PreToolUse", 5000),
    ("AfterTool", "PostToolUse", 5000),
    ("BeforeAgent", "UserPromptSubmit", 5000),
    ("AfterAgent", "Stop", 5000),
];

const ANTIGRAVITY_TOOL_EVENTS: &[&str] = &["PreToolUse", "PostToolUse"];
const ANTIGRAVITY_LIFECYCLE_EVENTS: &[&str] = &["PreInvocation", "PostInvocation", "Stop"];

/// Codex: Claude-style groups without a matcher, timeouts in seconds.
const CODEX_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PermissionRequest", 120),
    ("PostToolUse", 10),
    ("Stop", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
    ("Interrupt", 3),
    ("SessionEnd", 3),
];
const CODEX_WAITING: &str = "Waiting for your answer in Mochi (Coucou)";

/// `s` as one single-quoted POSIX shell word.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The relay command line for `args`.
fn command(relay: &str, args: &str) -> String {
    format!("{} {args}", sh_quote(relay)).trim_end().to_string()
}

fn unexpected(what: &str) -> String {
    format!("{what} has an unexpected type; Coucou has not touched it")
}

fn object_at(root: &Map<String, Value>, key: &str) -> Result<Map<String, Value>, String> {
    match root.get(key) {
        None => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => Err(unexpected(&format!("\"{key}\""))),
    }
}

fn list_at(hooks: &Map<String, Value>, event: &str) -> Result<Vec<Value>, String> {
    match hooks.get(event) {
        None => Ok(Vec::new()),
        Some(Value::Array(list)) => Ok(list.clone()),
        Some(_) => Err(unexpected(&format!("\"hooks\".\"{event}\""))),
    }
}

/// A command line written by Coucou for the agent tagged `tag`.
fn is_our_command(command: Option<&Value>, tag: &str) -> bool {
    command.and_then(Value::as_str).is_some_and(|c| {
        c.contains(MARKER)
            && if tag.is_empty() { !c.contains("--agent ") } else { c.contains(&format!("--agent {tag}")) }
    })
}

/// Claude-style groups without Coucou's entries for `tag`. A group left empty
/// goes; everything else stays exactly as it was.
fn without_ours_in_groups(groups: &[Value], tag: &str) -> Vec<Value> {
    groups
        .iter()
        .filter_map(|group| {
            if is_our_command(group.get("command"), tag) {
                return None;
            }
            let Some(inner) = group.get("hooks").and_then(Value::as_array) else {
                return Some(group.clone());
            };
            let kept: Vec<Value> = inner.iter().filter(|h| !is_our_command(h.get("command"), tag)).cloned().collect();
            if kept.len() == inner.len() {
                return Some(group.clone());
            }
            if kept.is_empty() {
                return None;
            }
            let mut group = group.clone();
            group["hooks"] = Value::Array(kept);
            Some(group)
        })
        .collect()
}

fn groups_install(root: &Value, tag: &str, events: Vec<(String, Value)>) -> Result<Value, String> {
    let mut root = root.as_object().cloned().unwrap_or_default();
    let mut hooks = object_at(&root, "hooks")?;
    for (event, group) in events {
        let mut list = without_ours_in_groups(&list_at(&hooks, &event)?, tag);
        list.push(group);
        hooks.insert(event, Value::Array(list));
    }
    root.insert("hooks".into(), Value::Object(hooks));
    Ok(Value::Object(root))
}

fn groups_uninstall(root: &Value, tag: &str) -> Result<Value, String> {
    let mut root = root.as_object().cloned().unwrap_or_default();
    if !root.contains_key("hooks") {
        return Ok(Value::Object(root));
    }
    let hooks = object_at(&root, "hooks")?;
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept = without_ours_in_groups(list, tag);
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Ok(Value::Object(root))
}

fn antigravity_is_ours(group: &Value) -> bool {
    let text = group.to_string();
    text.contains(MARKER) && text.contains("--agent antigravity")
}

/// The agent's config with Coucou's hooks added (or removed with `remove`).
pub fn transform(agent: Agent, root: &Value, relay: &str, remove: bool) -> Result<Value, String> {
    match (agent, remove) {
        (Agent::Antigravity, true) => {
            let mut root = root.as_object().cloned().unwrap_or_default();
            if root.get("coucou").is_some_and(antigravity_is_ours) {
                root.remove("coucou");
            }
            Ok(Value::Object(root))
        }
        (Agent::Antigravity, false) => {
            let mut root = root.as_object().cloned().unwrap_or_default();
            if root.get("coucou").is_some_and(|g| !antigravity_is_ours(g)) {
                return Err("a hook group named \"coucou\" that Coucou did not write is already there; Coucou has not touched it".into());
            }
            let handler = |event: &str| {
                json!({ "type": "command", "command": command(relay, &format!("--agent antigravity {event}")), "timeout": 10 })
            };
            let mut block = Map::new();
            for event in ANTIGRAVITY_TOOL_EVENTS {
                block.insert((*event).into(), json!([{ "matcher": "*", "hooks": [handler(event)] }]));
            }
            for event in ANTIGRAVITY_LIFECYCLE_EVENTS {
                block.insert((*event).into(), json!([handler(event)]));
            }
            root.insert("coucou".into(), Value::Object(block));
            Ok(Value::Object(root))
        }
        (_, true) => groups_uninstall(root, agent.tag()),
        (Agent::Claude, false) => {
            let events = CLAUDE_EVENTS
                .iter()
                .map(|(event, timeout)| {
                    let hook = json!({ "type": "command", "command": command(relay, event), "timeout": timeout });
                    (event.to_string(), json!({ "hooks": [hook] }))
                })
                .collect();
            groups_install(root, "", events)
        }
        (Agent::Gemini, false) => {
            let events = GEMINI_EVENTS
                .iter()
                .map(|(event, said, timeout)| {
                    let cmd = command(relay, &format!("--agent gemini {said}"));
                    let hook = json!({ "type": "command", "command": cmd, "timeout": timeout });
                    (event.to_string(), json!({ "matcher": "*", "hooks": [hook] }))
                })
                .collect();
            groups_install(root, "gemini", events)
        }
        (Agent::Codex, false) => {
            let cmd = command(relay, "--agent codex");
            let events = CODEX_EVENTS
                .iter()
                .map(|(event, timeout)| {
                    let mut hook = json!({ "type": "command", "command": cmd, "timeout": timeout });
                    if *event == "PermissionRequest" {
                        hook["statusMessage"] = json!(CODEX_WAITING);
                    }
                    (event.to_string(), json!({ "hooks": [hook] }))
                })
                .collect();
            groups_install(root, "codex", events)
        }
    }
}

/// `<file>.coucou-backup-YYYYMMDD-HHMMSS`, in UTC.
pub fn backup_path(file: &Path, unix_secs: u64) -> PathBuf {
    let secs = unix_secs as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    let stamp = format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", t / 3600, t % 3600 / 60, t % 60);
    let mut name = file.as_os_str().to_owned();
    name.push(format!(".coucou-backup-{stamp}"));
    PathBuf::from(name)
}

/// `(year, month, day)` of a day count since 1970-01-01 (Howard Hinnant's
/// civil_from_days).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// A line diff with two lines of context, `-`/`+` marked.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // LCS table, fine for config-sized files.
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(('+', b[j]));
            j += 1;
        } else {
            ops.push(('-', a[i]));
            i += 1;
        }
    }
    const CONTEXT: usize = 2;
    let near_change = |k: usize| {
        let lo = k.saturating_sub(CONTEXT);
        let hi = (k + CONTEXT).min(ops.len() - 1);
        (lo..=hi).any(|n| ops[n].0 != ' ')
    };
    let mut out = String::new();
    let mut skipped = false;
    for (k, (mark, line)) in ops.iter().enumerate() {
        if *mark == ' ' && !near_change(k) {
            if !skipped {
                out.push_str("  ...\n");
                skipped = true;
            }
            continue;
        }
        skipped = false;
        out.push_str(&format!("{mark} {line}\n"));
    }
    out
}

/// What one run would do to one file.
#[derive(Debug)]
pub struct Plan {
    pub path: PathBuf,
    /// The bytes on disk, `None` when the file does not exist.
    pub before: Option<String>,
    /// The bytes to write, `None` when the file should not exist.
    pub after: Option<String>,
}

impl Plan {
    pub fn changes(&self) -> bool {
        self.before != self.after
    }

    /// `--- path` / `+++ path` then the line diff, or "" when nothing changes.
    pub fn diff_text(&self) -> String {
        if !self.changes() {
            return String::new();
        }
        let path = self.path.display();
        let after = match &self.after {
            Some(text) => text.as_str(),
            None => "",
        };
        let removed = if self.after.is_none() { " (removed)" } else { "" };
        format!("--- {path}\n+++ {path} (Coucou){removed}\n{}", diff(self.before.as_deref().unwrap_or(""), after))
    }
}

/// The file's bytes, `None` when it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
}

/// The file as a JSON object, `{}` when absent or blank. Invalid JSON is an
/// error: we never overwrite what we cannot parse.
fn read_json(path: &Path, before: Option<&str>) -> Result<Value, String> {
    let current = match before.map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str::<Value>(text)
            .map_err(|err| format!("{} is not valid JSON ({err}); Coucou has not touched it", path.display()))?,
    };
    if !current.is_object() {
        return Err(unexpected(&path.display().to_string()));
    }
    Ok(current)
}

/// Reads the agent's files strictly and computes their new content.
pub fn plans(agent: Agent, home: &Path, relay: &str, remove: bool) -> Result<Vec<Plan>, String> {
    let path = agent.file(home);
    let before = read_optional(&path)?;
    let current = read_json(&path, before.as_deref())?;
    let next = transform(agent, &current, relay, remove)?;
    // Nothing to remove from a file that does not exist: leave it absent.
    if before.is_none() && remove {
        return Ok(vec![Plan { path, before: None, after: None }]);
    }
    // Same content, only formatted differently: no change.
    if before.as_deref().and_then(|b| serde_json::from_str::<Value>(b).ok()).as_ref() == Some(&next) {
        return Ok(vec![Plan { path, after: before.clone(), before }]);
    }
    let mut after = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
    after.push('\n');
    Ok(vec![Plan { path, before, after: Some(after) }])
}

/// The first free `<file>.coucou-backup-<stamp>[.n]`: two runs in the same
/// second keep both backups.
pub fn free_backup_path(file: &Path, unix_secs: u64) -> PathBuf {
    let base = backup_path(file, unix_secs);
    let mut path = base.clone();
    let mut n = 1;
    while path.exists() {
        path = PathBuf::from(format!("{}.{n}", base.display()));
        n += 1;
    }
    path
}

/// Takes the dated backup (when there is a file), then writes the new content
/// atomically keeping the file's permissions, or removes the file. Returns the
/// backup path.
pub fn apply(plan: &Plan, unix_secs: u64) -> io::Result<Option<PathBuf>> {
    let dir = plan.path.parent().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut backup = None;
    let mut perms = None;
    if plan.before.is_some() {
        let path = free_backup_path(&plan.path, unix_secs);
        std::fs::copy(&plan.path, &path)?;
        perms = Some(std::fs::metadata(&plan.path)?.permissions());
        backup = Some(path);
    }
    let Some(after) = &plan.after else {
        if plan.before.is_some() {
            std::fs::remove_file(&plan.path)?;
        }
        return Ok(backup);
    };
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.coucou-tmp", plan.path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, after)?;
    if let Some(perms) = perms {
        std::fs::set_permissions(&tmp, perms)?;
    }
    std::fs::rename(&tmp, &plan.path)?;
    Ok(backup)
}

/// Where coucou-hook is: beside this binary, else the packaged path, else PATH.
pub fn default_relay() -> String {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        let candidate = dir.join("coucou-hook");
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    if Path::new("/usr/bin/coucou-hook").is_file() {
        return "/usr/bin/coucou-hook".into();
    }
    "coucou-hook".into()
}

// ── Status: is the agent there, and does it reach Coucou? ────────────────────

/// An executable file named `name` in one of `path_env`'s folders.
fn find_on_path(name: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path_env?).map(|dir| dir.join(name)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Whether the relay at `relay` (a path, or a bare name looked up on PATH)
/// is there to run.
pub fn relay_exists(relay: &str, path_env: Option<&OsStr>) -> bool {
    if relay.contains('/') {
        Path::new(relay).is_file()
    } else {
        find_on_path(relay, path_env).is_some()
    }
}

/// The program of a POSIX command line: its first shell word, unquoted
/// (single quotes, double quotes and backslashes).
pub fn relay_of_command(command: &str) -> Option<String> {
    let mut word = String::new();
    let mut chars = command.trim_start().chars();
    let mut started = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                started = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        q => word.push(q),
                    }
                }
            }
            '\\' => {
                started = true;
                word.extend(chars.next());
            }
            c if c.is_whitespace() => break,
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    (started && !word.is_empty()).then_some(word)
}

/// Every command line Coucou wrote for `tag` in a JSON config.
fn our_commands(root: &Value, tag: &str) -> Vec<String> {
    fn walk(v: &Value, tag: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if matches!(k.as_str(), "command" | "bash" | "powershell") && is_our_command(Some(v), tag) {
                        out.extend(v.as_str().map(str::to_string));
                    }
                    walk(v, tag, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, tag, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(root, tag, &mut out);
    out
}

/// What Settings → Agents shows for one agent.
#[derive(Debug)]
pub struct Status {
    /// The agent itself is on this machine (a binary on PATH or its folder).
    pub detected: bool,
    /// Coucou's entries are in its config.
    pub installed: bool,
    /// Installed, and every relay those entries run exists.
    pub connected: bool,
    pub config_path: PathBuf,
    /// The relays Coucou's entries run, in order, without repeats.
    pub relays: Vec<String>,
}

/// Never fails: a file we cannot read just reads as "not installed".
pub fn status(agent: Agent, home: &Path, path_env: Option<&OsStr>) -> Status {
    let (bins, dirs) = agent.traces();
    let detected =
        bins.iter().any(|b| find_on_path(b, path_env).is_some()) || dirs.iter().any(|d| home.join(d).is_dir());
    let config_path = agent.file(home);
    let root = read_optional(&config_path)
        .ok()
        .flatten()
        .and_then(|text| read_json(&config_path, Some(&text)).ok())
        .unwrap_or_else(|| json!({}));
    let mut relays: Vec<String> = Vec::new();
    for r in our_commands(&root, agent.tag()).iter().filter_map(|c| relay_of_command(c)) {
        if !relays.contains(&r) {
            relays.push(r);
        }
    }
    let installed = !relays.is_empty();
    let connected = installed && relays.iter().all(|r| relay_exists(r, path_env));
    Status { detected, installed, connected, config_path, relays }
}

/// `--status --json`: the relay Connect would use, and every agent's status.
pub fn status_json(agents: &[Agent], home: &Path, path_env: Option<&OsStr>, relay: &str) -> Value {
    let list: Vec<Value> = agents
        .iter()
        .map(|&a| {
            let s = status(a, home, path_env);
            json!({
                "id": a.id(),
                "name": a.name(),
                "detected": s.detected,
                "installed": s.installed,
                "connected": s.connected,
                "approvals": a.approvals(),
                "config_path": s.config_path,
                "files": a.files(home),
                "relays": s.relays,
            })
        })
        .collect();
    json!({ "relay": relay, "relay_ok": relay_exists(relay, path_env), "agents": list })
}

/// `--dry-run --json`: what Connect (or `--remove`) would write, per agent,
/// and the backup it would take. Nothing is written.
pub fn dry_run_json(agents: &[Agent], home: &Path, relay: &str, remove: bool, unix_secs: u64) -> Value {
    let list: Vec<Value> = agents
        .iter()
        .map(|&a| match plans(a, home, relay, remove) {
            Ok(plans) => {
                let files: Vec<Value> = plans
                    .iter()
                    .map(|p| {
                        let backup = (p.changes() && p.before.is_some()).then(|| free_backup_path(&p.path, unix_secs));
                        json!({ "path": p.path, "changed": p.changes(), "backup_path": backup })
                    })
                    .collect();
                let backup = files.iter().find_map(|f| f["backup_path"].as_str().map(str::to_string));
                json!({
                    "id": a.id(),
                    "name": a.name(),
                    "changed": plans.iter().any(Plan::changes),
                    "diff": plans.iter().map(Plan::diff_text).collect::<String>(),
                    "backup_path": backup,
                    "files": files,
                })
            }
            Err(err) => json!({ "id": a.id(), "name": a.name(), "changed": false, "diff": "", "backup_path": null, "error": err }),
        })
        .collect();
    json!({ "relay": relay, "remove": remove, "agents": list })
}

// ── The command line ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Diff, confirm, back up, write.
    Apply,
    /// Detected / installed / connected, per agent. Writes nothing.
    Status,
    /// The diff and backup path that Apply would use. Writes nothing.
    DryRun,
}

/// The `install-hooks` command line.
pub struct Options {
    pub agents: Vec<Agent>,
    pub yes: bool,
    pub remove: bool,
    pub home: PathBuf,
    pub relay: String,
    pub mode: Mode,
    pub json: bool,
}

pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut agents = Vec::new();
    let mut yes = false;
    let mut remove = false;
    let mut json = false;
    let mut mode = Mode::Apply;
    let mut home = std::env::var_os("HOME").map(PathBuf::from);
    let mut relay = None;
    let mut it = args.iter();
    let mut set_mode = |m: Mode| {
        if mode != Mode::Apply {
            return Err("--status and --dry-run cannot be combined".to_string());
        }
        mode = m;
        Ok(())
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--yes" | "-y" => yes = true,
            "--remove" => remove = true,
            "--json" => json = true,
            "--status" => set_mode(Mode::Status)?,
            "--dry-run" => set_mode(Mode::DryRun)?,
            "--agent" => {
                let id = it.next().ok_or("--agent needs a value")?;
                let known = Agent::ALL.iter().map(|a| a.id()).collect::<Vec<_>>().join(", ");
                let agent = Agent::from_id(id).ok_or_else(|| format!("unknown agent: {id} ({known})"))?;
                if !agents.contains(&agent) {
                    agents.push(agent);
                }
            }
            "--home" => home = Some(PathBuf::from(it.next().ok_or("--home needs a value")?)),
            "--hook-path" => relay = Some(it.next().ok_or("--hook-path needs a value")?.clone()),
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if json && mode == Mode::Apply {
        return Err("--json goes with --status or --dry-run".into());
    }
    if agents.is_empty() {
        // Writing defaults to Claude Code alone; looking defaults to everyone.
        agents = if mode == Mode::Apply { vec![Agent::Claude] } else { Agent::ALL.to_vec() };
    }
    Ok(Options {
        agents,
        yes,
        remove,
        home: home.ok_or("HOME is not set")?,
        relay: relay.unwrap_or_else(default_relay),
        mode,
        json,
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Runs the installer: diff, confirmation, backup, write — per agent. In the
/// status and dry-run modes, only prints.
pub fn run(opts: &Options, input: &mut impl BufRead, out: &mut impl Write) -> Result<(), String> {
    let now = now_secs();
    let io_err = |e: io::Error| e.to_string();
    let path_env = std::env::var_os("PATH");
    match (opts.mode, opts.json) {
        (Mode::Status, true) => {
            let v = status_json(&opts.agents, &opts.home, path_env.as_deref(), &opts.relay);
            return writeln!(out, "{v}").map_err(io_err);
        }
        (Mode::DryRun, true) => {
            let v = dry_run_json(&opts.agents, &opts.home, &opts.relay, opts.remove, now);
            return writeln!(out, "{v}").map_err(io_err);
        }
        (Mode::Status, false) => {
            for &agent in &opts.agents {
                let s = status(agent, &opts.home, path_env.as_deref());
                let yes_no = |b: bool| if b { "yes" } else { "no" };
                writeln!(
                    out,
                    "{:<20} detected: {:<3}  connected: {:<3}  {}",
                    agent.name(),
                    yes_no(s.detected),
                    yes_no(s.connected),
                    s.config_path.display()
                )
                .map_err(io_err)?;
            }
            return Ok(());
        }
        _ => {}
    }
    for agent in &opts.agents {
        let plans = plans(*agent, &opts.home, &opts.relay, opts.remove)?;
        if !plans.iter().any(Plan::changes) {
            writeln!(out, "{}: {} is already up to date", agent.id(), plans[0].path.display()).map_err(io_err)?;
            continue;
        }
        for plan in &plans {
            write!(out, "{}", plan.diff_text()).map_err(io_err)?;
        }
        if opts.mode == Mode::DryRun {
            writeln!(out, "{}: dry run, nothing written", agent.id()).map_err(io_err)?;
            continue;
        }
        if !opts.yes {
            write!(out, "Write {}? A dated backup is kept. [y/N] ", agent.name()).map_err(io_err)?;
            out.flush().map_err(io_err)?;
            let mut answer = String::new();
            input.read_line(&mut answer).map_err(io_err)?;
            if !matches!(answer.trim(), "y" | "Y" | "yes") {
                writeln!(out, "{}: left unchanged", agent.id()).map_err(io_err)?;
                continue;
            }
        }
        for plan in plans.iter().filter(|p| p.changes()) {
            let backup = apply(plan, now).map_err(|e| format!("cannot write {}: {e}", plan.path.display()))?;
            match backup {
                Some(b) => writeln!(out, "{}: {} written (backup: {})", agent.id(), plan.path.display(), b.display()),
                None => writeln!(out, "{}: {} written", agent.id(), plan.path.display()),
            }
            .map_err(io_err)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY: &str = "/usr/bin/coucou-hook";

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coucou-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn commands(v: &Value) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    if let Some(Value::String(c)) = m.get("command") {
                        out.push(c.clone());
                    }
                    m.values().for_each(|v| walk(v, out));
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
        walk(v, &mut out);
        out
    }

    #[test]
    fn claude_merge_keeps_foreign_hooks_and_is_idempotent() {
        let existing = json!({
            "theme": "dark",
            "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-linter" }] }] }
        });
        let once = transform(Agent::Claude, &existing, RELAY, false).unwrap();
        let twice = transform(Agent::Claude, &once, RELAY, false).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once["theme"], "dark");
        let pre = once["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0]["hooks"][0]["command"], "my-linter");
        assert_eq!(pre[1]["hooks"][0]["command"], "'/usr/bin/coucou-hook' PreToolUse");
        assert_eq!(once["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 120);
        assert_eq!(once["hooks"].as_object().unwrap().len(), CLAUDE_EVENTS.len());

        let removed = transform(Agent::Claude, &once, RELAY, true).unwrap();
        assert_eq!(removed, existing);
    }

    #[test]
    fn claude_entries_never_touch_other_agents_relay_entries() {
        let existing = json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "'/x/coucou-hook' --agent codex Stop" }] }] } });
        let merged = transform(Agent::Claude, &existing, RELAY, false).unwrap();
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn unexpected_shapes_are_refused() {
        assert!(transform(Agent::Claude, &json!({ "hooks": [] }), RELAY, false).is_err());
        assert!(transform(Agent::Gemini, &json!({ "hooks": { "BeforeTool": {} } }), RELAY, false).is_err());
        assert!(transform(Agent::Antigravity, &json!({ "coucou": { "PreToolUse": "mine" } }), RELAY, false).is_err());
    }

    #[test]
    fn gemini_uses_matchers_milliseconds_and_reported_names() {
        let existing = json!({ "hooks": { "BeforeTool": [{ "matcher": "other", "hooks": [{ "command": "custom" }] }] } });
        let v = transform(Agent::Gemini, &existing, RELAY, false).unwrap();
        let before = v["hooks"]["BeforeTool"].as_array().unwrap();
        assert_eq!(before.len(), 2);
        assert_eq!(before[1]["matcher"], "*");
        assert_eq!(before[1]["hooks"][0]["command"], "'/usr/bin/coucou-hook' --agent gemini PreToolUse");
        assert_eq!(before[1]["hooks"][0]["timeout"], 5000);
        assert_eq!(transform(Agent::Gemini, &v, RELAY, false).unwrap(), v);
        assert_eq!(transform(Agent::Gemini, &v, RELAY, true).unwrap(), existing);
    }

    #[test]
    fn antigravity_owns_one_named_group() {
        let existing = json!({ "mine": { "Stop": [] } });
        let v = transform(Agent::Antigravity, &existing, RELAY, false).unwrap();
        assert_eq!(v["mine"], existing["mine"]);
        assert_eq!(v["coucou"]["PreToolUse"][0]["matcher"], "*");
        assert_eq!(v["coucou"]["Stop"][0]["command"], "'/usr/bin/coucou-hook' --agent antigravity Stop");
        assert_eq!(transform(Agent::Antigravity, &v, RELAY, false).unwrap(), v);
        assert_eq!(transform(Agent::Antigravity, &v, RELAY, true).unwrap(), existing);
    }

    #[test]
    fn codex_waits_on_permission_requests() {
        let v = transform(Agent::Codex, &json!({}), RELAY, false).unwrap();
        let hook = &v["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(hook["command"], "'/usr/bin/coucou-hook' --agent codex");
        assert_eq!(hook["timeout"], 120);
        assert_eq!(hook["statusMessage"], CODEX_WAITING);
        assert!(v["hooks"]["PermissionRequest"][0].get("matcher").is_none());
        assert_eq!(commands(&v).len(), CODEX_EVENTS.len());
    }

    #[test]
    fn relay_paths_are_one_shell_word() {
        let v = transform(Agent::Claude, &json!({}), "/home/o'neil/my bin/coucou-hook", false).unwrap();
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], r"'/home/o'\''neil/my bin/coucou-hook' Stop");
    }

    #[test]
    fn backup_names_are_dated_in_utc() {
        // 2026-10-10 07:46:05 UTC
        let p = backup_path(Path::new("/h/.claude/settings.json"), 1_791_618_365);
        assert_eq!(p, PathBuf::from("/h/.claude/settings.json.coucou-backup-20261010-074605"));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn diff_marks_added_and_removed_lines() {
        let d = diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert!(d.contains("- b\n"));
        assert!(d.contains("+ B\n"));
        assert!(d.contains("+ d\n"));
        assert!(d.contains("  a\n"));
    }

    #[test]
    fn apply_backs_up_then_writes_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let home = temp_home("apply");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"model\":\"opus\"}\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();

        let p = plans(Agent::Claude, &home, RELAY, false).unwrap().remove(0);
        assert!(p.changes());
        let backup = apply(&p, 1_791_618_365).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"model\":\"opus\"}\n");
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(written["model"], "opus");
        assert!(written["hooks"]["Stop"].is_array());
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);

        // A second run has nothing to do.
        assert!(!plans(Agent::Claude, &home, RELAY, false).unwrap().remove(0).changes());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_missing_file_is_created_without_backup() {
        let home = temp_home("missing");
        let p = plans(Agent::Codex, &home, RELAY, false).unwrap().remove(0);
        assert!(p.before.is_none());
        assert_eq!(apply(&p, 0).unwrap(), None);
        assert!(Agent::Codex.file(&home).is_file());
        // Removing from a file that never existed changes nothing.
        let home2 = temp_home("missing2");
        assert!(!plans(Agent::Gemini, &home2, RELAY, true).unwrap().remove(0).changes());
        std::fs::remove_dir_all(&home).unwrap();
        std::fs::remove_dir_all(&home2).unwrap();
    }

    #[test]
    fn invalid_json_is_never_overwritten() {
        let home = temp_home("invalid");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ not json").unwrap();
        assert!(plans(Agent::Claude, &home, RELAY, false).is_err());
        std::fs::remove_dir_all(&home).unwrap();
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn fake_relay(dir: &Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let relay = dir.join("bin").join("coucou-hook");
        std::fs::create_dir_all(relay.parent().unwrap()).unwrap();
        std::fs::write(&relay, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&relay, std::fs::Permissions::from_mode(0o755)).unwrap();
        relay.to_string_lossy().into_owned()
    }

    #[test]
    fn relay_paths_are_read_back_from_command_lines() {
        assert_eq!(relay_of_command("'/usr/bin/coucou-hook' Stop").as_deref(), Some("/usr/bin/coucou-hook"));
        assert_eq!(relay_of_command(r"'/home/o'\''neil/my bin/coucou-hook' --agent codex").as_deref(), Some("/home/o'neil/my bin/coucou-hook"));
        assert_eq!(relay_of_command("coucou-hook --agent gemini Stop").as_deref(), Some("coucou-hook"));
        assert_eq!(relay_of_command("").as_deref(), None);
    }

    #[test]
    fn status_reports_detected_installed_and_connected() {
        let home = temp_home("status");
        let relay = fake_relay(&home);
        // Codex: only its config folder. Gemini: only its binary on PATH.
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let bin = home.join("path");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("gemini"), "").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("gemini"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path_env = std::ffi::OsString::from(bin.as_os_str());
        for p in plans(Agent::Claude, &home, &relay, false).unwrap() {
            apply(&p, 0).unwrap();
        }

        let s = status(Agent::Claude, &home, Some(&path_env));
        assert!(s.detected && s.installed && s.connected, "claude");
        assert_eq!(s.relays, vec![relay.clone()]);
        assert_eq!(s.config_path, home.join(".claude").join("settings.json"));
        let s = status(Agent::Codex, &home, Some(&path_env));
        assert!(s.detected && !s.installed && !s.connected, "codex");
        assert!(status(Agent::Gemini, &home, Some(&path_env)).detected, "gemini on PATH");
        assert!(!status(Agent::Antigravity, &home, Some(&path_env)).detected, "antigravity");

        // The relay the entries point at is gone: still installed, not connected.
        std::fs::remove_file(&relay).unwrap();
        let s = status(Agent::Claude, &home, Some(&path_env));
        assert!(s.installed && !s.connected);

        let v = status_json(Agent::ALL, &home, Some(&path_env), &relay);
        assert_eq!(v["relay"], relay.as_str());
        assert_eq!(v["relay_ok"], false);
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), Agent::ALL.len());
        assert_eq!(agents[0]["id"], "claude");
        assert_eq!(agents[0]["installed"], true);
        assert_eq!(agents[0]["connected"], false);
        assert!(agents[0]["config_path"].as_str().unwrap().ends_with(".claude/settings.json"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn dry_run_shows_the_diff_and_backup_and_writes_nothing() {
        let home = temp_home("dryrun");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"model\":\"opus\"}\n").unwrap();

        let v = dry_run_json(&[Agent::Claude, Agent::Codex], &home, RELAY, false, 1_791_618_365);
        let claude = &v["agents"][0];
        assert_eq!(claude["id"], "claude");
        assert_eq!(claude["changed"], true);
        let diff = claude["diff"].as_str().unwrap();
        assert!(diff.contains("+++ ") && diff.contains("coucou-hook") && diff.contains("  \"model\": \"opus\""), "{diff}");
        assert!(claude["backup_path"].as_str().unwrap().ends_with("settings.json.coucou-backup-20261010-074605"));
        // Codex has no file yet: created, nothing to back up.
        assert_eq!(v["agents"][1]["changed"], true);
        assert!(v["agents"][1]["backup_path"].is_null());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"model\":\"opus\"}\n");
        assert!(!Agent::Codex.file(&home).exists());

        // An unparsable config is an error for that agent only.
        std::fs::write(&file, "{ nope").unwrap();
        let v = dry_run_json(&[Agent::Claude], &home, RELAY, true, 0);
        assert_eq!(v["agents"][0]["changed"], false);
        assert!(v["agents"][0]["error"].as_str().unwrap().contains("not valid JSON"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn status_and_dry_run_flags_parse() {
        let o = parse_args(&args(&["--status", "--json", "--home", "/h"])).unwrap();
        assert_eq!(o.mode, Mode::Status);
        assert!(o.json);
        assert_eq!(o.agents, Agent::ALL.to_vec());
        let o = parse_args(&args(&["--dry-run", "--json", "--agent", "codex", "--remove", "--home", "/h"])).unwrap();
        assert_eq!(o.mode, Mode::DryRun);
        assert_eq!(o.agents, vec![Agent::Codex]);
        assert!(o.remove);
        // Without a mode, the default stays Claude Code only.
        assert_eq!(parse_args(&args(&["--home", "/h"])).unwrap().agents, vec![Agent::Claude]);
        assert!(parse_args(&args(&["--json", "--home", "/h"])).is_err());
        assert!(parse_args(&args(&["--status", "--dry-run", "--home", "/h"])).is_err());
    }

    #[test]
    fn nothing_is_written_without_a_yes() {
        let home = temp_home("confirm");
        let opts = Options {
            agents: vec![Agent::Claude],
            yes: false,
            remove: false,
            home: home.clone(),
            relay: RELAY.into(),
            mode: Mode::Apply,
            json: false,
        };
        let mut out = Vec::new();
        run(&opts, &mut "n\n".as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("+++"), "{text}");
        assert!(text.contains("left unchanged"));
        assert!(!Agent::Claude.file(&home).exists());

        let mut out = Vec::new();
        run(&opts, &mut "y\n".as_bytes(), &mut out).unwrap();
        assert!(Agent::Claude.file(&home).is_file());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
