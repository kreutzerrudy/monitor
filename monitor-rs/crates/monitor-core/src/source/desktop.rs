use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::frame::Frame;
use super::{SourceMetadata, SourceType, StreamSource};

/// Captures a display via FFmpeg ddagrab (or gdigrab fallback).
/// Spawns an FFmpeg subprocess that outputs raw BGRA frames to stdout;
/// a reader thread continuously reads frames into a shared buffer.
pub struct DesktopCapture {
    ffmpeg_path: PathBuf,
    output_idx: u32,
    width: u32,
    height: u32,
    fps: u32,
    use_ddagrab: bool,
    // For gdigrab fallback:
    offset_x: i32,
    offset_y: i32,
    process: Option<Child>,
    reader_handle: Option<JoinHandle<()>>,
    current_frame: Arc<Mutex<Option<Frame>>>,
    running: Arc<AtomicBool>,
    metadata: SourceMetadata,
}

impl DesktopCapture {
    pub fn new(
        ffmpeg_path: PathBuf,
        output_idx: u32,
        width: u32,
        height: u32,
        fps: u32,
        use_ddagrab: bool,
        offset_x: i32,
        offset_y: i32,
        name: String,
    ) -> Self {
        Self {
            ffmpeg_path,
            output_idx,
            width,
            height,
            fps,
            use_ddagrab,
            offset_x,
            offset_y,
            process: None,
            reader_handle: None,
            current_frame: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            metadata: SourceMetadata {
                name,
                source_type: SourceType::Desktop,
                native_width: width,
                native_height: height,
            },
        }
    }

    fn build_ffmpeg_args(&self) -> Vec<String> {
        if self.use_ddagrab {
            vec![
                "-f".into(), "lavfi".into(),
                "-i".into(),
                format!(
                    "ddagrab=output_idx={}:framerate={}:draw_mouse=0",
                    self.output_idx, self.fps
                ),
                "-vf".into(), "hwdownload,format=bgra".into(),
                "-f".into(), "rawvideo".into(),
                "-pix_fmt".into(), "bgra".into(),
                "pipe:1".into(),
            ]
        } else {
            vec![
                "-f".into(), "gdigrab".into(),
                "-framerate".into(), self.fps.to_string(),
                "-offset_x".into(), self.offset_x.to_string(),
                "-offset_y".into(), self.offset_y.to_string(),
                "-video_size".into(), format!("{}x{}", self.width, self.height),
                "-i".into(), "desktop".into(),
                "-vf".into(), "format=bgra".into(),
                "-f".into(), "rawvideo".into(),
                "-pix_fmt".into(), "bgra".into(),
                "pipe:1".into(),
            ]
        }
    }
}

impl StreamSource for DesktopCapture {
    fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            return Ok(());
        }

        let args = self.build_ffmpeg_args();
        info!("desktop capture starting: {} {}", self.ffmpeg_path.display(), args.join(" "));

        let mut child = Command::new(&self.ffmpeg_path)
            .arg("-y")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn desktop capture FFmpeg")?;

        let stdout = child.stdout.take().context("no stdout from FFmpeg")?;
        self.process = Some(child);
        self.running.store(true, Ordering::Relaxed);

        let frame_buf = self.current_frame.clone();
        let running = self.running.clone();
        let w = self.width;
        let h = self.height;
        let frame_size = (w * h * 4) as usize;

        let handle = thread::Builder::new()
            .name("desktop-reader".into())
            .spawn(move || {
                reader_loop(stdout, frame_buf, running, w, h, frame_size);
            })
            .context("failed to spawn reader thread")?;

        self.reader_handle = Some(handle);
        info!("desktop capture started ({}x{}, idx={})", self.width, self.height, self.output_idx);
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
        info!("desktop capture stopped");
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

impl Drop for DesktopCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Continuously reads fixed-size raw frames from an FFmpeg stdout pipe.
fn reader_loop(
    mut stdout: impl Read,
    frame_buf: Arc<Mutex<Option<Frame>>>,
    running: Arc<AtomicBool>,
    width: u32,
    height: u32,
    frame_size: usize,
) {
    let mut buf = vec![0u8; frame_size];
    while running.load(Ordering::Relaxed) {
        match read_exact_or_eof(&mut stdout, &mut buf) {
            Ok(true) => {
                let frame = Frame {
                    data: buf.clone(),
                    width,
                    height,
                };
                *frame_buf.lock().unwrap() = Some(frame);
            }
            Ok(false) => {
                // EOF — FFmpeg exited.
                break;
            }
            Err(e) => {
                warn!("desktop reader error: {e}");
                break;
            }
        }
    }
    running.store(false, Ordering::Relaxed);
}

/// Read exactly `buf.len()` bytes. Returns `Ok(false)` on EOF.
pub fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return Ok(false),
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}
