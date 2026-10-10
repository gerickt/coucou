//! The interaction sounds the desktop widget plays (slap, annoyed, love,
//! dizzy, hover, pop, greeting, and peek when he flies home or peeks out),
//! played here when the overlay shows him. The
//! service keeps the agent sounds (approval, finish…), so nothing plays twice.
//! pw-play (PipeWire), else paplay; neither installed: silence.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Only these names, so a path is never built from outside input.
const KNOWN: [&str; 8] = ["slap", "annoyed", "love", "dizzy", "hover", "pop", "greeting", "peek"];

pub struct Sounds {
    dir: PathBuf,
    player: Option<&'static str>,
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

impl Sounds {
    pub fn new(plugin_dir: &Path) -> Self {
        let player = ["pw-play", "paplay"].into_iter().find(|c| on_path(c));
        Sounds { dir: plugin_dir.join("sounds"), player }
    }

    pub fn play(&self, name: &str) {
        let Some(player) = self.player else { return };
        if !KNOWN.contains(&name) {
            return;
        }
        let file = self.dir.join(format!("{name}.wav"));
        if !file.is_file() {
            return;
        }
        let child = Command::new(player)
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        // Reaped off the event loop; a sound lasts a second or two.
        if let Ok(mut child) = child {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
