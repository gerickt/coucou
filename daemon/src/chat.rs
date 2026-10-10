//! The chat, whoever answers it — a port of the original's chat.rs, claude.rs,
//! openai_compat.rs and local_chat.rs, streaming every provider.
//!
//! ```text
//! coucou-daemon chat --request <file>       one turn; JSON lines on stdout
//! coucou-daemon chat-models <provider> [--base-url <url>]
//! ```
//!
//! The plugin writes the turn (provider, model, plain-text history, the new
//! question, an attached file's path) to a private file in $XDG_RUNTIME_DIR and
//! names it; the file is removed as soon as it is read. API keys are looked up
//! here, in the Secret Service, and only ever go to curl's stdin (net.rs): the
//! plugin never sees them. Lines out:
//!
//! ```text
//! {"type":"start","pid":123}            kill this pid to stop the turn
//! {"type":"delta","text":"…"}           the visible answer so far (≤ 15/s)
//! {"type":"done","text":"…"}            the whole answer, <think> blocks gone
//! {"type":"error","message":"…"}
//! ```

use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::net::{self, Failure, Request, Url};
use crate::secrets;

const MAX_TOKENS: u32 = 4096;
const ANTHROPIC_DEFAULT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Server-side fallback: on a policy decline the API retries on a fallback model.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5";
/// Cloud providers: text files inline up to this size, larger ones by name.
const MAX_INLINE_TEXT: u64 = 200_000;
/// Cloud providers: images inline as data up to this size.
const MAX_IMAGE: u64 = 5_000_000;
/// Model servers: a text file inline up to this many characters, then cut.
const MAX_INLINE_CHARS: usize = 24_000;
const DELTA_INTERVAL: Duration = Duration::from_millis(1000 / 15);
const MAX_ANSWER: usize = 4 * 1024 * 1024;
const MAX_REQUEST_FILE: u64 = 8 * 1024 * 1024;
/// Ollama and LM Studio ignore the key, but some servers insist on one.
const NO_KEY: &str = "ollama";
const LOCAL_NOT_CHAT: &[&str] = &["embed", "bge-", "all-minilm", "clip", "rerank"];

// ── Providers ────────────────────────────────────────────────────────────────

/// Cloud providers that speak the OpenAI chat completions API.
pub struct Cloud {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub key: &'static str,
    pub default_model: &'static str,
    /// Model ids containing any of these are not chat models.
    pub not_chat: &'static [&'static str],
    /// OpenAI's reasoning models want `max_completion_tokens`.
    pub max_tokens_field: &'static str,
}

pub const CLOUDS: &[Cloud] = &[
    Cloud {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        key: "openai-api-key",
        default_model: "gpt-4o",
        not_chat: &[
            "embed", "tts", "whisper", "dall-e", "audio", "realtime", "moderat", "codex", "computer-use",
            "transcribe", "image", "sora", "babbage", "davinci", "instruct",
        ],
        max_tokens_field: "max_completion_tokens",
    },
    Cloud {
        id: "google",
        name: "Google AI",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        key: "google-api-key",
        default_model: "gemini-2.0-flash",
        not_chat: &["embed", "imagen", "veo", "aqa", "tts", "audio", "live"],
        max_tokens_field: "max_tokens",
    },
    Cloud {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        key: "openrouter-api-key",
        default_model: "openrouter/auto",
        not_chat: &[],
        max_tokens_field: "max_tokens",
    },
];

/// Who answers a turn.
pub enum Provider {
    Anthropic,
    Cloud(&'static Cloud),
    /// Ollama, LM Studio or the user's OpenAI-compatible server.
    Local { id: &'static str, name: &'static str },
}

impl Provider {
    pub fn from_id(id: &str) -> Option<Provider> {
        match id {
            "anthropic" | "" => Some(Provider::Anthropic),
            "ollama" => Some(Provider::Local { id: "ollama", name: "Ollama" }),
            "lmstudio" => Some(Provider::Local { id: "lmstudio", name: "LM Studio" }),
            "custom" => Some(Provider::Local { id: "custom", name: "OpenAI-compatible server" }),
            other => CLOUDS.iter().find(|c| c.id == other).map(Provider::Cloud),
        }
    }
}

/// A model server's address: the one in the settings, else its usual one on
/// this machine (Ollama's own OLLAMA_HOST first, as the Ollama CLI does).
fn server_url(id: &str, configured: Option<&str>) -> Result<Url, String> {
    let configured = configured.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let raw = match (configured, id) {
        (Some(raw), _) => raw,
        (None, "ollama") => std::env::var("OLLAMA_HOST")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:11434".into()),
        (None, "lmstudio") => "http://127.0.0.1:1234".into(),
        (None, _) => return Err("Set the server address in Mochi's chat settings first.".into()),
    };
    net::normalise_server_url(&raw)
}

/// A key may only travel over https, or to this machine.
fn may_carry_key(url: &Url) -> bool {
    url.scheme == "https" || url.is_loopback()
}

/// The custom server's key is stored with the one address it was entered for
/// (`{"url","key"}`, as the original's Local models section stored it).
pub fn bind_custom_key(typed_url: &str, key: &str) -> Result<String, String> {
    let url = net::normalise_server_url(typed_url)?;
    if !may_carry_key(&url) {
        return Err("A key is only sent over https, or to a server on this computer.".into());
    }
    let bound = json!({ "url": url.to_string().trim_end_matches('/'), "key": key.trim() });
    Ok(bound.to_string())
}

/// The custom key, only if it was entered for exactly this address.
fn custom_key_for(stored: &str, url: &Url) -> Option<String> {
    let bound: Value = serde_json::from_str(stored).ok()?;
    let key = bound.get("key")?.as_str()?;
    (may_carry_key(url) && bound.get("url")?.as_str()? == url.to_string().trim_end_matches('/') && !key.is_empty())
        .then(|| key.to_string())
}

fn anthropic_endpoint(env_base: Option<&str>) -> Result<Url, String> {
    match env_base.map(str::trim).filter(|v| !v.is_empty()) {
        Some(raw) => net::anthropic_endpoint(raw),
        None => Url::parse(ANTHROPIC_DEFAULT),
    }
}

// ── The turn the plugin asks for ─────────────────────────────────────────────

pub struct Turn {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    /// Earlier turns, plain text, oldest first.
    pub history: Vec<(String, String)>,
    pub question: String,
    /// A file attached to this question: (name, path).
    pub file: Option<(String, String)>,
}

pub fn parse_turn(text: &str) -> Result<Turn, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad request: {e}"))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut history = Vec::new();
    for m in v.get("history").and_then(Value::as_array).into_iter().flatten() {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("");
        let content = m.get("content").and_then(Value::as_str).unwrap_or("");
        if !matches!(role, "user" | "assistant") || content.is_empty() {
            return Err("bad request: history".into());
        }
        history.push((role.to_string(), content.to_string()));
    }
    let question = s("question");
    if question.trim().is_empty() {
        return Err("bad request: empty question".into());
    }
    let file = v.get("file").filter(|f| f.is_object()).map(|f| {
        let g = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        (g("name"), g("path"))
    });
    Ok(Turn {
        provider: s("provider"),
        model: s("model"),
        base_url: v.get("base_url").and_then(Value::as_str).map(str::to_string),
        history,
        question,
        file,
    })
}

// ── System prompt (chat.rs system_prompt_for) ────────────────────────────────

pub fn system_prompt(first_name: Option<&str>, web_search: bool) -> String {
    let opening = match first_name {
        Some(name) => format!("You are Mochi, {name}'s personal AI assistant living at the top of their screen."),
        None => "You are Mochi, a personal AI assistant living at the top of the user's screen.".to_string(),
    };
    let abilities = if web_search {
        "You have web search access and can help with absolutely anything — research, coding, finding places, recommendations, tasks, questions."
    } else {
        "You can help with absolutely anything — research, coding, recommendations, tasks, questions. You have no web access: say so when something needs current information."
    };
    format!(
        "{opening} {abilities} \
Respond in the user's language. Be thorough and complete — use as much detail as the task requires. \
Use light Markdown when it helps: short paragraphs, bullet lists, **bold**, `inline code` and fenced code blocks. Avoid tables and big headings: the chat window is small."
    )
}

fn is_handle(name: &str) -> bool {
    !name.contains(' ') && (name.to_lowercase() == name || name.chars().any(|c| c.is_ascii_digit() || "._-@".contains(c)))
}

/// "Théodore Riant" -> "Théodore"; a login handle or an odd name -> None
/// (identity.rs first_name_from).
pub fn first_name_from(full: &str) -> Option<String> {
    let full = full.trim();
    if full.is_empty() || full.chars().count() > 32 || is_handle(full) {
        return None;
    }
    let first = full.split(' ').next()?;
    first.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '-').then(|| first.to_string())
}

/// The account's first name, from the GECOS field.
pub fn first_name() -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_gecos.is_null() {
            return None;
        }
        let gecos = std::ffi::CStr::from_ptr((*pw).pw_gecos).to_string_lossy().into_owned();
        first_name_from(gecos.split(',').next().unwrap_or(""))
    }
}

// ── Files ────────────────────────────────────────────────────────────────────

fn extension(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

fn image_type(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// A regular file's size; None for anything else (folders, devices).
fn file_len(path: &str) -> Option<u64> {
    std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len())
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Claude: PDF -> document block, image -> image block, text -> inline text.
fn anthropic_file_block(path: &str) -> Option<Value> {
    let ext = extension(path);
    let len = file_len(path)?;
    let media = if ext == "pdf" { Some(("document", "application/pdf")) } else { image_type(&ext).map(|m| ("image", m)) };
    if let Some((kind, media)) = media {
        let bytes = std::fs::read(path).ok()?;
        return Some(json!({ "type": kind, "source": { "type": "base64", "media_type": media, "data": base64(&bytes) } }));
    }
    if len > MAX_INLINE_TEXT {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    Some(json!({ "type": "text", "text": format!("File contents:\n{text}") }))
}

/// The first `limit` bytes of a file as UTF-8 (a character cut at the end is dropped).
fn read_prefix(path: &str, limit: usize) -> Option<String> {
    file_len(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(limit as u64).read_to_end(&mut bytes).ok()?;
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(e) if e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()
        }
        Err(_) => None,
    }
}

/// Model servers: text inline (cut at 24 000 characters), anything else by name.
pub fn local_file_note(name: &str, path: &str) -> String {
    let ext = extension(path);
    let binary = ext == "pdf" || image_type(&ext).is_some();
    let text = if binary { None } else { read_prefix(path, MAX_INLINE_CHARS * 4) };
    match text {
        Some(text) if !text.is_empty() => {
            let mut body: String = text.chars().take(MAX_INLINE_CHARS).collect();
            if body.len() < text.len() {
                body.push_str("\n[truncated]");
            }
            format!("File: {name}\nFile contents:\n{body}")
        }
        _ => format!("File: {name}"),
    }
}

// ── Request bodies ───────────────────────────────────────────────────────────

fn plain_history(turn: &Turn) -> Vec<Value> {
    turn.history.iter().map(|(role, content)| json!({ "role": role, "content": content })).collect()
}

pub fn anthropic_body(turn: &Turn, model: &str, system: &str) -> Value {
    let mut content = Vec::new();
    if let Some((name, path)) = &turn.file {
        if let Some(block) = anthropic_file_block(path) {
            content.push(block);
        }
        content.push(json!({ "type": "text", "text": format!("File: {name}") }));
    }
    content.push(json!({ "type": "text", "text": turn.question }));
    let mut messages = plain_history(turn);
    messages.push(json!({ "role": "user", "content": content }));
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "tools": [{ "type": "web_search_20260209", "name": "web_search", "max_uses": 5 }],
        "fallbacks": "default",
        "messages": messages,
        "stream": true,
    })
}

/// Cloud OpenAI-compatible: images as data URLs, text inline, PDFs by name.
fn cloud_user_message(turn: &Turn) -> Value {
    let mut prefix = String::new();
    let mut image = None;
    if let Some((name, path)) = &turn.file {
        let ext = extension(path);
        let len = file_len(path).unwrap_or(u64::MAX);
        prefix = format!("File: {name}\n\n");
        if let Some(media) = image_type(&ext) {
            if len <= MAX_IMAGE {
                if let Ok(bytes) = std::fs::read(path) {
                    let url = format!("data:{media};base64,{}", base64(&bytes));
                    image = Some(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
            }
        } else if ext != "pdf" && len <= MAX_INLINE_TEXT {
            if let Ok(body) = std::fs::read_to_string(path) {
                prefix = format!("File: {name}\nFile contents:\n{body}\n\n");
            }
        }
    }
    let text = format!("{prefix}{}", turn.question);
    match image {
        None => json!({ "role": "user", "content": text }),
        Some(part) => json!({ "role": "user", "content": [part, { "type": "text", "text": text }] }),
    }
}

pub fn openai_body(turn: &Turn, model: &str, system: &str, max_tokens_field: &str, user: Value) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    messages.extend(plain_history(turn));
    messages.push(user);
    let mut body = json!({ "model": model, "messages": messages, "stream": true });
    body[max_tokens_field] = json!(MAX_TOKENS);
    body
}

/// What a prepared turn needs while its answer streams in.
pub struct Prepared {
    pub request: Request,
    pub wire: Wire,
    /// For messages: the provider's name, and a model server's address.
    pub name: String,
    pub model: String,
    pub server: Option<Url>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Wire {
    Anthropic,
    OpenAi,
}

pub struct Env<'a> {
    pub secret: &'a dyn Fn(&str) -> Option<String>,
    pub first_name: Option<&'a str>,
    pub anthropic_base: Option<&'a str>,
}

pub fn prepare(turn: &Turn, env: &Env) -> Result<Prepared, String> {
    let provider = Provider::from_id(&turn.provider).ok_or_else(|| format!("Unknown chat provider: {}", turn.provider))?;
    let model = turn.model.trim().to_string();
    match provider {
        Provider::Anthropic => {
            let key = (env.secret)("anthropic-api-key").ok_or("API key missing. Open settings.")?;
            let endpoint = anthropic_endpoint(env.anthropic_base)?;
            let model = if model.is_empty() { ANTHROPIC_DEFAULT_MODEL.to_string() } else { model };
            let body = anthropic_body(turn, &model, &system_prompt(env.first_name, true));
            Ok(Prepared {
                request: Request {
                    url: endpoint.to_string(),
                    headers: vec![
                        format!("x-api-key: {key}"),
                        format!("anthropic-version: {ANTHROPIC_VERSION}"),
                        format!("anthropic-beta: {FALLBACK_BETA}"),
                        "content-type: application/json".into(),
                        "accept: text/event-stream".into(),
                    ],
                    body: Some(body.to_string()),
                    max_time_secs: 300,
                    allow_http: endpoint.scheme == "http",
                    no_proxy: endpoint.is_loopback(),
                },
                wire: Wire::Anthropic,
                name: "Claude".into(),
                model,
                server: None,
            })
        }
        Provider::Cloud(c) => {
            let key = (env.secret)(c.key).ok_or_else(|| format!("{} API key missing. Add it in Settings.", c.name))?;
            let model = if model.is_empty() { c.default_model.to_string() } else { model };
            let body = openai_body(turn, &model, &system_prompt(env.first_name, false), c.max_tokens_field, cloud_user_message(turn));
            Ok(Prepared {
                request: Request {
                    url: format!("{}/chat/completions", c.base_url),
                    headers: vec![format!("Authorization: Bearer {key}"), "content-type: application/json".into()],
                    body: Some(body.to_string()),
                    max_time_secs: 300,
                    allow_http: false,
                    no_proxy: false,
                },
                wire: Wire::OpenAi,
                name: c.name.into(),
                model,
                server: None,
            })
        }
        Provider::Local { id, name } => {
            let base = server_url(id, turn.base_url.as_deref())?;
            if model.is_empty() {
                return Err("Pick a model above the chat box first.".into());
            }
            let key = if id == "custom" {
                (env.secret)("openai-compatible-key").and_then(|stored| custom_key_for(&stored, &base))
            } else {
                None
            };
            let text = match &turn.file {
                Some((fname, path)) => format!("{}\n\n{}", local_file_note(fname, path), turn.question),
                None => turn.question.clone(),
            };
            let user = json!({ "role": "user", "content": text });
            let body = openai_body(turn, &model, &system_prompt(env.first_name, false), "max_tokens", user);
            Ok(Prepared {
                request: Request {
                    url: net::join(&base, "v1/chat/completions"),
                    headers: vec![
                        format!("Authorization: Bearer {}", key.as_deref().unwrap_or(NO_KEY)),
                        "content-type: application/json".into(),
                    ],
                    body: Some(body.to_string()),
                    max_time_secs: 300,
                    allow_http: true,
                    no_proxy: base.is_loopback(),
                },
                wire: Wire::OpenAi,
                name: name.into(),
                model,
                server: Some(base),
            })
        }
    }
}

// ── Streaming ────────────────────────────────────────────────────────────────

/// One server-sent event line: `Some(Ok(text))` for a text delta,
/// `Some(Err(message))` for an error event, None for anything else.
pub fn parse_sse_line(wire: Wire, line: &str) -> Option<Result<String, String>> {
    let payload = line.strip_prefix("data:")?.trim_start();
    if payload == "[DONE]" {
        return None;
    }
    let json: Value = serde_json::from_str(payload).ok()?;
    if let Some(msg) = json.pointer("/error/message").and_then(Value::as_str) {
        return Some(Err(msg.to_string()));
    }
    match wire {
        Wire::OpenAi => json.pointer("/choices/0/delta/content")?.as_str().map(|s| Ok(s.to_string())),
        Wire::Anthropic => match json.get("type").and_then(Value::as_str)? {
            "content_block_delta" if json.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") => {
                json.pointer("/delta/text")?.as_str().map(|s| Ok(s.to_string()))
            }
            // A policy decline ends the stream with stop_reason "refusal".
            "message_delta" if json.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("refusal") => {
                Some(Err(json
                    .pointer("/delta/stop_details/explanation")
                    .and_then(Value::as_str)
                    .unwrap_or("Claude declined this one.")
                    .to_string()))
            }
            _ => None,
        },
    }
}

/// Removes finished `<think>…</think>` blocks (reasoning models).
pub fn filter_thinking_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        let Some(len) = rest[start..].find("</think>") else { break };
        out.push_str(&rest[..start]);
        rest = &rest[start + len + "</think>".len()..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// While streaming: finished blocks go, and so does everything after a block
/// that is still open.
pub fn progressive_filter(text: &str) -> String {
    let cleaned = filter_thinking_blocks(text);
    match cleaned.find("<think>") {
        Some(at) => cleaned[..at].trim().to_string(),
        None => cleaned,
    }
}

fn status_error(p: &Prepared, status: u16, detail: &str) -> String {
    let name = &p.name;
    if p.wire == Wire::Anthropic {
        return format!("Claude API {status}: {detail}");
    }
    if p.server.is_some() {
        return match status {
            404 => format!("{} isn't installed. Pick another model above the chat box.", p.model),
            401 | 403 => "The server refused the key. Check it, then connect again.".into(),
            _ if detail.is_empty() => format!("HTTP {status}"),
            _ => detail.to_string(),
        };
    }
    match status {
        401 | 403 => format!("{name} rejected the API key ({status}). Check it in Settings."),
        402 => format!("{name}: not enough credits (402). {detail}"),
        404 => format!("{name}: model not found (404). Pick another one above the chat box. {detail}"),
        429 => format!("{name} rate limit reached (429): {detail}"),
        _ => format!("{name} {status}: {detail}"),
    }
}

fn failure_message(p: &Prepared, failure: Failure) -> String {
    match failure {
        Failure::Status(status, body) => status_error(p, status, &net::error_detail(&body)),
        Failure::Transport(msg) => match &p.server {
            Some(url) => format!("Cannot reach {}. Is the server running?", url.to_string().trim_end_matches('/')),
            None => format!("Network error: {msg}"),
        },
        Failure::Other(msg) => msg,
    }
}

fn emit(out: &mut dyn Write, line: Value) -> bool {
    writeln!(out, "{line}").and_then(|_| out.flush()).is_ok()
}

/// Streams one prepared turn, writing deltas, then `done` or `error`.
pub fn stream_turn(p: &Prepared, out: &mut dyn Write) {
    let mut accumulated = String::new();
    let mut last = Instant::now().checked_sub(DELTA_INTERVAL).unwrap_or_else(Instant::now);
    let mut shown = String::new();
    let result = net::stream(&p.request, |line| {
        match parse_sse_line(p.wire, line) {
            Some(Ok(delta)) => accumulated.push_str(&delta),
            Some(Err(message)) => return Err(message),
            None => return Ok(()),
        }
        if accumulated.len() > MAX_ANSWER {
            return Err("The server's answer is too large.".into());
        }
        if last.elapsed() >= DELTA_INTERVAL {
            last = Instant::now();
            let visible = progressive_filter(&accumulated);
            if visible != shown && !visible.is_empty() {
                if !emit(out, json!({ "type": "delta", "text": visible })) {
                    return Err("stdout closed".into());
                }
                shown = visible;
            }
        }
        Ok(())
    });
    match result {
        Err(failure) => {
            emit(out, json!({ "type": "error", "message": failure_message(p, failure) }));
        }
        Ok(()) => {
            let answer = filter_thinking_blocks(&accumulated);
            if answer.is_empty() {
                emit(out, json!({ "type": "error", "message": "No response text." }));
            } else {
                emit(out, json!({ "type": "done", "text": answer }));
            }
        }
    }
}

/// The turn file: a regular file directly in the runtime directory. Removed
/// once read, whatever happens next.
fn read_turn_file(path: &Path, runtime: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("request: {e}"))?;
    let inside = path.parent().and_then(|p| p.canonicalize().ok()) == runtime.canonicalize().ok();
    if !meta.is_file() || !inside || meta.len() > MAX_REQUEST_FILE {
        return Err("request: not a turn file".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("request: {e}"));
    let _ = std::fs::remove_file(path);
    text
}

/// `coucou-daemon chat --request <file>`.
pub fn run(args: &[String], out: &mut dyn Write) -> bool {
    emit(out, json!({ "type": "start", "pid": std::process::id() }));
    let prepared = match args {
        [flag, path] if flag == "--request" => {
            read_turn_file(Path::new(path), &crate::paths::runtime_dir()).and_then(|text| parse_turn(&text)).and_then(|turn| {
                let name = first_name();
                let base = std::env::var("COUCOU_ANTHROPIC_BASE_URL").ok();
                prepare(&turn, &Env { secret: &secrets::get, first_name: name.as_deref(), anthropic_base: base.as_deref() })
            })
        }
        _ => Err("usage: coucou-daemon chat --request <file>".into()),
    };
    match prepared {
        Ok(p) => {
            stream_turn(&p, out);
            true
        }
        Err(message) => {
            emit(out, json!({ "type": "error", "message": message }));
            false
        }
    }
}

// ── Model lists ──────────────────────────────────────────────────────────────

pub fn models_request(provider: &str, base_url: Option<&str>, env: &Env) -> Result<(Request, Prepared), String> {
    let none = |name: &str, server: Option<Url>| Prepared {
        request: Request { url: String::new(), headers: vec![], body: None, max_time_secs: 15, allow_http: false, no_proxy: false },
        wire: Wire::OpenAi,
        name: name.into(),
        model: String::new(),
        server,
    };
    let no_key = "No API key — add it in Settings.";
    match Provider::from_id(provider).ok_or_else(|| format!("Unknown chat provider: {provider}"))? {
        Provider::Anthropic => {
            let key = (env.secret)("anthropic-api-key").ok_or(no_key)?;
            let endpoint = anthropic_endpoint(env.anthropic_base)?;
            let mut url = endpoint.clone();
            url.path = format!("{}/models", url.path.trim_end_matches("/messages"));
            let mut p = none("Claude", None);
            p.wire = Wire::Anthropic;
            let req = Request {
                url: format!("{url}?limit=100"),
                headers: vec![format!("x-api-key: {key}"), format!("anthropic-version: {ANTHROPIC_VERSION}")],
                body: None,
                max_time_secs: 15,
                allow_http: endpoint.scheme == "http",
                no_proxy: endpoint.is_loopback(),
            };
            Ok((req, p))
        }
        Provider::Cloud(c) => {
            let key = (env.secret)(c.key).ok_or(no_key)?;
            let req = Request {
                url: format!("{}/models", c.base_url),
                headers: vec![format!("Authorization: Bearer {key}")],
                body: None,
                max_time_secs: 15,
                allow_http: false,
                no_proxy: false,
            };
            Ok((req, none(c.name, None)))
        }
        Provider::Local { id, name } => {
            let base = server_url(id, base_url)?;
            let key = if id == "custom" {
                (env.secret)("openai-compatible-key").and_then(|stored| custom_key_for(&stored, &base))
            } else {
                None
            };
            let req = Request {
                url: net::join(&base, "v1/models"),
                headers: vec![format!("Authorization: Bearer {}", key.as_deref().unwrap_or(NO_KEY))],
                body: None,
                max_time_secs: 5,
                allow_http: true,
                no_proxy: base.is_loopback(),
            };
            Ok((req, none(name, Some(base))))
        }
    }
}

fn is_free(m: &Value, id: &str) -> bool {
    let zero = |ptr: &str| m.pointer(ptr).and_then(Value::as_str).is_some_and(|v| v.parse::<f64>().ok() == Some(0.0));
    id.ends_with(":free") || (zero("/pricing/prompt") && zero("/pricing/completion"))
}

/// `[{id, label}]` from a provider's model list, chat models only, in the
/// original's order (OpenAI newest first, OpenRouter free first then by name).
pub fn parse_models(provider: &str, json: &Value) -> Vec<Value> {
    let items = json.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    let cloud = CLOUDS.iter().find(|c| c.id == provider);
    let local = matches!(provider, "ollama" | "lmstudio" | "custom");
    let mut models: Vec<(String, String, i64, bool)> = items
        .iter()
        .filter(|m| {
            m.pointer("/architecture/output_modalities")
                .and_then(Value::as_array)
                .is_none_or(|mods| mods.iter().any(|x| x.as_str() == Some("text")))
        })
        .filter_map(|m| {
            let raw = m.get("id")?.as_str()?;
            let id = raw.strip_prefix("models/").unwrap_or(raw).to_string();
            let lower = id.to_lowercase();
            let not_chat: &[&str] = if local { LOCAL_NOT_CHAT } else { cloud.map(|c| c.not_chat).unwrap_or(&[]) };
            if id.is_empty() || not_chat.iter().any(|x| lower.contains(x)) {
                return None;
            }
            let created = m.get("created").and_then(Value::as_i64).unwrap_or(0);
            let free = provider == "openrouter" && is_free(m, &id);
            let name = m.get("display_name").or_else(|| m.get("name")).and_then(Value::as_str).filter(|_| !local).unwrap_or(&id);
            let label = if free && !name.to_lowercase().contains("(free)") { format!("{name} (free)") } else { name.to_string() };
            Some((id, label, created, free))
        })
        .collect();
    match provider {
        "openai" => models.sort_by_key(|m| std::cmp::Reverse(m.2)),
        "openrouter" => models.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))),
        _ => {}
    }
    models.into_iter().map(|(id, label, _, _)| json!({ "id": id, "label": label })).collect()
}

/// `coucou-daemon chat-models <provider> [--base-url <url>]`: one JSON line,
/// `{"models":[{"id","label"}]}` or `{"error":"…"}`.
pub fn run_models(args: &[String], out: &mut dyn Write) -> bool {
    let (provider, base) = match args {
        [p] => (p.as_str(), None),
        [p, flag, url] if flag == "--base-url" => (p.as_str(), Some(url.as_str())),
        _ => {
            emit(out, json!({ "error": "usage: coucou-daemon chat-models <provider> [--base-url <url>]" }));
            return false;
        }
    };
    let env_base = std::env::var("COUCOU_ANTHROPIC_BASE_URL").ok();
    let env = Env { secret: &secrets::get, first_name: None, anthropic_base: env_base.as_deref() };
    let result = models_request(provider, base, &env).and_then(|(req, p)| {
        let body = net::fetch(&req, net::MAX_BODY).map_err(|f| failure_message(&p, f))?;
        let json: Value = serde_json::from_slice(&body).map_err(|_| "Unexpected API response.".to_string())?;
        let models = parse_models(provider, &json);
        if models.is_empty() && p.server.is_some() {
            return Err(format!("No models yet. Download one in {} first.", p.name));
        }
        Ok(models)
    });
    match result {
        Ok(models) => emit(out, json!({ "models": models })),
        Err(e) => {
            emit(out, json!({ "error": e }));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(provider: &str, model: &str) -> Turn {
        Turn {
            provider: provider.into(),
            model: model.into(),
            base_url: None,
            history: vec![("user".into(), "hi".into()), ("assistant".into(), "hello".into())],
            question: "why?".into(),
            file: None,
        }
    }

    fn keys(name: &str) -> Option<String> {
        match name {
            "anthropic-api-key" => Some("sk-ant-SECRET".into()),
            "openai-api-key" => Some("sk-oa-SECRET".into()),
            "openai-compatible-key" => Some(r#"{"url":"https://llm.lan","key":"sk-c-SECRET"}"#.into()),
            _ => None,
        }
    }

    fn env() -> Env<'static> {
        Env { secret: &keys, first_name: Some("Louis"), anthropic_base: None }
    }

    #[test]
    fn the_prompt_greets_by_first_name_and_claims_web_search_only_for_claude() {
        let p = system_prompt(Some("Louis"), true);
        assert!(p.starts_with("You are Mochi, Louis's personal AI assistant living at the top of their screen."));
        assert!(p.contains("web search access") && p.contains("light Markdown"));
        let p = system_prompt(None, false);
        assert!(p.starts_with("You are Mochi, a personal AI assistant living at the top of the user's screen."));
        assert!(!p.contains("web search") && p.contains("no web access"));
    }

    #[test]
    fn first_names_follow_identity_rules() {
        assert_eq!(first_name_from("Théodore Riant").as_deref(), Some("Théodore"));
        assert_eq!(first_name_from("Jean-Luc Picard"), Some("Jean-Luc".into()));
        assert_eq!(first_name_from("gerick"), None);
        assert_eq!(first_name_from("dev_42"), None);
        assert_eq!(first_name_from("R2D2 Unit"), None);
        assert_eq!(first_name_from(""), None);
    }

    #[test]
    fn a_turn_file_is_parsed_and_bad_ones_refused() {
        let t = parse_turn(r#"{"provider":"openai","model":"m","history":[{"role":"user","content":"a"}],"question":"q","file":{"name":"a.txt","path":"/x/a.txt"}}"#).unwrap();
        assert_eq!((t.provider.as_str(), t.history.len(), t.question.as_str()), ("openai", 1, "q"));
        assert_eq!(t.file, Some(("a.txt".into(), "/x/a.txt".into())));
        assert!(parse_turn(r#"{"provider":"openai","question":"  "}"#).is_err());
        assert!(parse_turn(r#"{"history":[{"role":"system","content":"x"}],"question":"q"}"#).is_err());
    }

    #[test]
    fn anthropic_streams_with_web_search_history_and_the_key_only_in_a_header() {
        let p = prepare(&turn("anthropic", ""), &env()).unwrap();
        assert_eq!(p.request.url, ANTHROPIC_DEFAULT);
        assert_eq!(p.wire, Wire::Anthropic);
        let body: Value = serde_json::from_str(p.request.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["model"], ANTHROPIC_DEFAULT_MODEL);
        assert_eq!(body["stream"], true);
        assert_eq!(body["tools"][0]["name"], "web_search");
        assert!(body["system"].as_str().unwrap().contains("Louis's"));
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(body["messages"][2]["content"][0]["text"], "why?");
        assert!(p.request.headers.contains(&"x-api-key: sk-ant-SECRET".to_string()));
        assert!(!p.request.body.as_ref().unwrap().contains("SECRET"));
        assert!(!p.request.url.contains("SECRET"));
        assert!(!p.request.allow_http);
        // The key is on curl's stdin (the config), never on its command line.
        let cfg = net::config(&p.request).unwrap();
        assert!(cfg.contains("sk-ant-SECRET"));
        assert!(!net::CURL_ARGV.join(" ").contains("SECRET"));
    }

    #[test]
    fn the_anthropic_gateway_is_honoured_and_must_be_https() {
        let e = Env { secret: &keys, first_name: None, anthropic_base: Some("https://gw.example.com") };
        assert_eq!(prepare(&turn("anthropic", "m"), &e).unwrap().request.url, "https://gw.example.com/v1/messages");
        let e = Env { secret: &keys, first_name: None, anthropic_base: Some("http://gw.example.com") };
        assert!(prepare(&turn("anthropic", "m"), &e).is_err());
    }

    #[test]
    fn openai_uses_its_token_field_and_a_missing_key_says_so() {
        let p = prepare(&turn("openai", ""), &env()).unwrap();
        let body: Value = serde_json::from_str(p.request.body.as_ref().unwrap()).unwrap();
        assert_eq!(p.request.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["max_completion_tokens"], 4096);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][3], json!({ "role": "user", "content": "why?" }));
        assert!(p.request.headers.contains(&"Authorization: Bearer sk-oa-SECRET".to_string()));
        let err = prepare(&turn("google", "x"), &env()).err().unwrap();
        assert_eq!(err, "Google AI API key missing. Add it in Settings.");
        assert!(prepare(&turn("nope", "x"), &env()).is_err());
    }

    #[test]
    fn model_servers_stream_locally_and_the_custom_key_only_goes_where_it_was_bound() {
        let p = prepare(&turn("ollama", "llama3.2"), &env()).unwrap();
        assert!(p.request.url.ends_with("/v1/chat/completions"));
        assert!(p.request.allow_http && p.request.no_proxy);
        assert!(p.request.headers.contains(&"Authorization: Bearer ollama".to_string()));
        assert_eq!(prepare(&turn("ollama", ""), &env()).err().unwrap(), "Pick a model above the chat box first.");

        let mut t = turn("custom", "m");
        t.base_url = Some("https://llm.lan/v1".into());
        assert!(prepare(&t, &env()).unwrap().request.headers.contains(&"Authorization: Bearer sk-c-SECRET".to_string()));
        t.base_url = Some("https://other.lan".into());
        assert!(prepare(&t, &env()).unwrap().request.headers.contains(&"Authorization: Bearer ollama".to_string()));
        t.base_url = None;
        assert!(prepare(&t, &env()).is_err());
        assert!(bind_custom_key("http://10.0.0.2:8000", "k").is_err());
        assert_eq!(bind_custom_key("localhost:8000/v1", "k").unwrap(), r#"{"key":"k","url":"http://127.0.0.1:8000"}"#);
    }

    #[test]
    fn files_follow_the_original_rules_per_provider() {
        let dir = std::env::temp_dir().join(format!("coucou-chat-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "é".repeat(30_000)).unwrap();
        let png = dir.join("pic.png");
        std::fs::write(&png, [1u8, 2, 3]).unwrap();
        let pdf = dir.join("doc.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        let s = |p: &std::path::Path| p.to_str().unwrap().to_string();

        // Model servers: text inline, cut at 24 000 characters; images/PDF by name.
        let note = local_file_note("notes.txt", &s(&txt));
        assert!(note.starts_with("File: notes.txt\nFile contents:\n"));
        assert!(note.ends_with("\n[truncated]"));
        assert_eq!(note.matches('é').count(), 24_000);
        assert_eq!(local_file_note("pic.png", &s(&png)), "File: pic.png");
        assert_eq!(local_file_note("doc.pdf", &s(&pdf)), "File: doc.pdf");
        assert_eq!(local_file_note("dir", &s(&dir)), "File: dir");

        // Cloud: the image as a data URL; a PDF by name.
        let mut t = turn("openai", "m");
        t.file = Some(("pic.png".into(), s(&png)));
        let m = cloud_user_message(&t);
        assert_eq!(m["content"][0]["image_url"]["url"], "data:image/png;base64,AQID");
        assert_eq!(m["content"][1]["text"], "File: pic.png\n\nwhy?");
        t.file = Some(("doc.pdf".into(), s(&pdf)));
        assert_eq!(cloud_user_message(&t)["content"], "File: doc.pdf\n\nwhy?");

        // Claude: PDF as a document block.
        let mut t = turn("anthropic", "m");
        t.file = Some(("doc.pdf".into(), s(&pdf)));
        let body = anthropic_body(&t, "m", "sys");
        assert_eq!(body["messages"][2]["content"][0]["type"], "document");
        assert_eq!(body["messages"][2]["content"][1]["text"], "File: doc.pdf");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_text_deltas_are_taken_from_the_event_streams() {
        let o = |l: &str| parse_sse_line(Wire::OpenAi, l);
        assert_eq!(o(r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#), Some(Ok("Hel".into())));
        assert_eq!(o(r#"data:{"choices":[{"delta":{"role":"assistant"}}]}"#), None);
        assert_eq!(o("data: [DONE]"), None);
        assert_eq!(o(": OPENROUTER PROCESSING"), None);
        assert_eq!(o(r#"data: {"error":{"message":"boom"}}"#), Some(Err("boom".into())));
        let a = |l: &str| parse_sse_line(Wire::Anthropic, l);
        assert_eq!(a(r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#), Some(Ok("Hi".into())));
        assert_eq!(a(r#"data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}"#), None);
        assert_eq!(a("event: message_start"), None);
        assert_eq!(a(r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#), Some(Err("Overloaded".into())));
        assert_eq!(a(r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal"}}"#), Some(Err("Claude declined this one.".into())));
    }

    #[test]
    fn finished_thinking_blocks_go_and_an_open_one_hides_what_follows() {
        assert_eq!(filter_thinking_blocks("<think>hmm</think>\n\nThe answer."), "The answer.");
        assert_eq!(filter_thinking_blocks("a<think>x</think>b<think>y</think>c"), "abc");
        assert_eq!(filter_thinking_blocks("plain"), "plain");
        assert_eq!(progressive_filter("Sure. <think>let me think"), "Sure.");
        assert_eq!(progressive_filter("<think>still thinking"), "");
        assert_eq!(progressive_filter("<think>done</think>Visible <think>again"), "Visible");
    }

    #[test]
    fn model_lists_keep_chat_models_in_the_original_order() {
        let openai = json!({"data":[{"id":"gpt-4o","created":1},{"id":"text-embedding-3","created":9},{"id":"gpt-5","created":5}]});
        let ids: Vec<_> = parse_models("openai", &openai).iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, vec!["gpt-5", "gpt-4o"]);
        let gem = json!({"data":[{"id":"models/gemini-2.0-flash"},{"id":"models/imagen-3"}]});
        assert_eq!(parse_models("google", &gem), vec![json!({"id":"gemini-2.0-flash","label":"gemini-2.0-flash"})]);
        let or = json!({"data":[{"id":"b/x","name":"B"},{"id":"a/y:free","name":"A"},{"id":"img","architecture":{"output_modalities":["image"]}}]});
        assert_eq!(parse_models("openrouter", &or)[0]["label"], "A (free)");
        assert_eq!(parse_models("openrouter", &or).len(), 2);
        let claude = json!({"data":[{"id":"claude-opus-5","display_name":"Claude Opus 5"}]});
        assert_eq!(parse_models("anthropic", &claude)[0]["label"], "Claude Opus 5");
        let local = json!({"data":[{"id":"llama3.2"},{"id":"nomic-embed-text"}]});
        assert_eq!(parse_models("ollama", &local), vec![json!({"id":"llama3.2","label":"llama3.2"})]);
    }

    #[test]
    fn model_list_requests_carry_the_key_in_a_header_only() {
        let (req, _) = models_request("anthropic", None, &env()).unwrap();
        assert_eq!(req.url, "https://api.anthropic.com/v1/models?limit=100");
        assert!(req.headers[0].starts_with("x-api-key: "));
        assert_eq!(models_request("google", None, &env()).err().unwrap(), "No API key — add it in Settings.");
        let (req, p) = models_request("lmstudio", None, &env()).unwrap();
        assert_eq!(req.url, "http://127.0.0.1:1234/v1/models");
        assert!(p.server.is_some());
    }

    #[test]
    fn errors_read_like_the_original() {
        let p = prepare(&turn("openai", "m"), &env()).unwrap();
        assert_eq!(status_error(&p, 401, "x"), "OpenAI rejected the API key (401). Check it in Settings.");
        let p = prepare(&turn("ollama", "qwen"), &env()).unwrap();
        assert_eq!(status_error(&p, 404, ""), "qwen isn't installed. Pick another model above the chat box.");
        assert_eq!(failure_message(&p, Failure::Transport("refused".into())), "Cannot reach http://127.0.0.1:11434. Is the server running?");
    }

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }
}
