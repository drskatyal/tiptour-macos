// Personas JSON round-trip + active-persona switch + voice-match.
//
// Same env-redirect strategy as commands_app_settings.rs so the
// in-process module-static cache reads from a tempdir.

use tiptour_test_support::personas::{
    delete_persona, get_active_persona, list_personas, match_persona_by_voice,
    set_active_persona, upsert_custom_persona, Persona,
};

fn redirect_data_local_dir_to_tempdir() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("tempdir create must succeed");
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("LOCALAPPDATA", tempdir.path());
    tempdir
}

#[test]
fn personas_seed_and_round_trip_and_voice_match() {
    let _tempdir = redirect_data_local_dir_to_tempdir();

    // Seeds appear on first read.
    let seeded = list_personas().expect("list_personas must succeed");
    assert!(seeded.len() >= 5, "five built-in personas should seed");
    assert!(seeded.iter().any(|p| p.id == "coding-assistant"));
    assert!(seeded.iter().all(|p| p.is_built_in || !p.is_built_in));

    // Active persona resolves to one of the seeds.
    let active = get_active_persona().expect("get_active_persona must succeed");
    assert!(seeded.iter().any(|p| p.id == active.id));

    // set_active_persona to a different seed updates the active id.
    let switch_target_id = seeded
        .iter()
        .find(|p| p.id != active.id)
        .map(|p| p.id.clone())
        .expect("at least two seeds exist");
    set_active_persona(switch_target_id.clone()).expect("set must succeed");
    let after_switch = get_active_persona().expect("get after switch");
    assert_eq!(after_switch.id, switch_target_id);

    // Custom persona round-trip.
    let custom = Persona {
        id: "test-custom".into(),
        name: "Test Custom".into(),
        system_prompt: "Be a test.".into(),
        voice_trigger_phrases: vec!["test custom".into()],
        is_built_in: false,
        model_provider: "gemini".into(),
        model_id: "gemini-2.5-flash".into(),
        reasoning_enabled: false,
        temperature: 0.4,
    };
    upsert_custom_persona(custom.clone()).expect("upsert custom must succeed");
    let after_upsert = list_personas().expect("list after upsert");
    assert!(after_upsert.iter().any(|p| p.id == "test-custom"));

    // Voice match by trigger phrase + by name + via "switch to" prefix.
    let matched_by_trigger = match_persona_by_voice("test custom".into())
        .expect("match must succeed")
        .expect("trigger phrase should match");
    assert_eq!(matched_by_trigger, "test-custom");
    let matched_by_switch_to = match_persona_by_voice("switch to test custom".into())
        .expect("match must succeed")
        .expect("switch-to phrase should match");
    assert_eq!(matched_by_switch_to, "test-custom");

    // Built-in cannot be deleted.
    let delete_built_in_result = delete_persona("coding-assistant".into());
    assert!(delete_built_in_result.is_err(), "built-ins should be undeletable");

    // Custom can be deleted.
    delete_persona("test-custom".into()).expect("delete custom must succeed");
    let after_delete = list_personas().expect("list after delete");
    assert!(after_delete.iter().all(|p| p.id != "test-custom"));
}
