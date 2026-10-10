# Feature: Full parity with the original Linux app

## Objective
Keep every user-facing feature of the original Linux app (upstream/main 190fc21, `windows/`), with native Hyprland + Noctalia integration (Noctalia plugin + `coucou-daemon` + `coucou-hook`).

## Why
User requirement (2026-10-10): the fork must keep everything (LLM API config and chat, connecting installed agents, Mochi's reactions and interactions with user activity, integrations), only the platform integration changes.

## Constraints
- Noctalia plugin API: no canvas, no secret storage, no pointer coordinates outside onHover, desktop widgets cannot self-move nor take right click on images.
- Secrets via Secret Service (`secret-tool`), never on disk.
- Never block an agent; never approve without an explicit click; dated backup + diff + confirmation before writing any agent config.
- Original drawing reused through frames rendered from the vendored engine (tools/render-mochi).

## Gap analysis
Done 2026-10-10 against upstream/main 190fc21 (explorer). Present: relay, daemon approvals, install-hooks CLI (Claude, Gemini, Antigravity, Codex), frame Mochi with states/slap/dizzy/love/greet, approval + question panel, outfits auto/setting, 17 sounds.

## Tasks (each ~400 changed lines, heuristic only)
- [x] P1 Daemon: `install-hooks --status --json` (detected/connected per agent) and `--dry-run --json` (diff, backup path) — 12461aa
- [x] P2 Plugin: Agents section (detected agents, connected chip, diff preview, Apply/Remove with explicit click) — 2873fca
- [x] P3 Daemon + relay: Copilot CLI, Cursor, Muse, OpenCode, Amp, Hermes (port agents.rs + normalize.rs) — 0f4db82
- [ ] P4 Frames: emotes happy, proud, surprised, wink, yawn, annoyed, dance, box/gulp, peek
- [ ] P5 Reactions logic: sleep after 120 s agent inactivity, finished→happy, outfit change→proud, random idle emotes, first-name greeting, all original sounds
- [ ] P6 Secrets: daemon `secret get|set|has` over Secret Service; Keys section (presence only)
- [ ] P7 LLM chat core: chat panel, streaming Anthropic + OpenAI-compatible (OpenAI, OpenRouter, Ollama, LM Studio, custom), hidden think blocks, markdown
- [ ] P8 LLM chat extras: Google, model list, provider switch, file drop attachments + gulp, chat shortcut
- [ ] P9 Sessions: session list/pills, finished card + notify, open terminal (relay pid → /proc → hyprctl focus)
- [ ] P10 Shortcuts ([[shortcut]]) and wardrobe grid with preview
- [ ] P11 Diff ticker and viewer
- [ ] P12 Integrations: Spotify (playerctl) + dance, plan usage (statusline relay), GitHub, Stripe
- [ ] P13 Weekly recap (markdown card, wl-copy share) and i18n from strings.json; recap on logind wake
- [ ] P14 Companion layer-shell surface `coucou-mochi` (user approved 2026-10-10): eyes follow cursor (hyprctl cursorpos / socket2, ~8 Hz only while visible), free drag, double-click home, peek at the top edge, open on hover, follow focused monitor, sleep with screens (logind/hypridle), window drop as chat context (hyprctl clients + grim)
- [ ] P15 Universal MPRIS (any player, replaces Apple Music and Spotify) + dance
- [ ] P16 Demo mode (scripted in-memory session, no disk writes)
- [ ] P17 Dictation in chat: whisper.cpp sidecar, PipeWire capture, multi-language
- [ ] P18 Voice phases 1–2: wake word "OK/Hey Coucou" (openWakeWord + VAD, pause on lock), turn-end policy, conversation context, IntentParser + EntityResolver (music, pills), Piper/espeak spoken replies, captions, transcript history
- [ ] P19 Voice brain (Ollama tool calls) and voice functions: open apps/files/URLs, focus agent/terminal, dictate to agent, deny by voice (never allow), timers/reminders, state questions
- [ ] P20 Email a dropped file (xdg-email / Resend), Cal.com calendar grid, other integrations at parity (n8n, Vercel, Resend, Notion)
- [ ] P21 Phone link stand-in: ntfy actionable Allow/Deny notifications (iPhone CloudKit sync and Live Activities not practical from Linux)

## Not possible on Linux
- iPhone app sync, widgets and Live Activities: depend on the user's private iCloud CloudKit DB and an iOS-issued push token. Stand-in: P21.
- System share sheet: replaced by wl-copy / OpenURI portal.

## Acceptance criteria
Per task: Rust tests (`cargo test`), Luau specs (`mochi/tests/run.sh`), `noctalia plugins lint mochi`, live check on Hyprland/Noctalia where applicable.

## Progress
- 2026-10-10: document created from the gap analysis.
- 2026-10-10: P1 done (12461aa, delegated writer). `install-hooks --status [--json]` reports per agent detected (binary on PATH or its folder; the original app had no detection, so this is new), installed (Coucou entries present) and connected (installed and every relay those entries run exists), plus `relay_ok` for the relay Connect would use. `--dry-run --json` reports changed, diff, backup_path and per-file details, writing nothing. Plans now cover several files and file removal. Tests first: RED (missing `status`, `dry_run_json`, `Mode`, `relay_of_command`), then GREEN.
- 2026-10-10: P3 done (0f4db82, delegated writer). Copilot CLI (own file, removed when only Coucou's entries were in it), Cursor (flat camelCase entries + version), Muse (matcher groups, ms, schema_version when fresh), OpenCode/Amp/Hermes plugins from the original templates via include_str! (daemon/src/templates/), byte-equivalent to upstream except the relay path (checked against `git show upstream/main` output). A plugin file Coucou did not write is refused. hook/: normalize.rs and reply.rs are identical to upstream/main, so every agent mapping and the Claude Desktop tagging were already there; nothing to port (Claude Desktop has no Linux build, so no installer). Only hook/statusline.rs is missing (P12).
- 2026-10-10: P2 done (2873fca, delegated writer). New [[panel]] `agents` (mochi/agents.luau, 520x560): status list with Installed/Connected chips, Connect/Reconnect/Disconnect open a dry-run preview (```diff in ui.markdown + backup path); only the confirm button runs `install-hooks --agent <id> --yes [--remove]`. Reachable from the desktop hover plug button, "Agents…" in the bar's approval panel, and `noctalia msg plugin coucou/mochi:daemon all agents`. Specs: mochi/tests/agents_spec.luau plus entry-point checks in desktop/panel/service specs.
- Evidence (2026-10-10): daemon `cargo test` 47 passed, `cargo build --release` ok; hook `cargo test` 27 passed, release ok; `mochi/tests/run.sh` ALL/DESKTOP/BAR/PANEL/AGENTS PASSED; `noctalia plugins lint mochi` 0 errors, 0 warnings; no ERR lines for mochi in ~/.cache/noctalia/noctalia.log after hot reload. CLI samples run against a scratch --home only.
- Pending: the installed ~/.local/bin/coucou-daemon predates --status, so the live Agents panel shows a load error until the new binary is installed (not done: the running daemon must not be touched). No preview-to-apply fingerprint check (the original app had one): apply recomputes from the current file. Hermes' plugin folder keeps the dated backups after Disconnect (same as the original app). RDD review not run by the writer.
