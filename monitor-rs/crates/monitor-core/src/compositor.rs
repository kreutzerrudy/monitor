use std::io::Write;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::canvas::Canvas;
use crate::encoder::EncoderPipeline;
use crate::frame::Frame;
use crate::scene::Scene;

/// Runs on a dedicated OS thread at the canvas FPS.
/// Each tick: read-lock Scene, composite layers onto a canvas, write to encoder.
pub fn compositor_loop(
    canvas: Arc<RwLock<Canvas>>,
    scene: Arc<RwLock<Scene>>,
    encoder: Arc<std::sync::Mutex<EncoderPipeline>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) {
    tracing::info!("compositor thread started");

    loop {
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let tick_start = Instant::now();

        let (target_fps, mut frame) = {
            let c = canvas.read().unwrap();
            (c.fps, c.blank_frame())
        };

        let frame_duration = Duration::from_secs_f64(1.0 / target_fps as f64);

        // Composite all visible layers onto the canvas frame.
        {
            let scene = scene.read().unwrap();
            for layer in scene.iter() {
                if !layer.visible {
                    continue;
                }
                if let Some(src_frame) = layer.source.frame() {
                    frame.blit(&src_frame, layer.x, layer.y);
                }
            }
        }

        // Write composited frame to encoder.
        {
            let mut enc = encoder.lock().unwrap();
            if let Err(e) = enc.write_frame(&frame) {
                warn!("encoder write failed: {e}");
            }
        }

        let elapsed = tick_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        } else {
            debug!("compositor overran by {:?}", elapsed - frame_duration);
        }
    }

    tracing::info!("compositor thread stopped");
}
