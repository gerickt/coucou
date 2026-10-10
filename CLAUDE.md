# Coucou — guide for AI coding agents

Linux-only fork of Coucou for Hyprland + Noctalia. Mochi floats above the windows (the `coucou-mochi` overlay) and lives in the Noctalia bar, shows AI coding agent sessions, and lets the user approve or answer them with a click.

## Where things are
- `hook/` — `coucou-hook` (Rust, serde_json + libc): the relay agents run on each hook event; one JSON line to `$XDG_RUNTIME_DIR/coucou.sock`, reply printed in each agent's format (`reply.rs`).
- `daemon/` — `coucou-daemon` (Rust, serde_json + libc): relay socket, control socket `coucou-ctl.sock` (`--follow`, `ack`, `decline`, `decide`, `--status-json`), and `install-hooks`. `coucou-daemon.service` is the systemd user unit.
- `mochi/` — Noctalia plugin `coucou/mochi` (Luau): `service.luau` (state machine; starts the overlay and writes `$XDG_RUNTIME_DIR/coucou-mochi.json` for it), `desktop.luau` (animated Mochi on the desktop layer; steps aside for the overlay), `bar.luau`, `sprite.luau` (frame player, required by both widgets), `wardrobe.luau` (seasonal outfit), `panel.luau` (approvals and questions), `frames/` (generated), `sounds/`, `translations/`, `tests/` (specs against a mocked host).
- `overlay/` — `coucou-mochi` (Rust, smithay-client-toolkit + png + serde_json + libc): Mochi on a wlr-layer-shell surface above the windows (Top layer, input region = his body), follows Hyprland's focused monitor (socket2) and cursor (request socket). `mochi.rs` ports `desktop.luau`'s behaviour and `player.rs` ports `sprite.luau`'s player: keep them in step. `coucou-mochi.service` is an optional systemd user unit.
- `tools/render-mochi/` — generates `mochi/frames/`. `vendor/` holds the original engine files from 081a2ad, byte for byte. `render.sh` checks them against git, renders them in headless Chrome, then palettes and dedupes the frames and writes `mochi/frames/manifest.luau` and `manifest.json` (the overlay's copy).
- `catalog.toml` — Noctalia plugin source catalog. Noctalia looks for a plugin in the directory named after its id suffix (`mochi/` for `coucou/mochi`).
- `linux/arch/PKGBUILD` — Arch package.
- `odd/tasks/` — ODD task documents.

## Build and test
```
(cd hook && cargo test && cargo build --release)
(cd daemon && cargo test && cargo build --release)
(cd overlay && cargo test && cargo build --release)
noctalia plugins lint mochi
LUAU=/path/to/luau mochi/tests/run.sh
tools/render-mochi/render.sh   # only after changing tools/render-mochi (sequences, framing)
```

## Rules
- No third-party dependencies unless truly unavoidable.
- No telemetry. Never block an agent: the hook exits immediately if the daemon does not answer, and no decision means silence.
- Never approve an agent permission automatically; decisions come only from a click in the panel.
- Never overwrite an agent's config without a diff, a dated backup and a confirmation.
- The wire format (`allow`, `deny`, `{"answers":{...}}`) and the timeouts (300 ms, 800 ms ack, 108 s decision, 110 s relay) are a contract between hook and daemon.
- Performance: the overlay draws only when his frame changes, polls the cursor only while he is awake and visible, and sleeps on its event loop otherwise (0 % CPU when nothing animates).
- Mochi is the original character: never redraw him in Luau or Rust. Change his looks only through `tools/render-mochi` (sequences, framing), never by editing `vendor/` or the generated `mochi/frames/`.
- Conventional commits, no AI attribution.
