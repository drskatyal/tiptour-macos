// Cross-platform key/mouse capture backed by the `rdev` crate. The capture
// loop runs on a dedicated OS thread because `rdev::listen` blocks for the
// life of the process; we gate the actual emit behind an atomic flag so the
// recorder can start/stop capture cheaply without tearing down the listener.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel, Receiver};
use std::sync::Arc;
use std::thread;

use super::types::InputEvent;

static IS_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct InputCaptureHandle {
    pub event_receiver: Receiver<InputEvent>,
}

pub fn start() -> InputCaptureHandle {
    let (input_event_sender, input_event_receiver) = channel::<InputEvent>();

    // Only spin up the listener thread once per process. Subsequent `start()`
    // calls flip the active flag and reuse the existing OS-level tap.
    let was_active = IS_CAPTURE_ACTIVE.swap(true, Ordering::SeqCst);
    if !was_active {
        spawn_listener_thread(input_event_sender);
    }

    InputCaptureHandle {
        event_receiver: input_event_receiver,
    }
}

pub fn stop() {
    IS_CAPTURE_ACTIVE.store(false, Ordering::SeqCst);
}

fn spawn_listener_thread(input_event_sender: Sender<InputEvent>) {
    let sender_for_callback = Arc::new(input_event_sender);
    thread::spawn(move || {
        let sender_clone = sender_for_callback.clone();
        // rdev::listen blocks; any error here is logged and the thread exits.
        // The recorder treats absence of events as "no input" — it does not
        // crash the app.
        let listen_result = rdev::listen(move |raw_event| {
            if !IS_CAPTURE_ACTIVE.load(Ordering::SeqCst) {
                return;
            }
            if let Some(translated_event) = translate_rdev_event(&raw_event) {
                let _ = sender_clone.send(translated_event);
            }
        });
        if let Err(error) = listen_result {
            eprintln!("recorder input capture listener error: {error:?}");
        }
    });
}

fn translate_rdev_event(raw_event: &rdev::Event) -> Option<InputEvent> {
    match &raw_event.event_type {
        rdev::EventType::KeyPress(key) => Some(InputEvent::KeyDown {
            key_code: key_to_code(key),
            key_name: format!("{key:?}"),
        }),
        rdev::EventType::KeyRelease(key) => Some(InputEvent::KeyUp {
            key_code: key_to_code(key),
            key_name: format!("{key:?}"),
        }),
        rdev::EventType::ButtonPress(button) => {
            // rdev doesn't carry cursor position on button events. The
            // recorder fills coordinates from the last `MouseMove` it saw;
            // callers downstream merge the pair.
            Some(InputEvent::MouseClick {
                button: format!("{button:?}"),
                x: 0.0,
                y: 0.0,
            })
        }
        rdev::EventType::MouseMove { x, y } => Some(InputEvent::MouseMove { x: *x, y: *y }),
        rdev::EventType::Wheel { delta_x, delta_y } => Some(InputEvent::Scroll {
            delta_x: *delta_x as f64,
            delta_y: *delta_y as f64,
        }),
        _ => None,
    }
}

fn key_to_code(key: &rdev::Key) -> u32 {
    // rdev::Key is a non-numeric enum; for the trace we just want a stable
    // identifier per key. The Debug string is already used for `key_name`,
    // so the numeric code is a discriminant-style hash that's stable within
    // a single process. Pattern miner only compares within a single trace.
    let debug_string = format!("{key:?}");
    let mut hash: u32 = 0;
    for byte in debug_string.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }
    hash
}
