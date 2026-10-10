# Coucou for Hyprland + Noctalia

Mochi is a small animated character on your [Noctalia](https://github.com/noctalia-dev/noctalia-shell) desktop and bar. Mochi shows what your AI coding agents (Claude Code, Gemini CLI, Antigravity, Codex) are doing: thinking, working, searching, finished, or waiting for you. When an agent asks for a permission or asks a question, you approve, deny or answer with a click.

This is a Linux-only fork of [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou). The macOS, iPhone and Windows apps were removed.

## How it works

```
agent hook ──> coucou-hook ──> $XDG_RUNTIME_DIR/coucou.sock ──> coucou-daemon
                                                                     │
         Noctalia plugin coucou/mochi <── coucou-daemon --follow ────┘
         (desktop Mochi, bar, panel)  ──> coucou-daemon decide <id> allow|deny|answers
```

- `hook/` — `coucou-hook`, the relay each agent runs on every hook event. It forwards one JSON line to the daemon and exits at once if the daemon is not running, so an agent is never blocked.
- `daemon/` — `coucou-daemon`, a small Rust daemon. It holds a permission request only if the plugin shows it within 800 ms, then waits up to 108 s for a click. No click means no answer, and the agent asks in its terminal as usual.
- `noctalia/` — the Noctalia plugin `coucou/mochi`:
  - **desktop widget**: the animated Mochi. Click him and he gets annoyed. Click three times fast and he gets dizzy. Hover him for 2 s and he falls in love. When a request is waiting, a click opens the approval panel.
  - **bar widget**: a mini Mochi and the running tool. Left click opens the panel, right click pauses or resumes Mochi.
  - **panel**: Allow / Deny for permission requests, and the options of Claude Code's questions.
  - **service**: follows the daemon, keeps Mochi's state and plays his sounds.

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
install -Dm755 daemon/target/release/coucou-daemon ~/.local/bin/coucou-daemon
install -Dm755 hook/target/release/coucou-hook ~/.local/bin/coucou-hook
```

`coucou-daemon` and `coucou-hook` must be in the `PATH` Noctalia starts with.

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

Then place the Mochi bar widget and the Mochi desktop widget from Noctalia's settings.

### 4. Install the hooks

```
coucou-daemon install-hooks                      # Claude Code (~/.claude/settings.json)
coucou-daemon install-hooks --agent gemini       # Gemini CLI (~/.gemini/settings.json)
coucou-daemon install-hooks --agent antigravity  # Antigravity (~/.gemini/config/hooks.json)
coucou-daemon install-hooks --agent codex        # Codex (~/.codex/hooks.json)
```

Each run shows the diff and asks before writing. It keeps a dated backup (`settings.json.coucou-backup-YYYYMMDD-HHMMSS`) and leaves every other hook untouched. `--yes` skips the question, and `--remove` takes Coucou's hooks out again.

Approvals and questions work for Claude Code and Codex. Gemini CLI and Antigravity show their activity, and their prompts stay in their own terminal.

## Commands

| Command | What it does |
| --- | --- |
| `coucou-daemon` | Serves the relay and control sockets |
| `coucou-daemon --status-json` | Prints the current state (`{"running":false}` when stopped) |
| `coucou-daemon --follow` | Streams events as JSON lines (used by the plugin) |
| `coucou-daemon decide <id> allow\|deny` | Answers a pending request |
| `noctalia msg plugin coucou/mochi:daemon all toggle-pause` | Pauses or resumes Mochi |

## Development

```
(cd hook && cargo test) && (cd daemon && cargo test)
noctalia plugins lint noctalia
LUAU=/path/to/luau noctalia/tests/run.sh   # plugin specs against a mocked Noctalia host
```

## License

MIT for the code, see `LICENSE`. The names Coucou and Mochi, the Mochi character and the sounds are not covered by the MIT license; see `LICENSE-ASSETS.md`.
