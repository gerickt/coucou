# Coucou for Hyprland + Noctalia

Mochi is a small animated character that floats above your windows and lives in your [Noctalia](https://github.com/noctalia-dev/noctalia-shell) bar. Mochi shows what your AI coding agents (Claude Code, Codex, Copilot CLI, Muse Code, Gemini CLI, Antigravity, Cursor, OpenCode, Amp, Hermes) are doing: thinking, working, searching, finished, or waiting for you. When an agent asks for a permission or asks a question, you approve, deny or answer with a click.

This is a Linux-only fork of [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou). The macOS, iPhone and Windows apps were removed.

## How it works

```
agent hook ──> coucou-hook ──> $XDG_RUNTIME_DIR/coucou.sock ──> coucou-daemon
                                                                     │
         Noctalia plugin coucou/mochi <── coucou-daemon --follow ────┘
         (bar, panels, service)       ──> coucou-daemon decide <id> allow|deny|answers
                │
                └─ $XDG_RUNTIME_DIR/coucou-mochi.json ──> coucou-mochi (Mochi above the windows)
```

- `hook/` — `coucou-hook`, the relay each agent runs on every hook event. It forwards one JSON line to the daemon and exits at once if the daemon is not running, so an agent is never blocked.
- `daemon/` — `coucou-daemon`, a small Rust daemon. It holds a permission request only if the plugin shows it within 800 ms, then waits up to 108 s for a click. No click means no answer, and the agent asks in its terminal as usual.
- `mochi/` — the Noctalia plugin `coucou/mochi`:
  - **desktop widget**: the original animated Mochi on Noctalia's desktop layer, under the windows. With the default `Where Mochi lives` setting (*Above the windows*) the overlay shows him instead and this widget stays empty; choose *Desktop widget* or *Both* to use it. He greets you, then loops through each agent state. Click him and he gets annoyed. Click three times fast and he gets dizzy. Hover him for 2 s and he falls in love. When a request is waiting, a click opens the approval panel.
  - **bar widget**: a mini Mochi and the running tool. Left click opens the panel, right click pauses or resumes Mochi.
  - **outfit setting**: Auto (follows the seasons: party hat, Santa hat, witch hat, bunny ears, sunglasses), None, or one of the 11 wardrobe outfits.
  - **panel**: Allow / Deny for permission requests, and the options of Claude Code's questions.
  - **chat**: Mochi's chat with Anthropic, Google AI, OpenAI, OpenRouter, Ollama, LM Studio or any OpenAI-compatible server. Answers stream in as Markdown.
  - **service**: follows the daemon, keeps Mochi's state, runs the chat and plays his sounds. It starts the overlay and tells it Mochi's state.
- `overlay/` — `coucou-mochi`, Mochi above your windows (see [Mochi above your windows](#mochi-above-your-windows)).

## Install

### 1. Binaries

Arch Linux:

```
cd linux/arch && makepkg -si
```

Manual:

```
cargo build --release --manifest-path daemon/Cargo.toml
cargo build --release --manifest-path hook/Cargo.toml
cargo build --release --manifest-path overlay/Cargo.toml
install -Dm755 daemon/target/release/coucou-daemon ~/.local/bin/coucou-daemon
install -Dm755 hook/target/release/coucou-hook ~/.local/bin/coucou-hook
install -Dm755 overlay/target/release/coucou-mochi ~/.local/bin/coucou-mochi
```

`coucou-daemon`, `coucou-hook` and `coucou-mochi` must be in the `PATH` Noctalia starts with.

### 2. Run the daemon at login

With systemd (the package installs the unit; for a manual install copy `daemon/coucou-daemon.service` to `~/.config/systemd/user/` and fix `ExecStart`):

```
systemctl --user enable --now coucou-daemon
```

Or in `hyprland.conf`:

```
exec-once = coucou-daemon
```

### 3. Add the plugin to Noctalia

From the Arch package (a local plugin source):

```
noctalia msg plugins source add coucou path /usr/share/coucou/noctalia
noctalia msg plugins enable coucou/mochi
```

From this repository (git plugin source):

```
noctalia msg plugins source add coucou git https://github.com/gerickt/coucou
noctalia msg plugins enable coucou/mochi
```

Then place the Mochi bar widget from Noctalia's settings. Mochi himself appears above your windows on their own (the plugin starts `coucou-mochi`); the Mochi desktop widget is only needed with the *Desktop widget* or *Both* setting.

### Mochi above your windows

Noctalia's desktop widgets sit under every window, so Mochi floats on a layer-shell surface of his own, `coucou-mochi`, like the original Linux app. The plugin starts it when **Where Mochi lives** is *Above the windows* (the default) or *Both*, and stops it for *Desktop widget*.

- He sits in the bottom-right corner of the monitor you work on and follows Hyprland's focused monitor. Clicks anywhere but on his body go to the window underneath.
- **Drag** him anywhere; each monitor remembers his spot (`~/.local/state/coucou/overlay.json`). **Double-click** flies him home to the corner.
- **Click** him: annoyed; three clicks within 1.7 s: dizzy. **Hover** him 2 s: hearts. While a request waits, a click opens the approval panel. **Right-click** opens the plugin's settings (outfit, sounds, overlay).
- His **eyes follow the pointer** while he is awake (Hyprland's cursor position, 8 times a second, only then). He falls asleep after 120 s without agent activity once the pointer is away; the pointer over him wakes him.
- **Drop a file** on him: he swallows it and the chat opens about it.
- **Peek** (setting, off by default): dropped against the top edge, he hides there and peeks out when the pointer comes; with **Open on hover** that also opens the approval panel.
- **Layer**: *Top* (default, the original's) keeps him above windows and under fullscreen ones, so a fullscreen video hides him; *Overlay* keeps him above fullscreen windows too. The lock screen always covers him.
- Nothing is drawn unless his picture changes, and nothing animates while he is hidden (a fullscreen window, screens off).

`coucou-mochi --status` exits 0 while it runs, `coucou-mochi --quit` stops it. To run it without the plugin managing it: `systemctl --user enable --now coucou-mochi` (the package installs the unit; the user manager needs `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE`). Without Hyprland he stays on the first monitor and his eyes follow the pointer only over him.

### 4. Connect your agents

Open the Agents panel: the plug button that shows when you hover the desktop Mochi, **Agents…** in the bar panel, or `noctalia msg plugin coucou/mochi:daemon all agents`. It lists every agent with whether it is installed on this machine and whether it is connected to Mochi. **Connect…** and **Disconnect…** first show the exact change and where the backup goes; nothing is written until you click **Back up and connect** (or **Back up and disconnect**).

The same from a terminal:

```
coucou-daemon install-hooks                      # Claude Code (~/.claude/settings.json)
coucou-daemon install-hooks --agent codex        # Codex (~/.codex/hooks.json)
coucou-daemon install-hooks --agent copilot      # GitHub Copilot CLI (~/.copilot/hooks/coucou.json)
coucou-daemon install-hooks --agent muse         # Muse Code (~/.config/muse/settings.json)
coucou-daemon install-hooks --agent gemini       # Gemini CLI (~/.gemini/settings.json)
coucou-daemon install-hooks --agent antigravity  # Antigravity (~/.gemini/config/hooks.json)
coucou-daemon install-hooks --agent cursor       # Cursor Agent (~/.cursor/hooks.json)
coucou-daemon install-hooks --agent opencode     # OpenCode plugin (~/.config/opencode/plugins/coucou.js)
coucou-daemon install-hooks --agent amp          # Amp plugin (~/.config/amp/plugins/coucou.ts)
coucou-daemon install-hooks --agent hermes       # Hermes plugin (~/.hermes/plugins/coucou/)
coucou-daemon install-hooks --status [--json]    # installed / connected, per agent
coucou-daemon install-hooks --dry-run --json --agent cursor  # the diff and backup path, nothing written
```

Each run shows the diff and asks before writing. It keeps a dated backup (`settings.json.coucou-backup-YYYYMMDD-HHMMSS`) and leaves every other hook untouched. `--yes` skips the question, and `--remove` takes Coucou's hooks out again. `--dry-run --json` also gives each file's `sha256` (`absent` for a file that does not exist yet); passing them back with `--expect-sha256 <path>=<sha256>` makes the write refuse a file that changed since that preview (the Agents panel does this). OpenCode, Amp and Hermes get a plugin file of Coucou's own; a file of the same name that Coucou did not write is never touched. Hermes needs `hermes plugins enable coucou` once.

Approvals and questions work for Claude Code and Codex, and Allow / Deny for Copilot CLI and Muse Code. The other agents show their activity, and their prompts stay in their own terminal.

### 5. Chat

Open the chat from the speech-bubble button that shows when you hover the desktop Mochi, **Chat…** in the bar panel, the **Mochi chat** quick-settings tile, or `noctalia msg panel-toggle coucou/mochi:chat`. The original's Ctrl+Alt+Space, in `hyprland.conf`:

```
bind = CTRL ALT, Space, exec, noctalia msg panel-toggle coucou/mochi:chat
```

- **Keys**: the key button lists the providers with *Key saved* or *No key*. A saved key goes to the system keyring (Secret Service: GNOME Keyring, KWallet) with the original app's attributes; the plugin only learns whether it is there. A custom server's key is stored with its address and only ever sent there (over https, or to this computer).
- **Model**: click the model name above the conversation to switch provider and pick a model from the provider's own list.
- **Answers** stream in. Reasoning models' `<think>` blocks stay hidden. **Copy** and **Code n** copy an answer or one of its code blocks. Stop ends a turn and keeps what came so far. Mochi thinks while he answers.
- **Files**: copy a file in your file manager (Ctrl+C), then click the clip button. Mochi swallows it and the next question is about it: text inline (up to 24 000 characters for the model servers), images and PDF by name for the model servers, as data for Claude and the cloud providers. Noctalia's drop zones only take drags from inside the shell, so a file cannot be dropped on the panel.
- `COUCOU_ANTHROPIC_BASE_URL` points Claude at an Anthropic-compatible gateway (https only; plain http only to this computer), as in the original.

The requests leave from `coucou-daemon chat`, which hands `curl` the address, headers and body on its stdin: no key is ever on a command line or on disk. Nothing is sent anywhere until you ask a question or open the model list.

## Commands

| Command | What it does |
| --- | --- |
| `coucou-daemon` | Serves the relay and control sockets |
| `coucou-daemon --status-json` | Prints the current state (`{"running":false}` when stopped) |
| `coucou-daemon --follow` | Streams events as JSON lines (used by the plugin) |
| `coucou-daemon decide <id> allow\|deny` | Answers a pending request |
| `noctalia msg plugin coucou/mochi:daemon all toggle-pause` | Pauses or resumes Mochi |
| `noctalia msg plugin coucou/mochi:daemon all agents` | Opens the Agents panel |
| `noctalia msg panel-toggle coucou/mochi:chat` | Opens or closes the chat |
| `noctalia msg plugin coucou/mochi:daemon all attach <path>` | A file for a new chat (what a drop on Mochi does) |
| `coucou-mochi [--plugin-dir <dir>]` | Mochi above the windows (the plugin runs it with `--managed`) |
| `coucou-mochi --status` / `--quit` | Is it running / stop it |
| `coucou-daemon secret has\|set\|delete <provider>` | A key's presence, or saves (from stdin) / deletes it in the keyring |
| `coucou-daemon chat --request <file>` | One chat turn, streamed as JSON lines (used by the plugin) |
| `coucou-daemon chat-models <provider> [--base-url <url>]` | A provider's chat models as JSON |

## Development

```
(cd hook && cargo test) && (cd daemon && cargo test) && (cd overlay && cargo test)
noctalia plugins lint mochi
LUAU=/path/to/luau mochi/tests/run.sh   # plugin specs against a mocked Noctalia host
```

### Mochi's frames

Noctalia plugins have no canvas, so the widgets play PNG frames. `mochi/frames/` is generated from the original Mochi engine: `tools/render-mochi/vendor/` holds `engine.ts`, `outfits.ts`, `anim.ts` and `fonts.ts` exactly as they were at commit 081a2ad. `render.sh` runs that engine in headless Chrome on a virtual clock, calls its own API (`setState`, `slap`, `greet`, `triggerEmote`…) and saves every frame. Then it gives each sequence one palette, removes duplicate frames and writes `mochi/frames/manifest.luau` (the widgets) and `mochi/frames/manifest.json` (the overlay). The idle and working loops also come in eight gaze directions (`<outfit>.idle.look.ne`…), the engine's own `lookX`/`lookY`, for the overlay's eyes.

```
tools/render-mochi/render.sh                       # regenerate mochi/frames (about 5 min)
tools/render-mochi/render.sh --compare sheet.png   # also: shipped frame next to a fresh engine frame
```

It needs Node 23 or later, Chrome or Chromium (`CHROME=/path`), and Python 3 with Pillow and numpy. Pillow must be built with libimagequant. The same inputs give the same files.

## License

MIT for the code, see `LICENSE`. The names Coucou and Mochi, the Mochi character and the sounds are not covered by the MIT license; see `LICENSE-ASSETS.md`.
