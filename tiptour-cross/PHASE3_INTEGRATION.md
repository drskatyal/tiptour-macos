# Phase 3 Integration

Recorder scaffolding is additive — no existing files were modified. To wire
it into the build:

## 1. `src-tauri/Cargo.toml`

Add to `[dependencies]`:

```toml
rdev = "0.5"
hound = "3.5"
chrono = { version = "0.4", features = ["clock"] }
uuid = { version = "1", features = ["v4"] }
```

Already-present dependencies that the recorder also uses (no change required,
listed for completeness): `serde`, `serde_json`, `once_cell`, `parking_lot`,
`dirs`, `tauri`.

## 2. `src-tauri/src/main.rs`

Add the module declaration:

```rust
mod recorder;
```

Extend `invoke_handler` with the recorder commands:

```rust
.invoke_handler(tauri::generate_handler![
    // ...existing commands...
    recorder::is_recording_enabled,
    recorder::set_recording_enabled,
    recorder::start_passive_recording,
    recorder::stop_passive_recording,
    recorder::start_demonstration,
    recorder::stop_demonstration,
    recorder::append_narration_audio_chunk,
    recorder::list_demonstrations,
    recorder::load_demonstration,
    recorder::mine_patterns,
])
```

## 3. Wiring the real state provider (Phase 1 grounding)

`recorder::recorder::Recorder::new()` defaults to `NullStateProvider`. When
the grounding layer is ready to publish a `StateSnapshot`, expose it as an
`Arc<dyn StateProvider>` and inject it into the `GLOBAL_RECORDER` (the
`set_state_provider` placeholder in `recorder.rs` shows the intended shape;
the current `Arc<dyn StateProvider>` field needs to move behind a `Mutex` or
`ArcSwap` when this hookup lands).

## 4. Permissions

`rdev` requires macOS Accessibility permission (already granted for the
hotkey path) and is unrestricted on Windows. No new TCC entitlements are
needed.

## 5. Default-off invariants

- `recorder_settings.json` starts with `is_recording_enabled: false`.
- `start_passive_recording` and `start_demonstration` both refuse to run
  unless the persisted flag is true.
- `privacy::should_pause` is consulted on every input event before the
  trace touches disk; password fields and known credential-manager apps
  short-circuit the write.
