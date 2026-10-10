//! HTTP for the chat, through `curl` (no TLS crate in the daemon).
//!
//! Everything about a request — address, headers with the API key, body — goes
//! to curl as a config file on its stdin (`--config -`), so nothing secret or
//! private ever sits on a command line (`/proc/<pid>/cmdline` is world
//! readable) or on disk. Redirects are not followed, so a key is only ever sent
//! to the address it was meant for, and curl dies with the daemon.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::process::{Child, Command, Stdio};

/// Ceiling for a whole body that is read at once (model lists).
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Ceiling for an error body: only its message is shown.
pub const MAX_ERROR_BODY: usize = 64 * 1024;
/// curl refuses config lines past 10 MiB; the body is one of them.
const MAX_REQUEST_BODY: usize = 9 * 1024 * 1024;

/// The fixed command line. Nothing request-specific is ever added to it.
pub const CURL_ARGV: &[&str] = &["curl", "--silent", "--show-error", "--no-buffer", "--include", "--config", "-"];

// ── Addresses ────────────────────────────────────────────────────────────────

/// Just enough of a URL for the chat: scheme, host, port and path. A user
/// name or password in it is refused; query and fragment are dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
}

impl Url {
    pub fn parse(raw: &str) -> Result<Url, String> {
        let raw = raw.trim();
        let (scheme, rest) = raw.split_once("://").ok_or_else(|| format!("Not a valid address: {raw}"))?;
        let scheme = scheme.to_ascii_lowercase();
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if authority.contains('@') {
            return Err("Leave the user name and password out of the address.".into());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (host, tail) = v6.split_once(']').ok_or_else(|| format!("Not a valid address: {raw}"))?;
            (format!("[{host}]"), tail.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        let port = match port {
            Some(p) => Some(p.parse::<u16>().map_err(|_| format!("Not a valid address: {raw}"))?),
            None => None,
        };
        if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("Not a valid address: {raw}"));
        }
        Ok(Url { scheme, host: host.to_ascii_lowercase(), port, path: path.to_string() })
    }

    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    /// Only the host and port, for messages and logs: never a path or a key.
    pub fn origin(&self) -> String {
        match self.port {
            Some(p) => format!("{}://{}:{p}", self.scheme, self.host),
            None => format!("{}://{}", self.scheme, self.host),
        }
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.origin(), self.path)
    }
}

/// This machine: `localhost`, 127.0.0.0/8, ::1, and the unspecified addresses.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unspecified() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        Err(_) => false,
    }
}

/// A model server's address as typed: a missing scheme becomes http, the
/// documentation sub-paths people paste (`/api`, `/v1`) and trailing slashes
/// go, the loopback names become 127.0.0.1 (Ollama only listens on IPv4).
pub fn normalise_server_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter the server address first.".into());
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut url = Url::parse(&with_scheme)?;
    if !matches!(url.scheme.as_str(), "http" | "https") {
        return Err("The address must start with http:// or https://.".into());
    }
    if url.is_loopback() {
        url.host = "127.0.0.1".into();
    }
    let mut path = url.path.trim_end_matches('/').to_string();
    for suffix in ["/api", "/v1"] {
        if let Some(rest) = path.strip_suffix(suffix) {
            path = rest.trim_end_matches('/').to_string();
        }
    }
    url.path = path;
    Ok(url)
}

/// `base` + `tail`, with exactly one slash between them.
pub fn join(base: &Url, tail: &str) -> String {
    format!("{}/{}", base.to_string().trim_end_matches('/'), tail.trim_start_matches('/'))
}

/// The Messages endpoint of a gateway named in COUCOU_ANTHROPIC_BASE_URL. The
/// key goes wherever this points, so https only (plain http to this machine).
pub fn anthropic_endpoint(raw: &str) -> Result<Url, String> {
    const VAR: &str = "COUCOU_ANTHROPIC_BASE_URL";
    let mut url = Url::parse(raw).map_err(|_| format!("{VAR} is not a valid URL."))?;
    match url.scheme.as_str() {
        "https" => {}
        "http" if url.is_loopback() => {}
        "http" => return Err(format!("{VAR} must use https:// (plain http only to this computer).")),
        _ => return Err(format!("{VAR} must start with https://.")),
    }
    let path = url.path.trim_end_matches('/').to_string();
    url.path = if path.ends_with("/v1/messages") {
        path
    } else if path.ends_with("/v1") {
        format!("{path}/messages")
    } else {
        format!("{path}/v1/messages")
    };
    Ok(url)
}

/// The text of an error body: `error.message` (OpenAI, Anthropic), a bare
/// `error` string (Ollama), `message`, Gemini's one-element array, or the
/// first characters.
pub fn error_detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        for ptr in ["/error/message", "/error", "/message", "/0/error/message"] {
            if let Some(m) = v.pointer(ptr).and_then(|m| m.as_str()) {
                return m.to_string();
            }
        }
    }
    text.trim().chars().take(200).collect()
}

// ── Requests ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    /// Full `Name: value` lines.
    pub headers: Vec<String>,
    /// POST with this body; GET without one.
    pub body: Option<String>,
    pub max_time_secs: u32,
    /// Plain http allowed (a model server); https only otherwise.
    pub allow_http: bool,
    /// Skip any system proxy: it would not reach this machine's loopback.
    pub no_proxy: bool,
}

/// A value inside double quotes, as curl's config parser reads it back.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The config curl reads on its stdin.
pub fn config(req: &Request) -> Result<String, String> {
    let mut lines = vec![
        format!("url = {}", quote(&req.url)),
        format!("proto = {}", quote(if req.allow_http { "=http,https" } else { "=https" })),
        "connect-timeout = 10".to_string(),
        format!("max-time = {}", req.max_time_secs),
    ];
    if req.no_proxy {
        lines.push("noproxy = \"*\"".into());
    }
    for h in &req.headers {
        if h.chars().any(|c| c == '\n' || c == '\r') {
            return Err("bad header".into());
        }
        lines.push(format!("header = {}", quote(h)));
    }
    if let Some(body) = &req.body {
        if body.len() > MAX_REQUEST_BODY {
            return Err("The question is too large to send.".into());
        }
        // A body starting with '@' would name a file to curl; ours is JSON.
        if !body.starts_with('{') {
            return Err("bad body".into());
        }
        lines.push(format!("data-binary = {}", quote(body)));
    }
    Ok(lines.join("\n") + "\n")
}

/// Starts curl, hands it the config, and leaves its stdout to read.
pub fn spawn(req: &Request) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;
    let cfg = config(req).map_err(io::Error::other)?;
    let mut cmd = Command::new(CURL_ARGV[0]);
    cmd.args(&CURL_ARGV[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // curl goes when the daemon goes (a Stop kills the daemon's process).
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?;
    stdin.write_all(cfg.as_bytes())?;
    drop(stdin);
    Ok(child)
}

/// Splits `--include` output into the status and the body lines. Interim
/// (1xx) heads and a proxy's "Connection established" head are skipped.
#[derive(Default)]
pub struct Head {
    pub status: Option<u16>,
    in_body: bool,
    body_seen: bool,
}

impl Head {
    /// The body line, if `line` (without its line ending) is one.
    pub fn feed<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        if self.in_body {
            if !self.body_seen && line.starts_with("HTTP/") {
                self.in_body = false;
            } else {
                self.body_seen = true;
                return Some(line);
            }
        }
        if line.starts_with("HTTP/") {
            self.status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        } else if line.is_empty() && !self.status.is_some_and(|s| (100..200).contains(&s)) {
            self.in_body = true;
        }
        None
    }
}

/// What curl said on stderr when it failed.
fn stderr_text(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.by_ref().take(4096).read_to_string(&mut text);
    }
    text.trim().trim_start_matches("curl: ").to_string()
}

pub enum Failure {
    /// No HTTP answer at all: curl's own message.
    Transport(String),
    /// An answer with a non-2xx status, and its (capped) body.
    Status(u16, Vec<u8>),
    /// The answer went past a ceiling, or the line handler gave up.
    Other(String),
}

/// Runs `req`, handing each body line of a 2xx answer to `on_line`.
pub fn stream(req: &Request, mut on_line: impl FnMut(&str) -> Result<(), String>) -> Result<(), Failure> {
    let mut child = spawn(req).map_err(|e| Failure::Transport(e.to_string()))?;
    let stdout = child.stdout.take().ok_or_else(|| Failure::Transport("no stdout".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut head = Head::default();
    let mut error_body: Vec<u8> = Vec::new();
    let mut buf = Vec::new();
    let result = loop {
        buf.clear();
        match reader.by_ref().take(1024 * 1024 + 1).read_until(b'\n', &mut buf) {
            Ok(0) => break Ok(()),
            Ok(_) => {}
            Err(e) => break Err(Failure::Transport(e.to_string())),
        }
        if buf.len() > 1024 * 1024 {
            break Err(Failure::Other("The server's answer is too large.".into()));
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\n', '\r']);
        let Some(body) = head.feed(line) else { continue };
        match head.status {
            Some(s) if (200..300).contains(&s) => {
                if let Err(e) = on_line(body) {
                    break Err(Failure::Other(e));
                }
            }
            _ => {
                if error_body.len() < MAX_ERROR_BODY {
                    error_body.extend_from_slice(body.as_bytes());
                    error_body.push(b'\n');
                }
            }
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    result?;
    let ok = status.as_ref().map(|s| s.success()).unwrap_or(false);
    match head.status {
        // Cut off midway (a timeout, a dropped connection).
        Some(s) if (200..300).contains(&s) && !ok => Err(Failure::Transport(stderr_text(&mut child))),
        Some(s) if (200..300).contains(&s) => Ok(()),
        Some(s) => Err(Failure::Status(s, error_body)),
        None => {
            let msg = stderr_text(&mut child);
            Err(Failure::Transport(if msg.is_empty() || ok { "no answer".into() } else { msg }))
        }
    }
}

/// Runs `req` and returns the whole 2xx body (capped).
pub fn fetch(req: &Request, limit: usize) -> Result<Vec<u8>, Failure> {
    let mut body = Vec::new();
    stream(req, |line| {
        if body.len() + line.len() > limit {
            return Err("The server's answer is too large.".into());
        }
        body.extend_from_slice(line.as_bytes());
        body.push(b'\n');
        Ok(())
    })?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse_and_refuse_credentials() {
        let u = Url::parse("https://GW.example.com:8443/base/?x=1#f").unwrap();
        assert_eq!((u.scheme.as_str(), u.host.as_str(), u.port, u.path.as_str()), ("https", "gw.example.com", Some(8443), "/base/"));
        assert_eq!(Url::parse("http://[::1]:11434").unwrap().host, "[::1]");
        assert!(Url::parse("https://user:pw@x.com").is_err());
        assert!(Url::parse("x.com").is_err());
        assert!(Url::parse("http://x.com:notaport").is_err());
    }

    #[test]
    fn server_addresses_are_cleaned_like_the_original() {
        assert_eq!(normalise_server_url("localhost:11434/api/").unwrap().to_string(), "http://127.0.0.1:11434");
        assert_eq!(normalise_server_url("https://llm.lan/v1").unwrap().to_string(), "https://llm.lan");
        assert!(normalise_server_url("ftp://x").is_err());
        assert!(normalise_server_url("  ").is_err());
        assert!(is_loopback_host("[::1]") && is_loopback_host("127.4.5.6") && !is_loopback_host("10.0.0.1"));
    }

    #[test]
    fn the_anthropic_gateway_must_be_https() {
        assert_eq!(anthropic_endpoint("https://gw.example.com").unwrap().to_string(), "https://gw.example.com/v1/messages");
        assert_eq!(anthropic_endpoint("https://gw.example.com/v1/").unwrap().to_string(), "https://gw.example.com/v1/messages");
        assert_eq!(anthropic_endpoint("https://gw/x/v1/messages").unwrap().path, "/x/v1/messages");
        assert!(anthropic_endpoint("http://gw.example.com").is_err());
        assert!(anthropic_endpoint("http://127.0.0.1:8080").is_ok());
        assert!(anthropic_endpoint("https://k:s@gw.example.com").is_err());
    }

    #[test]
    fn the_config_quotes_everything_and_the_argv_never_changes() {
        let req = Request {
            url: "https://api.x/v1".into(),
            headers: vec!["Authorization: Bearer sk-\"q\\".into()],
            body: Some(r#"{"a":"line\nnext"}"#.into()),
            max_time_secs: 300,
            allow_http: false,
            no_proxy: false,
        };
        let cfg = config(&req).unwrap();
        assert!(cfg.contains(r#"header = "Authorization: Bearer sk-\"q\\""#), "{cfg}");
        assert!(cfg.contains(r#"data-binary = "{\"a\":\"line\\nnext\"}""#), "{cfg}");
        assert!(cfg.contains("proto = \"=https\""));
        assert!(!CURL_ARGV.iter().any(|a| a.contains("sk-")));
        let mut bad = req.clone();
        bad.headers = vec!["X: a\nY: b".into()];
        assert!(config(&bad).is_err());
        bad.headers.clear();
        bad.body = Some("@/etc/passwd".into());
        assert!(config(&bad).is_err());
    }

    #[test]
    fn the_head_is_split_from_the_body_past_interim_and_proxy_heads() {
        let mut h = Head::default();
        let lines = [
            "HTTP/1.1 200 Connection established", "", "HTTP/2 100", "", "HTTP/2 200", "content-type: x", "",
            "data: a", "", "data: b",
        ];
        let body: Vec<&str> = lines.iter().filter_map(|l| h.feed(l)).collect();
        assert_eq!(body, vec!["data: a", "", "data: b"]);
        assert_eq!(h.status, Some(200));
    }

    #[test]
    fn error_bodies_give_their_message() {
        assert_eq!(error_detail(br#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(error_detail(br#"{"error":"model not found"}"#), "model not found");
        assert_eq!(error_detail(br#"[{"error":{"message":"gem"}}]"#), "gem");
        assert_eq!(error_detail(b"  plain  "), "plain");
    }
}
