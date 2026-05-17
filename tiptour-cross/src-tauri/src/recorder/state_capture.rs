// Thin trait boundary between the recorder and the real UIA/AX
// grounding layer. The recorder doesn't itself walk accessibility trees —
// it asks a `StateProvider` for a `StateSnapshot` on every input event so
// that the same code paths can be exercised with a stub provider in tests
// and with the real grounding-layer provider in production.

use super::types::StateSnapshot;

pub trait StateProvider: Send + Sync {
    fn current_snapshot(&self) -> Option<StateSnapshot>;
}

// Default provider used until the grounding layer wires in a real one.
// Returns `None` so the recorder writes empty `snapshot_before`/`snapshot_after`
// fields — the trace is still useful for pattern mining on key/mouse events
// alone, and the privacy gate falls through to "don't pause" (which is safe
// because passive recording is opt-in and the user can revoke at any time).
pub struct NullStateProvider;

impl StateProvider for NullStateProvider {
    fn current_snapshot(&self) -> Option<StateSnapshot> {
        None
    }
}
