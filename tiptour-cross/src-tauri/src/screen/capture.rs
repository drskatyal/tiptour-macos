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
    use screencapturekit::{
        output::{CMSampleBufferRef, LockTrait},
        shareable_content::SCShareableContent,
        stream::{
            configuration::SCStreamConfiguration,
            content_filter::SCContentFilter,
            output_trait::SCStreamOutputTrait,
            output_type::SCStreamOutputType,
            SCStream,
        },
    };
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Mutex;

    // Discover the main display via SCShareableContent. If this errors out
    // with TCC -3801 the user has not granted Screen Recording yet — we
    // surface that as a clean message so the UI can show "grant screen
    // recording permission" instead of a generic failure.
    let shareable = SCShareableContent::get()
        .map_err(|error| format!("Screen recording permission denied or unavailable: {error:?}"))?;
    let displays = shareable.displays();
    let display = displays
        .first()
        .ok_or_else(|| "No displays available for capture".to_string())?
        .clone();

    let width = display.width() as u32;
    let height = display.height() as u32;

    let filter = SCContentFilter::new().with_display_excluding_windows(&display, &[]);
    let config = SCStreamConfiguration::new()
        .set_width(width as usize)
        .map_err(|error| format!("set_width: {error:?}"))?
        .set_height(height as usize)
        .map_err(|error| format!("set_height: {error:?}"))?
        .set_captures_audio(false)
        .map_err(|error| format!("set_captures_audio: {error:?}"))?;

    struct OneShotOutput {
        sender: Mutex<Option<Sender<Result<RawFrame, String>>>>,
    }
    impl SCStreamOutputTrait for OneShotOutput {
        fn did_output_sample_buffer(
            &self,
            sample_buffer: CMSampleBufferRef,
            of_type: SCStreamOutputType,
        ) {
            if of_type != SCStreamOutputType::Screen {
                return;
            }
            let sender = match self.sender.lock().ok().and_then(|mut g| g.take()) {
                Some(s) => s,
                None => return, // already delivered the first frame
            };
            let result = (|| -> Result<RawFrame, String> {
                let pixel_buffer = sample_buffer
                    .get_pixel_buffer()
                    .map_err(|error| format!("get_pixel_buffer: {error:?}"))?;
                let width = pixel_buffer.get_width() as u32;
                let height = pixel_buffer.get_height() as u32;
                let locked = pixel_buffer
                    .lock()
                    .map_err(|error| format!("pixel_buffer.lock: {error:?}"))?;
                let bytes_per_row = locked.get_bytes_per_row() as usize;
                let data = locked.as_slice();
                let row_bytes = (width as usize) * 4;
                let mut bgra = Vec::with_capacity(row_bytes * height as usize);
                for row in 0..(height as usize) {
                    let start = row * bytes_per_row;
                    bgra.extend_from_slice(&data[start..start + row_bytes]);
                }
                Ok(RawFrame { bgra, width, height })
            })();
            let _ = sender.send(result);
        }
    }

    let (tx, rx) = channel();
    let output = OneShotOutput {
        sender: Mutex::new(Some(tx)),
    };

    let mut stream = SCStream::new(&filter, &config);
    stream
        .add_output_handler(output, SCStreamOutputType::Screen)
        .map_err(|error| format!("add_output_handler: {error:?}"))?;
    stream
        .start_capture()
        .map_err(|error| format!("start_capture: {error:?}"))?;

    // Wait for the first frame on a blocking thread so we don't block the
    // tokio executor. ScreenCaptureKit usually delivers a frame within
    // ~50ms but we cap at 3s in case the user just granted permission and
    // the system is still spinning up.
    let frame_result = tokio::task::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "Timed out waiting for first screen frame".to_string())
    })
    .await
    .map_err(|error| format!("join error: {error}"))?;

    // Stop the stream regardless of whether the frame arrived.
    let _ = stream.stop_capture();

    frame_result?
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
