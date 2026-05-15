// Cross-platform single-frame screen capture.
//
// `capture_primary_screen` always returns a `RawFrame` in BGRA8 layout
// (b, g, r, a per pixel, row-major, no padding). Each backend converts
// its native pixel format into BGRA so the downstream JPEG encoder and
// dHash code don't need to know which OS produced the frame.

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "macos")]
pub async fn capture_primary_screen() -> Result<RawFrame, String> {
    // We use `CGDisplay::create_image()` rather than ScreenCaptureKit
    // because the Rust SCK bindings churn faster than we can chase. Apple
    // formally deprecated CGDisplayCreateImage in macOS 14 but it still
    // works through macOS 15 (the runtime warning is suppressible), and
    // the 1.5-second cadence of our screenshot streamer doesn't need
    // SCK's higher-throughput pixel-buffer pipeline. When the Rust SCK
    // bindings stabilize we'll migrate, but for now CGDisplay buys us a
    // dependency-free path that compiles cleanly against core-graphics
    // 0.25 with no version conflicts. The TCC permission check fires at
    // the moment we call this — if the user hasn't granted Screen
    // Recording, CGDisplay::create_image returns None and we surface
    // that as a clean permission-denied error.
    tokio::task::spawn_blocking(|| {
        use core_graphics::display::CGDisplay;

        let main_display = CGDisplay::main();
        let cg_image = main_display.image().ok_or_else(|| {
            "Screen recording permission denied — grant it in System Settings → Privacy & Security → Screen Recording, then relaunch TipTour".to_string()
        })?;

        let width = cg_image.width() as u32;
        let height = cg_image.height() as u32;
        let bytes_per_row = cg_image.bytes_per_row() as usize;
        let raw_data = cg_image.data();
        let raw_bytes: &[u8] = raw_data.bytes();

        // CGImage's row stride may exceed `width * 4` because Quartz aligns
        // rows to multiples of 16 or 64 bytes. We strip the padding by
        // copying `width * 4` bytes per row into a tightly-packed buffer.
        let tight_row_bytes = (width as usize) * 4;
        let mut bgra = Vec::with_capacity(tight_row_bytes * height as usize);
        for row in 0..(height as usize) {
            let row_start = row * bytes_per_row;
            let row_end = row_start + tight_row_bytes;
            if row_end > raw_bytes.len() {
                return Err(format!(
                    "row {row} extends past CGImage data ({}..{} > {})",
                    row_start, row_end, raw_bytes.len()
                ));
            }
            bgra.extend_from_slice(&raw_bytes[row_start..row_end]);
        }

        Ok::<RawFrame, String>(RawFrame { bgra, width, height })
    })
    .await
    .map_err(|error| format!("screen capture join error: {error}"))?
}

#[cfg(target_os = "windows")]
pub async fn capture_primary_screen() -> Result<RawFrame, String> {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Mutex;
    use windows_capture::{
        capture::{Context, GraphicsCaptureApiHandler},
        frame::Frame,
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{ColorFormat, CursorCaptureSettings, DrawBorderSettings, Settings},
    };

    // The `windows-capture` crate doesn't give the handler constructor a
    // clean way to receive a sender, so we stash the sender in a static
    // slot and let the handler pick it up via `PENDING_SENDER`.
    static PENDING_SENDER: once_cell::sync::Lazy<
        Mutex<Option<Sender<Result<RawFrame, String>>>>,
    > = once_cell::sync::Lazy::new(|| Mutex::new(None));

    struct Handler;
    impl GraphicsCaptureApiHandler for Handler {
        type Flags = ();
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(_: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Handler)
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let width = frame.width();
            let height = frame.height();
            let result = (|| -> Result<RawFrame, String> {
                let mut buffer = frame
                    .buffer()
                    .map_err(|error| format!("frame.buffer: {error:?}"))?;
                let raw = buffer.as_raw_buffer();
                Ok(RawFrame {
                    bgra: raw.to_vec(),
                    width,
                    height,
                })
            })();
            if let Some(sender) = PENDING_SENDER.lock().ok().and_then(|mut g| g.take()) {
                let _ = sender.send(result);
            }
            control.stop();
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    let (tx, rx) = channel();
    {
        let mut slot = PENDING_SENDER
            .lock()
            .map_err(|error| format!("sender lock poisoned: {error}"))?;
        *slot = Some(tx);
    }

    let monitor = Monitor::primary()
        .map_err(|error| format!("Monitor::primary: {error:?}"))?;
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        ColorFormat::Bgra8,
        (),
    );

    let frame_result = tokio::task::spawn_blocking(move || -> Result<RawFrame, String> {
        // start() blocks the calling thread until the handler calls stop(),
        // which our handler does after the first frame.
        Handler::start(settings)
            .map_err(|error| format!("Handler::start: {error:?}"))?;
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "Timed out waiting for first screen frame".to_string())?
    })
    .await
    .map_err(|error| format!("join error: {error}"))?;

    frame_result
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub async fn capture_primary_screen() -> Result<RawFrame, String> {
    Err("Screen capture is only supported on macOS and Windows".to_string())
}
