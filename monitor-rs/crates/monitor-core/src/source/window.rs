use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::frame::Frame;
use super::{SourceMetadata, SourceType, StreamSource};

/// Captures a specific window by title via FFmpeg gdigrab.
pub struct WindowCapture {
    ffmpeg_path: PathBuf,
    window_title: String,
    width: u32,
    height: u32,
    fps: u32,
    process: Option<Child>,
    reader_handle: Option<JoinHandle<()>>,
    current_frame: Arc<Mutex<Option<Frame>>>,
    running: Arc<AtomicBool>,
    metadata: SourceMetadata,
}

impl WindowCapture {
    pub fn new(
        ffmpeg_path: PathBuf,
        window_title: String,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Self {
        Self {
            ffmpeg_path,
            window_title: window_title.clone(),
            width,
            height,
            fps,
            process: None,
            reader_handle: None,
            current_frame: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            metadata: SourceMetadata {
                name: format!("window:{window_title}"),
                source_type: SourceType::Window,
                native_width: width,
                native_height: height,
            },
        }
    }
}

impl StreamSource for WindowCapture {
    fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            return Ok(());
        }

        // gdigrab window capture. The scale filter ensures even dimensions.
        let mut child = Command::new(&self.ffmpeg_path)
            .args([
                "-y",
                "-f", "gdigrab",
                "-framerate", &self.fps.to_string(),
                "-i", &format!("title={}", self.window_title),
                "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=bgra",
                "-f", "rawvideo",
                "-pix_fmt", "bgra",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn window capture FFmpeg")?;

        let stdout = child.stdout.take().context("no stdout")?;
        self.process = Some(child);
        self.running.store(true, Ordering::Relaxed);

        let frame_buf = self.current_frame.clone();
        let running = self.running.clone();
        let w = self.width;
        let h = self.height;
        let frame_size = (w * h * 4) as usize;

        let handle = thread::Builder::new()
            .name("window-reader".into())
            .spawn(move || {
                let mut stdout = stdout;
                let mut buf = vec![0u8; frame_size];
                while running.load(Ordering::Relaxed) {
                    match super::desktop::read_exact_or_eof(&mut stdout, &mut buf) {
                        Ok(true) => {
                            *frame_buf.lock().unwrap() = Some(Frame {
                                data: buf.clone(),
                                width: w,
                                height: h,
                            });
                        }
                        _ => break,
                    }
                }
                running.store(false, Ordering::Relaxed);
            })
            .context("failed to spawn window reader thread")?;

        self.reader_handle = Some(handle);
        info!("window capture started: '{}'", self.window_title);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(h) = self.reader_handle.take() {
            let _ = h.join();
        }
        *self.current_frame.lock().unwrap() = None;
        info!("window capture stopped: '{}'", self.window_title);
        Ok(())
    }

    fn frame(&self) -> Option<Frame> {
        self.current_frame.lock().unwrap().clone()
    }

    fn metadata(&self) -> &SourceMetadata {
        &self.metadata
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
