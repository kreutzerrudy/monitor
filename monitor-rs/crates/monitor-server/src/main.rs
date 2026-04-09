mod api;
mod config;
mod state;

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock, atomic::AtomicBool};

use axum::Router;
use axum::routing::{delete, get, patch, post};
use tracing::info;

use monitor_core::canvas::Canvas;
use monitor_core::compositor;
use monitor_core::encoder::{EncoderPipeline, spawn_encoder_watchdog};
use monitor_core::scene::Scene;
use monitor_core::source::StreamSource;
use monitor_core::source::image::ImageSource;
use monitor_platform::display::MonitorRegistry;
use monitor_platform::process;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let base = Path::new("C:/monitor");
    let config = Config::load(base)?;

    // Ensure runtime directories exist.
    std::fs::create_dir_all(base.join("logs"))?;
    std::fs::create_dir_all(base.join("media"))?;

    // Kill stale FFmpeg processes from previous runs.
    let killed = process::kill_stale("ffmpeg.exe");
    if killed > 0 {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // Build monitor registry.
    info!("building monitor registry...");
    let registry = MonitorRegistry::new();
    registry.refresh()?;

    let vmon = registry.get_virtual();
    if let Some(ref m) = vmon {
        info!("virtual monitor: {m}");
    } else {
        tracing::warn!("no virtual display found — desktop capture will require explicit monitor key");
    }

    // Start encoder pipeline.
    let mut encoder = EncoderPipeline::new(
        config.ffmpeg_path.clone(),
        config.rtsp_url.clone(),
        config.canvas_width,
        config.canvas_height,
        config.fps,
    );
    encoder.start()?;

    let canvas = Arc::new(RwLock::new(Canvas::new(
        config.canvas_width,
        config.canvas_height,
        config.fps,
    )));
    let scene = Arc::new(RwLock::new(Scene::new()));
    let encoder = Arc::new(Mutex::new(encoder));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Spawn encoder watchdog — restarts encoder on unexpected exit.
    spawn_encoder_watchdog(encoder.clone(), shutdown.clone());

    let port = config.port;

    let shared = Arc::new(AppState {
        scene: scene.clone(),
        canvas: canvas.clone(),
        encoder: encoder.clone(),
        monitor_registry: registry,
        config,
        shutdown: shutdown.clone(),
    });

    // Add a default black image layer so the stream has content immediately.
    {
        let mut s = scene.write().unwrap();
        let black = ImageSource::black(shared.config.canvas_width, shared.config.canvas_height);
        s.add(Box::new(black), 0, 0, 0);
    }

    // Spawn compositor on a dedicated OS thread.
    std::thread::Builder::new()
        .name("compositor".into())
        .spawn(move || {
            compositor::compositor_loop(canvas, scene, encoder, shutdown);
        })?;

    // Build HTTP API.
    let app = Router::new()
        .route("/status", get(api::get_status))
        .route("/monitors", get(api::get_monitors))
        .route("/presets", get(api::get_presets))
        .route("/layers", post(api::add_layer))
        .route("/layers/{id}", delete(api::remove_layer))
        .route("/layers/{id}", patch(api::update_layer))
        .route("/switch", get(api::switch_preset).post(api::switch_preset))
        .route("/canvas", get(api::get_canvas).post(api::set_canvas))
        .with_state(shared);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("control API listening on :{port}");
    axum::serve(listener, app).await?;

    Ok(())
}
