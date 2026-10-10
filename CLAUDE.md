# Coucou — guide for AI coding agents

Linux-only fork of Coucou for Hyprland + Noctalia. Mochi lives in the Noctalia bar and shows AI coding agent sessions.

## Where things are
- `daemon/` — `coucou-daemon` (Rust, std + serde + libc). Unix socket in, state JSON out.
- `noctalia/` — Noctalia plugin `coucou/mochi` (Luau): `bar.luau`, `service.luau`, `plugin.toml`, `translations/`.
- `linux/arch/PKGBUILD` — Arch package.
- `odd/tasks/` — ODD task documents.

## Build
```
cd daemon && cargo build --release
```

## Rules
- No third-party dependencies unless truly unavoidable.
- No telemetry. Never block an agent: the hook must exit immediately if the daemon does not answer.
- Never approve an agent permission automatically.
- Never overwrite an agent's config without a dated backup.
- Conventional commits, no AI attribution.
