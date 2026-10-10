//! API keys in the Secret Service (GNOME Keyring, KWallet) through `secret-tool`.
//!
//! The items carry the attributes the original app's keyring crate (v3) wrote
//! (`service`, `username`, `target`, `application`), so a key saved by either
//! app is found by the other. A key only ever travels over secret-tool's stdin
//! and stdout: never on a command line, never on disk, never printed. The
//! plugin can only ask whether a key is there (`secret has`).

use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub const SERVICE: &str = "fr.louisraille.coucou";
const TARGET: &str = "default";
const APPLICATION: &str = "rust-keyring";
/// A key longer than this is not a key.
const MAX_SECRET: usize = 16 * 1024;

/// Every key Coucou may store (the original's secrets.rs list). Anything else is refused.
pub const KNOWN_KEYS: &[&str] = &[
    "anthropic-api-key",
    "openai-api-key",
    "google-api-key",
    "openrouter-api-key",
    "openai-compatible-key",
    "n8n-url",
    "n8n-api-key",
    "vercel-token",
    "github-token",
    "stripe-api-key",
    "resend-api-key",
    "notion-api-key",
    "calcom-api-key",
];

/// The stored key's name for a chat provider id, or a known key name as is.
pub fn key_name(name: &str) -> Option<&'static str> {
    let mapped = match name {
        "anthropic" => "anthropic-api-key",
        "openai" => "openai-api-key",
        "google" => "google-api-key",
        "openrouter" => "openrouter-api-key",
        "custom" => "openai-compatible-key",
        other => return KNOWN_KEYS.iter().copied().find(|k| *k == other),
    };
    Some(mapped)
}

/// The attributes keyring v3 searches on.
fn search_attributes(key: &str) -> Vec<String> {
    ["service", SERVICE, "username", key, "target", TARGET].iter().map(|s| s.to_string()).collect()
}

pub fn lookup_argv(key: &str) -> Vec<String> {
    let mut argv = vec!["secret-tool".to_string(), "lookup".to_string()];
    argv.extend(search_attributes(key));
    argv
}

pub fn store_argv(key: &str) -> Vec<String> {
    let mut argv = vec![
        "secret-tool".to_string(),
        "store".to_string(),
        format!("--label={key}@{SERVICE}:{TARGET} (Coucou)"),
    ];
    argv.extend(search_attributes(key));
    argv.push("application".into());
    argv.push(APPLICATION.into());
    argv
}

pub fn clear_argv(key: &str) -> Vec<String> {
    let mut argv = vec!["secret-tool".to_string(), "clear".to_string()];
    argv.extend(search_attributes(key));
    argv
}

/// A pasted key, cleaned: surrounding blanks go; a control character inside
/// (a newline would split an HTTP header) refuses it.
pub fn clean_secret(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("empty key".into());
    }
    if value.len() > MAX_SECRET {
        return Err("key too long".into());
    }
    if value.chars().any(char::is_control) {
        return Err("the key contains a control character".into());
    }
    Ok(value.to_string())
}

fn command(argv: &[String]) -> Command {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd
}

/// The stored value, or None when there is none (or no Secret Service).
pub fn get(key: &str) -> Option<String> {
    let out = command(&lookup_argv(key)).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?;
    let value = value.trim_end_matches(['\n', '\r']).to_string();
    (!value.is_empty()).then_some(value)
}

pub fn has(key: &str) -> bool {
    get(key).is_some()
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    let mut child = command(&store_argv(key))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("secret-tool: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("secret-tool: no stdin")?;
        stdin.write_all(value.as_bytes()).map_err(|e| format!("secret-tool: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("secret-tool: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("secret-tool store failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

pub fn delete(key: &str) -> Result<(), String> {
    let out = command(&clear_argv(key))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("secret-tool: {e}"))?;
    // clear succeeds when nothing matched, too.
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("secret-tool clear failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// `coucou-daemon secret has|set|delete <provider|key> [--bound-url <url>]`.
/// `has` prints `{"key":…,"present":bool}`; `set` reads the key from stdin.
/// `--bound-url` (custom server only) stores the key with the one address it
/// may be sent to, as the original's Local models section did.
pub fn run(args: &[String], stdin: &mut dyn Read, out: &mut dyn Write) -> Result<(), String> {
    let (verb, name) = match args {
        [verb, name, ..] => (verb.as_str(), name.as_str()),
        _ => return Err("usage: coucou-daemon secret has|set|delete <provider|key>".into()),
    };
    let key = key_name(name).ok_or_else(|| format!("unknown key {name}"))?;
    let bound = match &args[2..] {
        [] => None,
        [flag, url] if flag == "--bound-url" && verb == "set" && key == "openai-compatible-key" => Some(url.as_str()),
        _ => return Err("unexpected arguments".into()),
    };
    match verb {
        "has" => {
            let line = serde_json::json!({ "key": key, "present": has(key) });
            writeln!(out, "{line}").map_err(|e| e.to_string())
        }
        "set" => {
            let mut raw = String::new();
            stdin.take(MAX_SECRET as u64 + 1).read_to_string(&mut raw).map_err(|e| e.to_string())?;
            let value = clean_secret(&raw)?;
            let value = match bound {
                Some(url) => crate::chat::bind_custom_key(url, &value)?,
                None => value,
            };
            set(key, &value)
        }
        "delete" => delete(key),
        other => Err(format!("unknown secret command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_map_to_the_original_key_names_and_others_are_refused() {
        assert_eq!(key_name("anthropic"), Some("anthropic-api-key"));
        assert_eq!(key_name("google"), Some("google-api-key"));
        assert_eq!(key_name("custom"), Some("openai-compatible-key"));
        assert_eq!(key_name("github-token"), Some("github-token"));
        assert_eq!(key_name("ollama"), None);
        assert_eq!(key_name("../etc"), None);
    }

    #[test]
    fn the_attributes_match_keyring_v3_and_no_value_is_on_a_command_line() {
        let store = store_argv("openai-api-key");
        assert_eq!(store[..2], ["secret-tool", "store"]);
        let pairs: Vec<_> = store[3..].chunks(2).map(|c| (c[0].as_str(), c[1].as_str())).collect();
        assert_eq!(
            pairs,
            vec![
                ("service", SERVICE),
                ("username", "openai-api-key"),
                ("target", "default"),
                ("application", "rust-keyring"),
            ]
        );
        assert_eq!(lookup_argv("x")[2..], ["service", SERVICE, "username", "x", "target", "default"]);
        assert_eq!(clear_argv("x")[1], "clear");
    }

    #[test]
    fn a_pasted_key_is_trimmed_and_one_with_a_newline_inside_is_refused() {
        assert_eq!(clean_secret("  sk-abc\n").unwrap(), "sk-abc");
        assert!(clean_secret("sk-a\nX-Evil: 1").is_err());
        assert!(clean_secret("   ").is_err());
    }

    /// Against the real Secret Service, with a throwaway entry:
    /// `cargo test -- --ignored live_secret_round_trip`.
    #[test]
    #[ignore]
    fn live_secret_round_trip() {
        let key = "coucou-test-roundtrip";
        let _ = delete(key);
        assert!(!has(key));
        set(key, "test-value-123").unwrap();
        assert!(has(key));
        assert_eq!(get(key).as_deref(), Some("test-value-123"));
        delete(key).unwrap();
        assert!(!has(key));
    }

    #[test]
    fn the_cli_refuses_unknown_keys_and_stray_arguments() {
        let mut out = Vec::new();
        assert!(run(&["has".into(), "nope".into()], &mut std::io::empty(), &mut out).is_err());
        assert!(run(&["has".into(), "openai".into(), "--bound-url".into(), "x".into()], &mut std::io::empty(), &mut out).is_err());
        assert!(run(&["set".into(), "openai".into()], &mut "  ".as_bytes(), &mut out).is_err());
        assert!(out.is_empty());
    }
}
