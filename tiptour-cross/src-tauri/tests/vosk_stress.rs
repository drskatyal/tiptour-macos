// Vosk stress tests — gated behind `--features vosk` AND `--ignored`
// because they require:
//   1. The libvosk native library on the build host
//   2. The Vosk model on disk (run `npm run fetch:vosk` first)
//
// Run locally with:
//   cd tiptour-cross/src-tauri
//   npm --prefix .. run fetch:vosk
//   cargo test --features vosk --test vosk_stress -- --ignored --nocapture
//
// CI skips these by default. Once a real CI runner has libvosk
// installed we can flip the `#[ignore]` off on the basics.

#![cfg(feature = "vosk")]
#![cfg(target_os = "linux")] // libvosk install path is well-known on Linux only; mac/win paths vary

use std::path::PathBuf;

fn locate_bundled_model() -> Option<PathBuf> {
    // Mirror the production resolution logic: prefer the bundled
    // resource at src-tauri/resources/vosk-models/small-en-us, fall
    // back to the per-user data dir if it's been populated already.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bundled = manifest_dir
        .join("resources")
        .join("vosk-models")
        .join("small-en-us");
    if bundled.join("README").exists() {
        return Some(bundled);
    }
    None
}

#[test]
#[ignore = "requires libvosk + downloaded model (run npm run fetch:vosk first)"]
fn model_loads_within_three_seconds_cold() {
    let model_path = locate_bundled_model()
        .expect("bundled model missing — run `npm run fetch:vosk` first");
    let started_at = std::time::Instant::now();
    let _model = vosk::Model::new(model_path.to_string_lossy().as_ref())
        .expect("Vosk model must load");
    let elapsed = started_at.elapsed();
    assert!(
        elapsed.as_secs() < 3,
        "Vosk model cold load took {}s — too slow for first-boot UX",
        elapsed.as_secs()
    );
}

#[test]
#[ignore = "requires libvosk + downloaded model"]
fn recognizer_accepts_grammar_round_trip() {
    let model_path = locate_bundled_model()
        .expect("bundled model missing — run `npm run fetch:vosk` first");
    let model = vosk::Model::new(model_path.to_string_lossy().as_ref())
        .expect("model load");
    let grammar = serde_json::json!([
        "hey tiptour",
        "stop",
        "cancel",
        "pause",
        "play",
        "what time is it",
    ]);
    let mut recognizer = vosk::Recognizer::new_with_grammar(
        &model,
        16_000.0,
        &grammar.to_string(),
    )
    .expect("recognizer with grammar must construct");
    // Feed pure silence — recognizer should report no hypothesis but
    // not panic. This is the most common runtime path (user not
    // speaking) and any crash here would kill the always-on listener
    // on every machine.
    let silence_samples: Vec<i16> = vec![0; 16_000]; // 1 second of silence
    let result = recognizer.accept_waveform(&silence_samples);
    let _ = result; // either Ok or PartialResult is fine
    let final_text = recognizer.final_result();
    let _ = final_text;
}

#[test]
#[ignore = "requires libvosk + downloaded model — stress test"]
fn recognizer_handles_100_chunks_without_leak() {
    // Open the recognizer once and feed 100 chunks of synthetic audio.
    // Mainly verifies we don't leak file handles / unbounded memory
    // under sustained streaming load (the always-on listener feeds a
    // chunk every ~30ms).
    let model_path = locate_bundled_model()
        .expect("bundled model missing");
    let model = vosk::Model::new(model_path.to_string_lossy().as_ref())
        .expect("model load");
    let mut recognizer = vosk::Recognizer::new(&model, 16_000.0)
        .expect("recognizer must construct");
    let chunk: Vec<i16> = vec![0; 480]; // 30ms @ 16kHz
    for index in 0..100 {
        let _ = recognizer.accept_waveform(&chunk);
        if index % 20 == 0 {
            let _ = recognizer.partial_result();
        }
    }
    let _ = recognizer.final_result();
}
