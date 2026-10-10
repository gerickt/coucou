# Coucou for Hyprland + Noctalia

Mochi, a tiny character in your [Noctalia](https://github.com/noctalia-dev/noctalia-shell) bar, shows what your AI coding agents (Claude Code, Gemini CLI, Antigravity…) are doing: idle, thinking, running a tool, or waiting for a permission.

This is a Linux-only fork of [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou). The macOS and Windows apps were removed.

## How it works

- `daemon/` — `coucou-daemon`, a small Rust daemon. It listens on `$XDG_RUNTIME_DIR/coucou.sock` for one JSON line per hook event and writes the current state to `$XDG_RUNTIME_DIR/coucou-state.json`. `coucou-daemon --status-json` prints that state.
- `noctalia/` — the Noctalia plugin (`coucou/mochi`): a bar widget and a service that polls the daemon.

## Install

Arch Linux: `cd linux/arch && makepkg -si`.

Manual:

```
cd daemon && cargo build --release
install -Dm755 target/release/coucou-daemon ~/.local/bin/coucou-daemon
cp -r ../noctalia ~/.config/noctalia/plugins/coucou-mochi
```

Run `coucou-daemon` at login (for example `exec-once = coucou-daemon` in Hyprland).

## Hooking an agent

Send one JSON line to the socket from the agent's hook, for example:

```
echo '{"event":"PreToolUse","tool_name":"Bash","agent":"Claude Code"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/coucou.sock
```

Events: `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `Stop`.

## License

MIT for the code, see `LICENSE`; artwork under `LICENSE-ASSETS.md`.
