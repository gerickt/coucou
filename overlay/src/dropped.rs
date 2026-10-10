//! Files dropped on Mochi: the text/uri-list of a Wayland drag and drop
//! (RFC 2483: one URI per line, CRLF, # comments), reduced to local paths.

use std::path::PathBuf;

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hex(*b.get(i + 1)?)?;
            let lo = hex(*b.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Local files of a uri-list, in order. Other schemes and remote hosts are
/// left out; so are paths with a NUL byte.
pub fn local_paths(list: &str) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut out = Vec::new();
    for line in list.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("file://") else { continue };
        let path = if rest.starts_with('/') {
            rest
        } else if let Some(p) = rest.strip_prefix("localhost") {
            p
        } else {
            continue;
        };
        if !path.starts_with('/') {
            continue;
        }
        let Some(bytes) = percent_decode(path) else { continue };
        if bytes.contains(&0) {
            continue;
        }
        out.push(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_local_files() {
        let list = "# from a file manager\r\nfile:///home/me/My%20Notes.md\r\nfile://localhost/tmp/a%C3%A9.txt\r\n";
        assert_eq!(local_paths(list), [PathBuf::from("/home/me/My Notes.md"), PathBuf::from("/tmp/aé.txt")]);
    }

    #[test]
    fn leaves_out_remote_and_odd_entries() {
        let list = "https://example.com/x\nfile://server/share/x\nfile:///bad%zz\nfile:///nul%00x\nfile:///ok\n";
        assert_eq!(local_paths(list), [PathBuf::from("/ok")]);
    }
}
