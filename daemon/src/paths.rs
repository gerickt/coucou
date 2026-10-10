//! Where the sockets live. Must match coucou-hook's `socket_path()` exactly.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` when the variable is missing.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", uid())))
}

/// The socket coucou-hook connects to.
pub fn relay_socket(dir: &Path) -> PathBuf {
    dir.join("coucou.sock")
}

/// The socket the plugin and the CLI talk to.
pub fn control_socket(dir: &Path) -> PathBuf {
    dir.join("coucou-ctl.sock")
}

/// A real directory (not a symlink), owned by us, no access for group or others.
pub fn is_private_dir(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir)
        .map(|m| m.file_type().is_dir() && m.uid() == uid() && m.mode() & 0o077 == 0)
        .unwrap_or(false)
}

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn only_a_private_directory_qualifies() {
        let dir = std::env::temp_dir().join(format!("coucou-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(is_private_dir(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_private_dir(&dir));
        assert!(!is_private_dir(&dir.join("missing")));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(relay_socket(Path::new("/r")), PathBuf::from("/r/coucou.sock"));
        assert_eq!(control_socket(Path::new("/r")), PathBuf::from("/r/coucou-ctl.sock"));
    }
}
