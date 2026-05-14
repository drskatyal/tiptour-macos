# TipTour (cross-platform)

Tauri 2 build of TipTour that targets macOS and Windows from a single codebase. Sits next to the existing Swift macOS app in `../TipTour/` — the Swift app keeps building untouched. This directory is the long-term home of the Windows port and the eventual unified app.

Phase 0 covers only the voice loop: tray icon, floating panel, global push-to-talk hotkey, Gemini Live WebSocket round-trip. No automation, no screen capture, no overlay cursor yet — those layer on top.

## Stack

- **Shell**: Rust + Tauri 2 (native tray, native window, OS keychain, global hotkey, mic capture/playback).
- **UI**: TypeScript + Vite, rendered in the OS's native webview (WKWebView on macOS, WebView2 on Windows).
- **Model**: Gemini Live WebSocket, called directly from the webview with the user-supplied API key.

## Prerequisites

- Rust toolchain (stable) — https://rustup.rs
- Node 20+ and npm
- macOS: Xcode CLT (`xcode-select --install`)
- Windows: WebView2 runtime (preinstalled on Win11) and the MSVC build tools

## Develop

```bash
cd tiptour-cross
npm install
npm run tauri dev
```

`tauri dev` boots Vite on `:1420` and launches the Tauri shell pointing at it. Edits to `src/` hot-reload; edits to `src-tauri/` rebuild the Rust binary.

## Build

```bash
npm run tauri build
```

Outputs platform installers under `src-tauri/target/release/bundle/`.

## Layout

```
src/                  TypeScript UI + Gemini Live client
src-tauri/            Rust shell (tray, hotkey, audio, keychain)
src-tauri/icons/      Tray + app icons (placeholders until designed)
```

## Tray icons

`src-tauri/icons/` needs real PNGs before first build. Drop in:
- `tray.png` (16×16 and 32×32 @2x)
- `icon.png` (512×512 app icon)

For Phase 0 development you can use any placeholder. The app fails to launch with no icon at all because Tauri's window builder requires one.

## Where Phase 0 ends

When the six checks in `/root/.claude/plans/kind-drifting-pretzel.md` (or the PR description) pass on both Windows 11 and macOS 14+, Phase 0 is done. Phase 1 (UIA shortcut indexer on Windows, AX prefetch on macOS) starts on top of this scaffold.
