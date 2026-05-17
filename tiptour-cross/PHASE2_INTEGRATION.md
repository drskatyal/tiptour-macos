# Phase 2 Integration Notes

Phase 2 (capability graph + tool registry) is self-contained inside
`src-tauri/src/capabilities/` and `src/capabilities/`. Two files need
manual edits to wire it into the existing app:

## `src-tauri/Cargo.toml`

No new dependencies are required. Phase 2 uses crates already declared by
Phase 0/1:

- `serde` / `serde_json`
- `dirs`
- `tauri` (for `#[tauri::command]` and `tauri::async_runtime::spawn_blocking`)
- `std::collections` / `std::time` (no crate edit needed)

If you want explicit unit tests under `cargo test`, no test-only deps are
needed either — the tests use only `std`.

## `src-tauri/src/main.rs`

Add the module declaration alongside the existing ones:

```rust
mod audio;
mod capabilities;   // <-- add this line
mod grounding;
mod hotkey;
mod keychain;
mod tray;
```

Append the four new Tauri commands to `invoke_handler`:

```rust
.invoke_handler(tauri::generate_handler![
    keychain::get_api_key,
    keychain::set_api_key,
    audio::start_mic_capture,
    audio::stop_mic_capture,
    audio::play_audio_chunk,
    grounding::prefetch_target_app,
    grounding::resolve_label,
    grounding::get_shortcut_index,
    capabilities::explore_app,            // <-- add
    capabilities::list_capabilities,      // <-- add
    capabilities::retrieve_tools,         // <-- add
    capabilities::invoke_capability,      // <-- add
])
```

## Wiring grounding into the explorer

The explorer uses a `GroundingProvider` trait (defined in
`capabilities/mod.rs`). It currently runs against a
`PlaceholderGroundingProvider` that returns `Err` on snapshot so
`explore_app` is a no-op until Phase 1's grounding module provides a
concrete impl. Once Phase 1 is ready:

1. Implement `capabilities::GroundingProvider` on a struct in the
   `grounding` module (e.g. `GroundingExplorerAdapter`).
2. In `mod.rs`, replace the `PlaceholderGroundingProvider` construction
   inside `explore_app` with the real adapter.
