# TipTour

**A voice agent that lives in your menu bar, sees your screen, and runs your computer.**

Press **Option + X** (mac) or **Alt + X** (Windows). Talk. TipTour streams what you say and what you see to Gemini Live in realtime, then clicks, types, and scrolls for you. Or it points and you click — your choice.

Built as a Tauri 2 native app. Single codebase, native shell on macOS (WKWebView) and Windows (WebView2). One Gemini API key, stored in your OS keychain, never leaves your machine.

## What it does

- **Realtime voice agent** — Gemini Live bidirectional WebSocket. Speak naturally; it speaks back. Audio in/out, screen vision, and tool calls all over one streaming connection.
- **Autopilot or Teaching mode** — Autopilot drives the computer for you (clicks, hotkeys, types, scrolls). Teaching points at the next thing to click so you learn.
- **Action grounding via AX / UIA** — pixel-perfect element lookup against the macOS Accessibility tree (mac) and UI Automation (Windows), with Gemini's `box_2d` spatial coordinates as a last-resort fallback. No flaky vision-only clicking.
- **Focus highlight brush** — hold Ctrl+Shift and paint over a region of the screen. Gemini gets the painted area, the window underneath, the AX/UIA element it intersects, the user's text selection, and a normalized screenshot crop. Say "rewrite this paragraph" and it stays inside the highlighted text — never the wrong app.
- **Multiflow** — record a cross-app routine once, recall it by voice ("run my morning standup setup"). Opt-in input recording, stored locally.
- **Agent memory** — long-running facts, preferences, and notes the agent remembers across sessions. JSON-backed bag-of-words by default; build with `--features agent-memory-vector` for on-device semantic search (LanceDB + quantized MiniLM-L6-v2 via fastembed-rs; downloads ~30 MB ONNX model on first use).
- **Sub-agent kanban** — long-running tasks fan out into named sub-agents with their own budgets, transcripts, and a kanban board to track them. (Heads up — sub-agent Gemini Live sockets are stubbed in this build; the registry, budgets, transcripts, and UI are wired end-to-end but conversational loops for spawned agents land in the next milestone.)
- **On-device wake word** *(optional)* — say "tiptour" without touching the hotkey. Vosk, fully local, no audio leaves your machine. Build with `--features vosk`. Requires `libvosk` on your dynamic library path (`libvosk.dll` next to the exe on Windows; brew/apt install on mac/Linux).
- **Indicators rail** — fourth always-on-top window pinned to the side of your screen showing live status (listening, thinking, executing), running tasks, and recent flow hits.
- **Custom commands** — define a shortcut name + system prompt once; trigger by voice forever.
- **Cross-platform app discovery** — scans installed apps at boot so the agent knows what you have and can launch them by alias.
- **Built-in screen recording** — voluntary, used by the flow recorder. Stored locally under `~/Library/Application Support/TipTour/recordings/` (mac) or `%APPDATA%\TipTour\recordings\` (Windows).

## Quick start

```bash
git clone <this repo>
cd tiptour-cross
npm install
npm run tauri dev
```

The first launch:

1. Tray icon appears. Click it to open the floating panel.
2. Paste your [Gemini API key](https://aistudio.google.com/apikey) — stored in your OS keychain.
3. **macOS only**: grant Accessibility + Screen Recording when prompted. **Quit and relaunch after granting Screen Recording** — TCC caches per-pid, so the running process won't see the new permission.
4. Press **Option + X** (mac) or **Alt + X** (Windows) anywhere on the system. Start talking.

That's it. End-to-end voice → action in under a minute.

## Build

```bash
# default build (no vosk, no vector memory)
npm run tauri build

# everything on
cd src-tauri
cargo build --release --features "vosk agent-memory-vector"
```

Installers land in `src-tauri/target/release/bundle/`.

### Feature flags

| Flag | What it adds | What it costs |
|------|-------------|---------------|
| `vosk` | "tiptour" wake-word, fully on-device | Requires libvosk on dyld path |
| `agent-memory-vector` | Semantic memory recall (LanceDB + MiniLM-L6-v2) | First-run downloads ~30 MB ONNX model; longer compile (Arrow + ONNX Runtime) |

Default builds work without either — Vosk is hidden behind the always-on-listening toggle, memory falls back to a bag-of-words ranker.

## Prerequisites

- Rust toolchain (stable) — https://rustup.rs
- Node 20+ and npm
- **macOS**: Xcode CLT (`xcode-select --install`); macOS 14+ recommended for ScreenCaptureKit
- **Windows**: WebView2 runtime (preinstalled on Win11), MSVC build tools
- **Optional**: libvosk if you build `--features vosk`

## Architecture at a glance

```
src/                        TypeScript UI (4 webviews)
  ├── main.ts                 Floating panel
  ├── overlay.ts              Full-screen cursor / response overlay
  ├── indicators.ts           Side-of-screen status rail
  ├── settings/               10-tab settings dashboard
  └── gemini/                 Gemini Live WebSocket client
src-tauri/src/              Rust shell
  ├── tray.rs / hotkey.rs     Menu bar, global shortcut
  ├── executor/               Click + type + scroll dispatch
  ├── grounding/              AX (mac) + UIA (win) element lookup
  ├── recorder/               Input recorder + pattern miner
  ├── multiflow/              Recorded-flow replayer
  ├── agent_memory/           JSON + (optional) vector backends
  ├── subagents/              Pool, budgets, transcripts, kanban
  ├── vosk_listener/          On-device wake word
  ├── app_discovery/          Installed-app scan + alias gen
  └── capabilities/           Per-app capability graph
```

A companion Swift macOS app lives under `../TipTour/` in the parent repo — Phase 0 of this Tauri port replaced its voice loop; remaining native features (the focus highlight, kanban, indicators) are now exclusive to this codebase.

## Privacy

- Your Gemini API key is stored in the macOS Keychain (mac) or Windows Credential Manager (win). Never in plaintext on disk, never sent anywhere except `generativelanguage.googleapis.com`.
- Screen captures and microphone audio go directly from your machine to Gemini Live over a TLS WebSocket. There is no TipTour-operated proxy.
- The "always-on listening" wake word runs locally with Vosk. Audio in that mode never leaves the machine until the wake word triggers.
- Input recording (for multiflow) is **opt-in** behind a toggle on the panel, off by default.

## Status

Pre-1.0. Known gaps:

- Sub-agent Gemini Live sockets are stubbed (kanban, budgets, transcripts all work; the conversational loop for spawned agents is the next milestone).
- Windows code paths are present and compile but have received less testing than macOS.
- No code signing yet — expect Gatekeeper / SmartScreen warnings on first launch.

## License

MIT. See [LICENSE](./LICENSE).
