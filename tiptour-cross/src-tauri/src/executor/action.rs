// Flat, executor-ready action enum. The workflow runner turns each
// `WorkflowStep` into one of these variants after grounding, and the
// `cross_platform_input` layer delivers it via enigo/arboard.
//
// Mirrors the action surface in `TipTour/ActionExecutor.swift` (left/right/
// double click, keyboard shortcut, type, set-selected-text, scroll, open).
// We keep the shape flat so the runner can pattern-match exhaustively and
// new variants surface as compile errors instead of silent no-ops.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone)]
pub enum ExecutableAction {
    LaunchApp {
        /// Bundle identifier (`com.apple.Safari`) on macOS or path/exe
        /// name (`notepad.exe`) on Windows. The launcher chooses the
        /// right strategy from the shape of the string.
        bundle_id_or_exe: String,
    },
    OpenUrl {
        url: String,
    },
    Click {
        x: f64,
        y: f64,
        button: MouseButton,
    },
    DoubleClick {
        x: f64,
        y: f64,
    },
    RightClick {
        x: f64,
        y: f64,
    },
    /// Modifier-and-key chord, e.g. `["Cmd","S"]` or `["Ctrl","Shift","P"]`.
    /// Cross-platform input routes `Cmd` -> `Ctrl` on Windows automatically.
    KeyboardShortcut {
        keys: Vec<String>,
    },
    Type {
        text: String,
        /// Whether to bias toward AX/UIA selected-text insertion before
        /// falling back to clipboard-staged paste. The Swift app uses
        /// this distinction when the focused control accepts direct AX
        /// value writes (most Cocoa text fields) vs rich web editors
        /// (Google Docs) that require paste.
        into_focused: bool,
    },
    /// Replace the user's currently-highlighted text. The runner is
    /// expected to apply the armed selection range first (mac AX) and
    /// only then paste; if the range can't be restored, the executor
    /// should fail loudly rather than typing into the wrong field.
    SetSelectedText {
        text: String,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
}
