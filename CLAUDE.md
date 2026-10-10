# Coucou — guide for AI coding agents

Linux-only fork of Coucou for Hyprland + Noctalia. Mochi lives on the Noctalia desktop and bar, shows AI coding agent sessions, and lets the user approve or answer them with a click.

## Where things are
- `hook/` — `coucou-hook` (Rust, serde_json + libc): the relay agents run on each hook event; one JSON line to `$XDG_RUNTIME_DIR/coucou.sock`, reply printed in each agent's format (`reply.rs`).
- `daemon/` — `coucou-daemon` (Rust, serde_json + libc): relay socket, control socket `coucou-ctl.sock` (`--follow`, `ack`, `decline`, `decide`, `--status-json`), and `install-hooks`. `coucou-daemon.service` is the systemd user unit.
- `mochi/` — Noctalia plugin `coucou/mochi` (Luau): `service.luau` (state machine), `desktop.luau` (animated Mochi), `bar.luau`, `panel.luau` (approvals and questions), `sounds/`, `translations/`, `tests/` (specs against a mocked host).
- `catalog.toml` — Noctalia plugin source catalog. Noctalia looks for a plugin in the directory named after its id suffix (`mochi/` for `coucou/mochi`).
- `linux/arch/PKGBUILD` — Arch package.
- `odd/tasks/` — ODD task documents.

## Build and test
```
(cd hook && cargo test && cargo build --release)
(cd daemon && cargo test && cargo build --release)
noctalia plugins lint mochi
LUAU=/path/to/luau mochi/tests/run.sh
```

## Rules
- No third-party dependencies unless truly unavoidable.
- No telemetry. Never block an agent: the hook exits immediately if the daemon does not answer, and no decision means silence.
- Never approve an agent permission automatically; decisions come only from a click in the panel.
- Never overwrite an agent's config without a diff, a dated backup and a confirmation.
- The wire format (`allow`, `deny`, `{"answers":{...}}`) and the timeouts (300 ms, 800 ms ack, 108 s decision, 110 s relay) are a contract between hook and daemon.
- Conventional commits, no AI attribution.
