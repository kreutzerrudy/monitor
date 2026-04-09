use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::frame::Frame;

const MAX_RETRIES: u32 = 5;
const RETRY_DELAY_SECS: u64 = 3;
/// A run lasting longer than this resets the retry counter.
const STABLE_RUN_SECS: u64 = 30;

/// Manages a long-lived FFmpeg process that accepts raw BGRA frames on stdin
/// and encodes + pushes RTSP to MediaMTX.
pub struct EncoderPipeline {
    ffmpeg_path: PathBuf,
    rtsp_url: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    process: Option<Child>,
}

impl EncoderPipeline {
    pub fn new(ffmpeg_path: PathBuf, rtsp_url: String, width: u32, height: u32, fps: u32) -> Self {
        Self {
            ffmpeg_path,
            rtsp_url,
            width,
            height,
            fps,
            process: None,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let child = self.spawn_process()?;
        self.process = Some(child);
        Ok(())
    }

    fn spawn_process(&self) -> Result<Child> {
        let gop = self.fps.max(2);
        let child = Command::new(&self.ffmpeg_path)
            .args([
                "-y",
                "-f", "rawvideo",
                "-pix_fmt", "bgra",
                "-s", &format!("{}x{}", self.width, self.height),
                "-r", &self.fps.to_string(),
                "-i", "pipe:0",
                "-vf", "format=yuv420p",
                "-vcodec", "libx264",
                "-preset", "ultrafast",
                "-tune", "zerolatency",
                "-b:v", "4M",
                "-g", &gop.to_string(),
                "-keyint_min", &gop.to_string(),
                "-fflags", "nobuffer",
                "-f", "rtsp",
                "-rtsp_transport", "tcp",
                &self.rtsp_url,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn encoder FFmpeg")?;

        info!(
            "encoder started (pid {}, {}x{} @{}fps → {})",
            child.id(),
            self.width,
            self.height,
            self.fps,
            self.rtsp_url
        );
        Ok(child)
    }

    pub fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        let child = self.process.as_mut().context("encoder not started")?;
        let stdin = child.stdin.as_mut().context("encoder stdin closed")?;
        stdin
            .write_all(&frame.data)
            .context("failed to write frame to encoder")?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.process.take() {
            drop(child.stdin.take());
            match child.wait() {
                Ok(status) => info!("encoder exited: {status}"),
                Err(e) => warn!("encoder wait error: {e}"),
            }
        }
        Ok(())
    }

    /// Restart the encoder process (e.g. after MediaMTX reconnect).
    pub fn restart(&mut self) -> Result<()> {
        let _ = self.stop();
        thread::sleep(Duration::from_secs(RETRY_DELAY_SECS));
        let child = self.spawn_process()?;
        self.process = Some(child);
        Ok(())
    }

    pub fn is_running(&mut self) -> bool {
        match &mut self.process {
            None => false,
            Some(child) => matches!(child.try_wait(), Ok(None)),
        }
    }

    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Drop for EncoderPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Spawn a watchdog thread that monitors the encoder and restarts it on
/// unexpected exit. Runs until `shutdown` is set.
pub fn spawn_encoder_watchdog(
    encoder: Arc<Mutex<EncoderPipeline>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) {
    thread::Builder::new()
        .name("encoder-watchdog".into())
        .spawn(move || {
            encoder_watchdog_loop(encoder, shutdown);
        })
        .expect("failed to spawn encoder watchdog thread");
}

fn encoder_watchdog_loop(
    encoder: Arc<Mutex<EncoderPipeline>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) {
    info!("encoder watchdog started");
    let mut consecutive_failures: u32 = 0;
    let mut started_at = std::time::Instant::now();

    loop {
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Poll every second whether the encoder is still alive.
        thread::sleep(Duration::from_secs(1));

        let alive = encoder.lock().unwrap().is_running();
        if alive {
            // Reset failure counter after a stable run.
            if started_at.elapsed().as_secs() > STABLE_RUN_SECS {
                consecutive_failures = 0;
            }
            continue;
        }

        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let ran_for = started_at.elapsed().as_secs();
        if ran_for > STABLE_RUN_SECS {
            consecutive_failures = 0;
        }
        consecutive_failures += 1;

        if consecutive_failures > MAX_RETRIES {
            warn!(
                "encoder failed {} times in a row — giving up",
                MAX_RETRIES
            );
            break;
        }

        warn!(
            "encoder stopped unexpectedly (ran {}s) — restarting in {}s [{}/{}]",
            ran_for, RETRY_DELAY_SECS, consecutive_failures, MAX_RETRIES
        );

        match encoder.lock().unwrap().restart() {
            Ok(()) => {
                started_at = std::time::Instant::now();
                info!("encoder restarted successfully");
            }
            Err(e) => {
                warn!("encoder restart failed: {e}");
            }
        }
    }

    info!("encoder watchdog stopped");
}
