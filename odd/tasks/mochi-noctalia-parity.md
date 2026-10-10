# Feature: Mochi parity as a native Noctalia plugin

## Objective
Ship Coucou for Hyprland + Noctalia as an installable Noctalia plugin whose core is an interactive, animated on-screen Mochi, keeping the essential behaviour of the original Linux app (Tauri, deleted at 081a2ad:windows/).

## Problem
The current `noctalia/` plugin is a text glyph in the bar; the daemon only tracks a state string. No character on screen, no approvals, no questions, no sounds, no hook installer, invalid manifest (`type = "boolean"`), right-click IPC unhandled.

## Constraints
- Noctalia 5.2.1 Luau plugin API: no canvas/path primitives; draw Mochi from nested rounded `ui.box` nodes animated with `desktopWidget.setNeedsFrameTick`.
- Desktop widgets cannot take keyboard focus; approvals/questions use a `[[panel]]`.
- No Unix-socket API in Luau: plugin reads daemon via `noctalia.runStream`, sends decisions via `runAsync`.
- Never block the agent: relay connect 300 ms, ack 800 ms, decision 108 s, silence on timeout (original contract).
- Never approve without an explicit click. Never overwrite agent configs without dated backup + diff + confirmation.

## Scope (essential)
- Relay `coucou-hook` restored from 081a2ad:windows/hook (wire format + replies + timeouts).
- `coucou-daemon`: socket server compatible with the relay, pending-request registry, `--follow` event stream, `--decide` control, `install-hooks` (Claude Code, Gemini, Antigravity, Codex) with backup/diff/confirm.
- Plugin: service (state machine from original hooks.ts), desktop widget (animated Mochi: idle, working, thinking, searching, approval, question, error, finished, ratelimit, sleeping, dizzy; blink, bob, squash, badges; click=annoyed, triple click=dizzy, hover 2 s=love, click on approval opens panel), bar widget (mini Mochi + tool), panel (approval Allow/Deny, AskUserQuestion options), original WAV sounds, settings, catalog.toml for install as a plugin source.

## Out of scope (deferred, documented)
Chat providers, GitHub, Spotify, weekly recap, plan usage, file drop, diff viewer, 3D outfits, global shortcuts.

## Tasks
- [x] T1 Relay + daemon protocol (Rust) with unit tests — route: delegated (writer trigger: 2+ non-trivial files) — d1e6641, a346672 (multi-select answers), 0c36438 (heartbeat)
- [x] T2 Hook installer with backup/diff/confirm + tests — route: delegated — 45128cd
- [x] T3 Plugin: service + desktop Mochi + bar + panel + sounds + manifest/lint — route: delegated — fe1f852
- [x] T4 Packaging: catalog.toml, PKGBUILD, README install-as-plugin — route: delegated — be54766 (git-source layout gap, see Progress)
- [x] T5 Live validation on Hyprland/Noctalia: fake hook events, screenshot, approve/deny round trip — route: parent (inline: state checks and one CPU fix)

- [x] T6 Reuse the original Mochi: render windows/src/mochi/engine.ts + outfits.ts (081a2ad) unmodified in headless Chrome to PNG frame sequences per state/emote/reaction/outfit; desktop and bar widgets play those frames instead of the box-drawn Mochi — route: delegated (user correction: original character, animations and reactions must be reused, not redesigned) — e060ac9 (renderer), 2997335 (frames), 3d5514e (widgets)

## Acceptance criteria
- `cargo test` and `cargo build --release` pass for daemon and hook.
- `noctalia plugins lint mochi` passes.
- A fake PermissionRequest piped through `coucou-hook` shows Mochi in approval state and the panel; clicking Allow returns the allow JSON to the hook; no click returns silence after timeout.
- Mochi visible and animated on the desktop; click/hover reactions observed.

## Progress
- Created 2026-10-10.
- T1 (d1e6641, a346672, 0c36438): `hook/` restored from 081a2ad:windows/hook (Unix socket only, statusline and named pipes dropped). `coucou-daemon` rewritten: relay socket `coucou.sock`, control socket `coucou-ctl.sock` (`follow`, `status`, `ack`, `decline`, `decide`), ack 800 ms / decision 108 s (env overrides `COUCOU_ACK_TIMEOUT_MS`, `COUCOU_DECISION_TIMEOUT_MS`), one card at a time (extra requests broadcast with `coucou_declined` and get silence), no followers means immediate silence, relay hang-up detected (`gone`), `resolved` events, 15 s `ping` heartbeat. RED observed for hub tests (10 failing on `todo!()`) and multi-select answers before GREEN.
- T2 (45128cd): `coucou-daemon install-hooks [--agent claude|gemini|antigravity|codex]... [--remove] [--yes] [--home] [--hook-path]`: strict JSON read, merge preserving foreign entries, idempotent, line diff, `<file>.coucou-backup-YYYYMMDD-HHMMSS` (UTC), atomic write keeping permissions, write only on `y`/`--yes`. RED observed (11 failing on `todo!()`) before GREEN.
- T3 (fe1f852): plugin rewritten: `service.luau`, `desktop.luau`, `bar.luau`, `panel.luau`, 14 original WAVs, valid manifest (`plugin_api = 24`, `bool`/`int` settings), en translations, Luau specs in `noctalia/tests/` (run with `LUAU=... noctalia/tests/run.sh`).
- T4 (be54766): root `catalog.toml`, `daemon/coucou-daemon.service`, PKGBUILD builds+tests both crates and installs `/usr/share/coucou/noctalia/{catalog.toml,mochi/}` as a local plugin source, CI for both crates, README/CHANGELOG/CLAUDE.md.

### Verification evidence (2026-10-10)
- `cd hook && cargo test && cargo build --release`: 27 passed, release build ok, no warnings.
- `cd daemon && cargo test && cargo build --release`: 38 passed, release build ok, no warnings.
- `noctalia plugins lint noctalia`: `coucou/mochi ok`, 0 errors, 0 warnings (also on the packaged source layout).
- Luau specs (luau 0.740 CLI, mocked host): service ALL PASSED (24 checks), desktop/bar/panel PASSED; `luau-compile` syntax ok for all entries.
- E2E smoke (temp XDG_RUNTIME_DIR): follow + `coucou-hook PermissionRequest` + `ack` + `decide <id> allow` -> hook printed `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`; with `COUCOU_DECISION_TIMEOUT_MS=1000` and ack but no decision -> 0 bytes after ~1007 ms; no daemon -> 0 bytes in ~3 ms.

### Open gaps
- Git plugin source needs the plugin in `mochi/` at the repo root (Noctalia resolves `<id suffix>/plugin.toml`; verified on all 246 community rows). The repo keeps `noctalia/`, outside the authorized rename surface: `git mv noctalia mochi` (and path updates) is required before `plugins source add coucou git ...` works. The packaged path source already uses `mochi/`.
- No volume control: `noctalia.sound` has only `load`/`play`; only an on/off setting ships.
- `serde_json` sorts keys when install-hooks rewrites a config (no `preserve_order`, to avoid extra deps).
- Untested live: panel placement, glyph/label rendering of `♥ ^ @ > <` eyes, bar `state.watch` delivery, desktop frame cost (renders capped at ~30 fps), `getConfig("sound_enabled")` from the desktop entry.
- Sounds, the Mochi name and character are under `LICENSE-ASSETS.md` (not MIT); distributing them in a public plugin source needs the owner's permission.
- Engram mirror `odd/mochi-noctalia-parity/tasks`: pending (worker did not write it).

### T5 live validation (2026-10-10, Noctalia 5.2.1, Hyprland 0.56.2)
- Plugin moved to `mochi/` (d5bdb4f); loaded from local source `noctalia msg plugins source add coucou path <repo>`: log `loaded plugin 'coucou/mochi' (4 entries)`, service started, 14 sounds loaded, `coucou-daemon --follow` attached.
- Desktop widget via `[desktop_widgets.widget.mochi_main] type = "coucou/mochi:desktop"`: layer surface 240x250 on Bottom layer; screenshots show idle, approval (!), thinking (dots), finished (^ ^), question (?) and the working state animating between frames.
- PermissionRequest: panel "Allow Bash?" with command, Deny/Allow; `decide <id> allow` (the Allow button's call) -> hook printed the allow JSON. A second concurrent request was declined (silence, terminal asks).
- AskUserQuestion: panel with options; answer -> hook printed allow + updatedInput.answers.
- CPU: desktop Mochi cost ~8 points idle at 30 fps; now drops frame ticks when calm and steps from update() at 8 Hz: 3.2% total vs 1.4% with desktop widgets hidden.
- Not verified: real mouse clicks/hover (no ydotool/wtype to inject input), sound playback audibility, multi-select UI.

### T6 original Mochi frames (2026-10-10)
- `tools/render-mochi/`: `vendor/` = engine.ts, outfits.ts, anim.ts, fonts.ts from 081a2ad byte for byte (render.sh re-checks with `git show`; sha256 in `vendor/SOURCE.txt`); only `core/sound` is shimmed (no-op `Sound.play`). Types stripped by `node:module` (no esbuild). `page/driver.js` replaces `performance.now`, `Math.random` (seeded mulberry32) and `setTimeout` (virtual queue), steps `update()` at 1/30 s like the desktop window and calls only BotEngine's public API (`setState`, `slap`, `greet`, `triggerEmote`, `gulp`, `animateMorph`, `setOutfit`). Desktop framing = windows/src/desktop/main.ts (120 px window, 96 px draw width) at 1.5x (180 px); bar 32 px at 2x.
- Matrix: desktop, 12 outfits x (11 state loops + approval/error/finished/dizzy entrances + dizzy exit + slap, love, greet); `none` also has happy, surprised, proud, wink, yawn, annoyed, gulp. Bar: 12 outfits x 11 state loops. 367 sequences, 10434 rendered frames -> 4747 PNGs, 14.98 MB (desktop 128-colour, bar 48-colour per-sequence libimagequant palette, nearest mapping without dither so still parts do not shimmer, content-hash dedupe, run-length holds in `mochi/frames/manifest.luau`).
- Widgets: `mochi/sprite.luau` (shared player + wardrobe.ts seasonal outfit port, required by both entries, gets the host through `sprite.setup(noctalia)`). Desktop: greet on start, state loops with approval/error/finished entrances, click = slap (+ annoyed 60 ms later), 3 slaps in 1.7 s = dizzy 3.3 s then dizzy.exit (happy), hover 1.9 s = love with a 6 s cooldown, click on approval/question toggles the panel, paused = sleeping. Frame ticks only when the next change is due within 120 ms; otherwise `setUpdateInterval` set to the next change (50-1000 ms). Bar: same loops, 26 px, interval 50-400 ms. New plugin-level `outfit` select (auto, none, 11 outfits), desktop `size` default 160. Added original `annoyed`, `greet`, `hover` WAVs (081a2ad^).
- Evidence: `render.sh` twice (WORK dirs a/b): sha256 of the full sorted file list identical (`78f5c609...`), `manifest.luau` sha256 `a0d3e9a2...`; `render.sh --compare`: 37 shipped frames vs fresh engine frames at the same virtual instant, composited over grey, mean abs diff <= 0.592/255 (avg 0.323), max 49 on a few edge pixels (palette). `noctalia plugins lint mochi`: 0 errors, 0 warnings. Luau specs: ALL/DESKTOP/BAR/PANEL PASSED (every outfit has every sequence, entrances, slap/annoyed, dizzy + recovery, hover love + cooldown, approval click, paused, outfit, seasons incl. Easter 2026/2027). Live Noctalia: hot reload loaded `sprite.luau` and `frames/manifest.luau`, 6 sounds loaded, bar shows the witch-hat Mochi (auto, October); no ERR lines.
- Gaps: reactions are rendered on the idle base (a click while working shows the white body for the slap); no gaze following the pointer (lookX/lookY = 0); hover's eye widening (`tgEs` 1.08) and the double-click home / drag of the original desktop window are not reproduced; dancing not rendered; the running Noctalia keeps the old manifest until the plugin is re-enabled, so it logs "read undeclared setting 'outfit'" and uses the default (auto); render.sh wipes `mochi/frames` while a live plugin reads it (transient "image failed to load" warnings during regeneration only).
- Engram mirror: pending (worker did not write it).

## Next step
User: click-test Mochi (click, triple click, hover 2 s) and Allow/Deny; run `coucou-daemon install-hooks` to wire real agents; push and decide on asset licence before publishing the source.
