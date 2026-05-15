// Indicator-pill emit API. Every "noteworthy thing TipTour did" funnels
// through `emit` here, which checks the user's persisted indicator
// settings, applies the per-type enable filter, enforces the
// max-concurrent cap (dropping the oldest payload at the JS layer), and
// fires a single `indicator_event` Tauri event into the indicators
// webview.
//
// Privacy: payloads carry a title + optional subtitle string and a
// kind tag. Nothing else. Never include raw screen pixels, trace
// contents, recognized voice text, AX-tree dumps, or anything else
// that could leak the user's screen or input into a UI surface the
// user might screenshot.

use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::indicators_settings::{load_indicators_settings_from_disk, IndicatorTypeToggles};

const INDICATOR_EVENT_NAME: &str = "indicator_event";

/// Six kinds of indicator. Each maps 1:1 to a CSS class + accent color
/// on the JS side, and to one bool field in `IndicatorTypeToggles`.
#[derive(Debug, Clone, Copy)]
pub enum IndicatorKind {
    WorkflowStep,
    FlowDone,
    VoiceCommand,
    AppLaunched,
    Screenshot,
    Error,
}

impl IndicatorKind {
    fn wire_tag(&self) -> &'static str {
        match self {
            IndicatorKind::WorkflowStep => "step",
            IndicatorKind::FlowDone => "flow",
            IndicatorKind::VoiceCommand => "voice",
            IndicatorKind::AppLaunched => "app",
            IndicatorKind::Screenshot => "screenshot",
            IndicatorKind::Error => "error",
        }
    }

    fn is_enabled_in(&self, type_toggles: &IndicatorTypeToggles) -> bool {
        match self {
            IndicatorKind::WorkflowStep => type_toggles.workflow_step,
            IndicatorKind::FlowDone => type_toggles.flow_done,
            IndicatorKind::VoiceCommand => type_toggles.voice_command,
            IndicatorKind::AppLaunched => type_toggles.app_launched,
            IndicatorKind::Screenshot => type_toggles.screenshot,
            IndicatorKind::Error => type_toggles.error,
        }
    }
}

#[derive(Serialize, Clone)]
struct IndicatorEventPayload {
    kind: &'static str,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sourceId")]
    source_id: Option<String>,
    #[serde(rename = "autoDismissMs")]
    auto_dismiss_milliseconds: u32,
    #[serde(rename = "maxVisible")]
    max_visible: usize,
    density: String,
    sound: String,
}

/// Process-global AppHandle slot so deep callers (recorder screenshot
/// writer, anywhere without an AppHandle in scope) can still emit. Set
/// once at app setup.
static GLOBAL_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn install_global_app_handle(app: AppHandle) {
    let _ = GLOBAL_APP_HANDLE.set(app);
}

/// Direct emit when the caller has an AppHandle in scope. This is the
/// preferred path — the global slot is the fallback for places that
/// don't.
pub fn emit(
    app: &AppHandle,
    kind: IndicatorKind,
    title: String,
    subtitle: Option<String>,
    source_id: Option<String>,
) {
    let settings = load_indicators_settings_from_disk();
    if matches!(
        settings.position,
        crate::indicators_settings::IndicatorPosition::Disabled
    ) {
        return;
    }
    if !kind.is_enabled_in(&settings.types_enabled) {
        return;
    }
    let density_wire = match settings.density {
        crate::indicators_settings::IndicatorDensity::Compact => "compact",
        crate::indicators_settings::IndicatorDensity::Normal => "normal",
        crate::indicators_settings::IndicatorDensity::Verbose => "verbose",
    };
    let sound_wire = match settings.sound {
        crate::indicators_settings::IndicatorSound::Silent => "silent",
        crate::indicators_settings::IndicatorSound::SoftTick => "soft-tick",
    };
    let payload = IndicatorEventPayload {
        kind: kind.wire_tag(),
        title,
        subtitle,
        source_id,
        auto_dismiss_milliseconds: settings.auto_dismiss.timeout_milliseconds(),
        max_visible: settings.max_visible.pill_count(),
        density: density_wire.to_string(),
        sound: sound_wire.to_string(),
    };
    let _ = app.emit(INDICATOR_EVENT_NAME, payload);
}

/// Emit using the process-global AppHandle. Returns silently when the
/// global hasn't been installed yet — that only happens during app
/// boot before `main.rs::setup` ran.
pub fn emit_via_global(
    kind: IndicatorKind,
    title: String,
    subtitle: Option<String>,
    source_id: Option<String>,
) {
    let Some(app) = GLOBAL_APP_HANDLE.get() else {
        return;
    };
    emit(app, kind, title, subtitle, source_id);
}

/// Tauri command — lets the TS layer fire indicators directly (e.g. the
/// frontend `showError` path) without needing every error site to know
/// about a Rust-side emit function.
#[tauri::command]
pub fn emit_indicator_from_frontend(
    app: AppHandle,
    kind: String,
    title: String,
    subtitle: Option<String>,
    source_id: Option<String>,
) -> Result<(), String> {
    let kind_parsed = match kind.as_str() {
        "step" => IndicatorKind::WorkflowStep,
        "flow" => IndicatorKind::FlowDone,
        "voice" => IndicatorKind::VoiceCommand,
        "app" => IndicatorKind::AppLaunched,
        "screenshot" => IndicatorKind::Screenshot,
        "error" => IndicatorKind::Error,
        unknown => return Err(format!("unknown indicator kind '{unknown}'")),
    };
    emit(&app, kind_parsed, title, subtitle, source_id);
    Ok(())
}
