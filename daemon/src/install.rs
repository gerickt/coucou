//! `coucou-daemon install-hooks`: routes each agent's hooks to coucou-hook.
//!
//! The rule is strict: read the agent's config, merge without touching anybody
//! else's entries, show the diff, take a dated backup, and write only after an
//! explicit "yes" (or `--yes`). `--remove` takes Coucou's entries out and
//! nothing else. A config whose `hooks` has an unexpected shape is refused,
//! never replaced. `--dry-run --json` gives each file's `sha256` (or
//! "absent"); passing them back as `--expect-sha256 <path>=<hash>` makes the
//! write refuse any file that changed since that preview.

use std::io::{self, BufRead, Write};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// Marker that identifies a Coucou entry: the relay's file name.
const MARKER: &str = "coucou-hook";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Agent {
    Claude,
    Codex,
    Copilot,
    Muse,
    Gemini,
    Antigravity,
    Cursor,
    OpenCode,
    Amp,
    Hermes,
}

impl Agent {
    pub const ALL: &'static [Agent] = &[
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::Muse,
        Agent::Gemini,
        Agent::Antigravity,
        Agent::Cursor,
        Agent::OpenCode,
        Agent::Amp,
        Agent::Hermes,
    ];

    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.iter().copied().find(|a| a.id() == id)
    }

    /// The `--agent` name the relay gets (and the pill, `agent_<id>`).
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Copilot => "copilot",
            Agent::Muse => "muse",
            Agent::Gemini => "gemini",
            Agent::Antigravity => "antigravity",
            Agent::Cursor => "cursor",
            Agent::OpenCode => "opencode",
            Agent::Amp => "amp",
            Agent::Hermes => "hermes",
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
            Agent::Codex => "Codex",
            Agent::Copilot => "GitHub Copilot CLI",
            Agent::Muse => "Muse Code",
            Agent::Gemini => "Gemini CLI",
            Agent::Antigravity => "Antigravity",
            Agent::Cursor => "Cursor Agent",
            Agent::OpenCode => "OpenCode",
            Agent::Amp => "Amp",
            Agent::Hermes => "Hermes Agent",
        }
    }

    /// What to do once the file is written.
    pub fn note(self) -> &'static str {
        match self {
            Agent::Claude => "Start a new Claude Code session to pick the hooks up.",
            Agent::Codex => "Codex runs new hooks only once you trust them: start Codex and review them once with /hooks.",
            Agent::Copilot => "Start a new Copilot CLI session to pick the hooks up.",
            Agent::Muse => "Start a new Muse Code session to pick the hooks up.",
            Agent::Gemini => "Start a new Gemini CLI session to pick the hooks up.",
            Agent::Antigravity => "Start a new Antigravity conversation to pick the hooks up.",
            Agent::Cursor => "Restart Cursor to pick the hooks up.",
            Agent::OpenCode => "Restart OpenCode to load the plugin.",
            Agent::Amp => "Restart Amp to load the plugin.",
            Agent::Hermes => "Turn it on once with `hermes plugins enable coucou`, then start a new Hermes session.",
        }
    }

    /// Whether Mochi can allow or deny this agent's permission requests. Must
    /// match `takes_decisions` in the relay (hook/src/reply.rs).
    pub fn approvals(self) -> bool {
        matches!(self, Agent::Claude | Agent::Codex | Agent::Copilot | Agent::Muse)
    }

    /// An agent that loads a plugin file rather than running hook commands.
    fn is_plugin(self) -> bool {
        matches!(self, Agent::OpenCode | Agent::Amp | Agent::Hermes)
    }

    /// The agent's own binaries and folders: any of them means it is installed.
    fn traces(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Agent::Claude => (&["claude"], &[".claude"]),
            Agent::Codex => (&["codex"], &[".codex"]),
            Agent::Copilot => (&["copilot"], &[".copilot"]),
            Agent::Muse => (&["muse"], &[".config/muse"]),
            Agent::Gemini => (&["gemini"], &[".gemini"]),
            Agent::Antigravity => (&["antigravity"], &[".antigravity", ".gemini/antigravity"]),
            Agent::Cursor => (&["cursor-agent", "cursor"], &[".cursor"]),
            Agent::OpenCode => (&["opencode"], &[".config/opencode", ".opencode"]),
            Agent::Amp => (&["amp"], &[".config/amp"]),
            Agent::Hermes => (&["hermes"], &[".hermes"]),
        }
    }

    /// The config file Coucou edits (the first of `files`).
    pub fn file(self, home: &Path) -> PathBuf {
        self.files(home).remove(0)
    }

    /// Every file Coucou writes for this agent (paths as in the original app).
    pub fn files(self, home: &Path) -> Vec<PathBuf> {
        let config = home.join(".config");
        match self {
            Agent::Claude => vec![home.join(".claude").join("settings.json")],
            Agent::Codex => vec![home.join(".codex").join("hooks.json")],
            Agent::Copilot => vec![home.join(".copilot").join("hooks").join("coucou.json")],
            Agent::Muse => vec![config.join("muse").join("settings.json")],
            Agent::Gemini => vec![home.join(".gemini").join("settings.json")],
            Agent::Antigravity => vec![home.join(".gemini").join("config").join("hooks.json")],
            Agent::Cursor => vec![home.join(".cursor").join("hooks.json")],
            Agent::OpenCode => vec![config.join("opencode").join("plugins").join("coucou.js")],
            Agent::Amp => vec![config.join("amp").join("plugins").join("coucou.ts")],
            Agent::Hermes => {
                let dir = home.join(".hermes").join("plugins").join("coucou");
                vec![dir.join("__init__.py"), dir.join("plugin.yaml")]
            }
        }
    }

    /// For a plugin agent, the contents of each of its files (same order as
    /// `files`). A JSON string is also a valid JavaScript, TypeScript and
    /// Python one.
    fn plugin(self, relay: &str) -> Vec<String> {
        let hook = serde_json::to_string(relay).unwrap_or_default();
        match self {
            Agent::OpenCode => vec![OPENCODE_PLUGIN.replace("{HOOK}", &hook)],
            Agent::Amp => vec![AMP_PLUGIN.replace("{HOOK}", &hook)],
            Agent::Hermes => vec![HERMES_PLUGIN.replace("{HOOK}", &hook), HERMES_PLUGIN_YAML.to_string()],
            _ => Vec::new(),
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

/// Cursor Agent: flat `{"command"}` entries under camelCase events; the relay
/// reads the event from the payload and never answers for Cursor.
const CURSOR_EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "beforeSubmitPrompt",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "stop",
];

/// GitHub Copilot CLI: a file of Coucou's own, camelCase events, timeouts in
/// seconds. The event goes on the command line (Copilot leaves it out of the
/// payload); the relay always answers permissionRequest, "ask" by default.
const COPILOT_EVENTS: &[(&str, u64)] = &[
    ("sessionStart", 10),
    ("userPromptSubmitted", 10),
    ("preToolUse", 10),
    ("permissionRequest", 120),
    ("postToolUse", 10),
    ("agentStop", 10),
    ("sessionEnd", 3),
    ("notification", 10),
];

/// Muse Code: Claude-style groups with a `*` matcher, timeouts in seconds here
/// and in milliseconds in the file.
const MUSE_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("UserPromptSubmit", 5),
    ("PreToolUse", 5),
    ("PermissionRequest", 120),
    ("PostToolUse", 5),
    ("Stop", 5),
    ("SessionEnd", 3),
];

/// Every plugin file Coucou writes says so; only such a file is replaced or
/// removed.
const GENERATED: &str = "generated by Coucou";

// The original app's plugins (windows/src-tauri/src/agents.rs), byte for byte;
// `{HOOK}` becomes the relay path as a JSON string. Fire-and-forget, display
// only: none of them ever answers a permission.
const OPENCODE_PLUGIN: &str = include_str!("templates/opencode.js");
const AMP_PLUGIN: &str = include_str!("templates/amp.ts");
const HERMES_PLUGIN: &str = include_str!("templates/hermes.py");
const HERMES_PLUGIN_YAML: &str = include_str!("templates/hermes.yaml");

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

fn copilot_entry_is_ours(entry: &Value) -> bool {
    ["bash", "powershell", "command"].iter().any(|k| is_our_command(entry.get(*k), "copilot"))
}

fn copilot_install(root: &Value, relay: &str) -> Result<Value, String> {
    let mut root = root.as_object().cloned().unwrap_or_default();
    let mut hooks = object_at(&root, "hooks")?;
    for (event, timeout) in COPILOT_EVENTS {
        let mut list = list_at(&hooks, event)?;
        list.retain(|e| !copilot_entry_is_ours(e));
        let bash = command(relay, &format!("--agent copilot {event}"));
        list.push(json!({ "type": "command", "bash": bash, "timeoutSec": timeout }));
        hooks.insert((*event).into(), Value::Array(list));
    }
    root.insert("hooks".into(), Value::Object(hooks));
    root.insert("version".into(), json!(1));
    Ok(Value::Object(root))
}

/// Coucou's entries out; `plans` removes a file left with nobody else's.
fn copilot_uninstall(root: &Value) -> Result<Value, String> {
    let mut root = root.as_object().cloned().unwrap_or_default();
    if root.contains_key("hooks") {
        let mut hooks = object_at(&root, "hooks")?;
        for list in hooks.values_mut() {
            if let Value::Array(entries) = list {
                entries.retain(|e| !copilot_entry_is_ours(e));
            }
        }
        hooks.retain(|_, v| !v.as_array().is_some_and(Vec::is_empty));
        if hooks.is_empty() {
            root.remove("hooks");
        } else {
            root.insert("hooks".into(), Value::Object(hooks));
        }
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
        (Agent::OpenCode | Agent::Amp | Agent::Hermes, _) => Err(format!("{} takes a plugin, not hook entries", agent.name())),
        (Agent::Copilot, true) => copilot_uninstall(root),
        (Agent::Copilot, false) => copilot_install(root, relay),
        (_, true) => groups_uninstall(root, agent.tag()),
        (Agent::Cursor, false) => {
            let cmd = command(relay, "--agent cursor");
            let events = CURSOR_EVENTS.iter().map(|e| (e.to_string(), json!({ "command": cmd }))).collect();
            let mut v = groups_install(root, "cursor", events)?;
            if let Value::Object(m) = &mut v {
                m.entry("version").or_insert(json!(1));
            }
            Ok(v)
        }
        (Agent::Muse, false) => {
            let fresh = root.as_object().is_none_or(Map::is_empty);
            let events = MUSE_EVENTS
                .iter()
                .map(|(event, seconds)| {
                    let cmd = command(relay, &format!("--agent muse {event}"));
                    let hook = json!({ "type": "command", "command": cmd, "timeout": seconds * 1000 });
                    (event.to_string(), json!({ "matcher": "*", "hooks": [hook] }))
                })
                .collect();
            let mut v = groups_install(root, "muse", events)?;
            if let (true, Value::Object(m)) = (fresh, &mut v) {
                m.insert("schema_version".into(), json!(1));
            }
            Ok(v)
        }
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

/// The fingerprint `--dry-run --json` gives a file that does not exist.
pub const ABSENT: &str = "absent";

/// SHA-256 of `data`, lowercase hex (FIPS 180-4). Small enough to keep the
/// daemon free of a crypto dependency; it only fingerprints config files.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
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

    /// SHA-256 of the bytes the plan was computed from, or [`ABSENT`].
    pub fn fingerprint(&self) -> String {
        match &self.before {
            Some(text) => sha256_hex(text.as_bytes()),
            None => ABSENT.to_string(),
        }
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
    if agent.is_plugin() {
        // Whole files, never merged: only a file Coucou wrote is replaced or removed.
        return agent
            .files(home)
            .into_iter()
            .zip(agent.plugin(relay))
            .map(|(path, content)| {
                let before = read_optional(&path)?;
                if before.as_deref().is_some_and(|text| !text.contains(GENERATED)) {
                    return Err(format!("{} wasn't written by Coucou; Coucou has not touched it", path.display()));
                }
                Ok(Plan { path, before, after: (!remove).then_some(content) })
            })
            .collect();
    }
    let path = agent.file(home);
    let before = read_optional(&path)?;
    let current = read_json(&path, before.as_deref())?;
    let next = transform(agent, &current, relay, remove)?;
    // Nothing to remove from a file that does not exist: leave it absent.
    if before.is_none() && remove {
        return Ok(vec![Plan { path, before: None, after: None }]);
    }
    // Copilot's file is Coucou's own: with nobody else's entries left, it goes.
    if agent == Agent::Copilot && remove && next.as_object().is_some_and(|m| m.keys().all(|k| k == "version")) {
        return Ok(vec![Plan { path, before, after: None }]);
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

/// The relay a generated plugin starts: its `HOOK = "<json string>"` line.
fn plugin_relay(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.strip_prefix("const HOOK = ").or_else(|| line.strip_prefix("HOOK = "))?;
        serde_json::from_str::<String>(rest.trim().trim_end_matches(';')).ok()
    })
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
    if agent.is_plugin() {
        let relays: Vec<String> = read_optional(&config_path)
            .ok()
            .flatten()
            .filter(|text| text.contains(GENERATED))
            .and_then(|text| plugin_relay(&text))
            .into_iter()
            .collect();
        let installed = !relays.is_empty();
        let connected = installed && relays.iter().all(|r| relay_exists(r, path_env));
        return Status { detected, installed, connected, config_path, relays };
    }
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
                "plugin": a.is_plugin(),
                "note": a.note(),
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
                        json!({ "path": p.path, "changed": p.changes(), "backup_path": backup, "sha256": p.fingerprint() })
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
    /// `--expect-sha256 <path>=<hash>`: the fingerprints a preview showed.
    /// When given, every file to write must be listed and still match.
    pub expect: Vec<(PathBuf, String)>,
}

pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut agents = Vec::new();
    let mut yes = false;
    let mut remove = false;
    let mut json = false;
    let mut mode = Mode::Apply;
    let mut home = std::env::var_os("HOME").map(PathBuf::from);
    let mut relay = None;
    let mut expect = Vec::new();
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
            "--expect-sha256" => {
                let value = it.next().filter(|v| !v.starts_with("--")).ok_or("--expect-sha256 needs <path>=<sha256>")?;
                let (path, hash) = value.rsplit_once('=').ok_or("--expect-sha256 needs <path>=<sha256>")?;
                if path.is_empty() || hash.is_empty() {
                    return Err("--expect-sha256 needs <path>=<sha256>".into());
                }
                expect.push((PathBuf::from(path), hash.to_string()));
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if json && mode == Mode::Apply {
        return Err("--json goes with --status or --dry-run".into());
    }
    if !expect.is_empty() && mode != Mode::Apply {
        return Err("--expect-sha256 goes with writing, not --status or --dry-run".into());
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
        expect,
    })
}

/// With fingerprints from a preview, refuses when a file to write is not
/// among them or no longer matches: the user confirmed another diff.
fn check_expected(plans: &[Plan], expect: &[(PathBuf, String)]) -> Result<(), String> {
    if expect.is_empty() {
        return Ok(());
    }
    for plan in plans.iter().filter(|p| p.changes()) {
        let Some((_, hash)) = expect.iter().find(|(path, _)| path == &plan.path) else {
            return Err(format!("{} was not in the preview; Coucou has not touched it", plan.path.display()));
        };
        if *hash != plan.fingerprint() {
            return Err(format!(
                "{} changed since the preview; Coucou has not touched it (preview again)",
                plan.path.display()
            ));
        }
    }
    Ok(())
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
        if opts.mode == Mode::Apply {
            check_expected(&plans, &opts.expect)?;
        }
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
        // Hermes loads every folder under plugins/: an empty `coucou` one goes too.
        if *agent == Agent::Hermes && opts.remove {
            if let Some(dir) = agent.file(&opts.home).parent() {
                let _ = std::fs::remove_dir(dir);
            }
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

    #[test]
    fn copilot_owns_its_file_and_removes_it_when_empty() {
        let foreign = json!({ "version": 1, "hooks": { "preToolUse": [{ "type": "command", "bash": "my-check" }] } });
        let v = transform(Agent::Copilot, &foreign, RELAY, false).unwrap();
        let pre = v["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[1], json!({ "type": "command", "bash": "'/usr/bin/coucou-hook' --agent copilot preToolUse", "timeoutSec": 10 }));
        assert_eq!(v["hooks"]["permissionRequest"][0]["timeoutSec"], 120);
        assert_eq!(v["version"], 1);
        assert_eq!(transform(Agent::Copilot, &v, RELAY, false).unwrap(), v);
        assert_eq!(transform(Agent::Copilot, &v, RELAY, true).unwrap(), foreign);

        let home = temp_home("copilot");
        let p = plans(Agent::Copilot, &home, RELAY, false).unwrap().remove(0);
        assert_eq!(p.path, home.join(".copilot/hooks/coucou.json"));
        apply(&p, 0).unwrap();
        assert!(status(Agent::Copilot, &home, None).installed);
        // Only Coucou's entries in it: removing them removes the file.
        let p = plans(Agent::Copilot, &home, RELAY, true).unwrap().remove(0);
        assert!(p.after.is_none() && p.changes());
        assert!(apply(&p, 0).unwrap().is_some(), "backed up first");
        assert!(!p.path.exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn cursor_takes_flat_camel_case_entries_and_a_version() {
        let existing = json!({ "hooks": { "stop": [{ "command": "notify-send done" }] } });
        let v = transform(Agent::Cursor, &existing, RELAY, false).unwrap();
        assert_eq!(v["version"], 1);
        let stop = v["hooks"]["stop"].as_array().unwrap();
        assert_eq!(stop[0]["command"], "notify-send done");
        assert_eq!(stop[1], json!({ "command": "'/usr/bin/coucou-hook' --agent cursor" }));
        assert_eq!(v["hooks"].as_object().unwrap().len(), CURSOR_EVENTS.len());
        assert_eq!(transform(Agent::Cursor, &v, RELAY, false).unwrap(), v);
        let removed = transform(Agent::Cursor, &v, RELAY, true).unwrap();
        assert_eq!(removed["hooks"], existing["hooks"]);
    }

    #[test]
    fn muse_uses_matchers_and_milliseconds() {
        let v = transform(Agent::Muse, &json!({}), RELAY, false).unwrap();
        assert_eq!(v["schema_version"], 1);
        let hook = &v["hooks"]["PermissionRequest"][0];
        assert_eq!(hook["matcher"], "*");
        assert_eq!(hook["hooks"][0]["command"], "'/usr/bin/coucou-hook' --agent muse PermissionRequest");
        assert_eq!(hook["hooks"][0]["timeout"], 120_000);
        // An existing config gets no schema_version of ours.
        let v = transform(Agent::Muse, &json!({ "theme": "x" }), RELAY, false).unwrap();
        assert!(v.get("schema_version").is_none());
        assert_eq!(transform(Agent::Muse, &v, RELAY, true).unwrap(), json!({ "theme": "x" }));
    }

    #[test]
    fn plugins_are_the_original_templates_with_this_relay() {
        let relay = "/home/me/bin/coucou-hook";
        let home = temp_home("plugins");
        for agent in [Agent::OpenCode, Agent::Amp, Agent::Hermes] {
            let plans = plans(agent, &home, relay, false).unwrap();
            assert_eq!(plans.len(), agent.files(&home).len());
            for p in &plans {
                apply(p, 0).unwrap();
            }
            let s = status(agent, &home, None);
            assert!(s.installed, "{agent:?}");
            assert_eq!(s.relays, vec![relay.to_string()]);
        }
        let js = std::fs::read_to_string(home.join(".config/opencode/plugins/coucou.js")).unwrap();
        assert_eq!(js, OPENCODE_PLUGIN.replace("{HOOK}", "\"/home/me/bin/coucou-hook\""));
        assert!(js.contains("const HOOK = \"/home/me/bin/coucou-hook\";"));
        let ts = std::fs::read_to_string(home.join(".config/amp/plugins/coucou.ts")).unwrap();
        assert_eq!(ts, AMP_PLUGIN.replace("{HOOK}", "\"/home/me/bin/coucou-hook\""));
        let yaml = std::fs::read_to_string(home.join(".hermes/plugins/coucou/plugin.yaml")).unwrap();
        assert_eq!(yaml, HERMES_PLUGIN_YAML);
        // Installing again changes nothing.
        assert!(!plans(Agent::Amp, &home, relay, false).unwrap().iter().any(Plan::changes));

        // Removal takes the files away, and Hermes' folder with them.
        let mut out = Vec::new();
        let opts = Options {
            agents: vec![Agent::Hermes, Agent::OpenCode],
            yes: true,
            remove: true,
            home: home.clone(),
            relay: relay.into(),
            mode: Mode::Apply,
            json: false,
            expect: Vec::new(),
        };
        run(&opts, &mut "".as_bytes(), &mut out).unwrap();
        // The plugin files are gone; only their dated backups stay (the folder
        // itself goes once it is empty).
        let left: Vec<String> = std::fs::read_dir(home.join(".hermes/plugins/coucou"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|n| n.contains(".coucou-backup-")), "{left:?}");
        assert!(!home.join(".config/opencode/plugins/coucou.js").exists());
        assert!(home.join(".config/amp/plugins/coucou.ts").exists());

        // A file of the same name that Coucou did not write is never touched.
        std::fs::write(home.join(".config/amp/plugins/coucou.ts"), "// mine\n").unwrap();
        assert!(plans(Agent::Amp, &home, relay, false).is_err());
        assert!(plans(Agent::Amp, &home, relay, true).is_err());
        assert!(!status(Agent::Amp, &home, None).installed);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn every_agent_has_a_distinct_id_and_a_tag_on_its_entries() {
        for (i, a) in Agent::ALL.iter().enumerate() {
            assert_eq!(Agent::from_id(a.id()), Some(*a));
            assert!(Agent::ALL[i + 1..].iter().all(|b| b.id() != a.id()));
        }
        assert_eq!(Agent::ALL.len(), 10);
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
            expect: Vec::new(),
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

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn dry_run_reports_each_file_fingerprint() {
        let home = temp_home("fingerprint");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"model\":\"opus\"}\n").unwrap();
        let v = dry_run_json(&[Agent::Claude, Agent::Codex], &home, RELAY, false, 0);
        assert_eq!(v["agents"][0]["files"][0]["sha256"], sha256_hex(b"{\"model\":\"opus\"}\n").as_str());
        // A file that does not exist yet has the "absent" fingerprint.
        assert_eq!(v["agents"][1]["files"][0]["sha256"], ABSENT);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn expect_sha256_parses_and_goes_with_writing_only() {
        let o = parse_args(&args(&["--agent", "claude", "--yes", "--expect-sha256", "/h/.claude/settings.json=abc", "--home", "/h"])).unwrap();
        assert_eq!(o.expect, vec![(PathBuf::from("/h/.claude/settings.json"), "abc".to_string())]);
        // The path may itself hold '=': the hash is after the last one.
        let o = parse_args(&args(&["--expect-sha256", "/h/a=b.json=absent", "--home", "/h"])).unwrap();
        assert_eq!(o.expect, vec![(PathBuf::from("/h/a=b.json"), "absent".to_string())]);
        assert!(parse_args(&args(&["--expect-sha256", "nohash", "--home", "/h"])).is_err());
        assert!(parse_args(&args(&["--expect-sha256", "--home", "/h"])).is_err());
        assert!(parse_args(&args(&["--dry-run", "--expect-sha256", "/x=abc", "--home", "/h"])).is_err());
    }

    fn apply_opts(home: &Path, expect: Vec<(PathBuf, String)>) -> Options {
        Options {
            agents: vec![Agent::Claude],
            yes: true,
            remove: false,
            home: home.to_path_buf(),
            relay: RELAY.into(),
            mode: Mode::Apply,
            json: false,
            expect,
        }
    }

    #[test]
    fn a_file_changed_since_the_preview_is_never_written() {
        let home = temp_home("expect");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"model\":\"opus\"}\n").unwrap();
        let previewed = dry_run_json(&[Agent::Claude], &home, RELAY, false, 0);
        let hash = previewed["agents"][0]["files"][0]["sha256"].as_str().unwrap().to_string();

        // Someone edits the file between the preview and the click.
        std::fs::write(&file, "{\"model\":\"sonnet\"}\n").unwrap();
        let err = run(&apply_opts(&home, vec![(file.clone(), hash.clone())]), &mut "".as_bytes(), &mut Vec::new()).unwrap_err();
        assert!(err.contains("changed since the preview"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"model\":\"sonnet\"}\n");
        let backups = std::fs::read_dir(file.parent().unwrap()).unwrap().count();
        assert_eq!(backups, 1, "no backup either");

        // A file to write that the preview did not fingerprint is refused too.
        let other = home.join("elsewhere.json");
        let err = run(&apply_opts(&home, vec![(other, hash)]), &mut "".as_bytes(), &mut Vec::new()).unwrap_err();
        assert!(err.contains("not in the preview"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"model\":\"sonnet\"}\n");

        // The fingerprint of what is on disk now: written.
        let now = sha256_hex(b"{\"model\":\"sonnet\"}\n");
        run(&apply_opts(&home, vec![(file.clone(), now)]), &mut "".as_bytes(), &mut Vec::new()).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("coucou-hook"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_file_created_since_the_preview_is_never_written() {
        let home = temp_home("expect-absent");
        let file = Agent::Claude.file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{}\n").unwrap();
        let err = run(&apply_opts(&home, vec![(file.clone(), ABSENT.into())]), &mut "".as_bytes(), &mut Vec::new()).unwrap_err();
        assert!(err.contains("changed since the preview"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}\n");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
