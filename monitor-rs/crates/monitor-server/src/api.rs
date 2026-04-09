use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use monitor_core::source::StreamSource;
use monitor_core::source::desktop::DesktopCapture;
use monitor_core::source::image::ImageSource;
use monitor_core::source::window::WindowCapture;

use crate::config::{Config, PresetInputType};
use crate::state::AppState;

// ---------- Response types ----------

#[derive(Serialize)]
pub struct StatusResponse {
    pub canvas: CanvasInfo,
    pub layers: Vec<LayerInfo>,
}

#[derive(Serialize)]
pub struct CanvasInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Serialize)]
pub struct LayerInfo {
    pub id: u64,
    pub z_index: i32,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub source_type: String,
    pub source_name: String,
    pub running: bool,
}

#[derive(Serialize)]
pub struct MonitorInfoResponse {
    pub key: String,
    pub index: u32,
    pub resolution: String,
    pub is_virtual: bool,
}

// ---------- Request types ----------

#[derive(Deserialize)]
pub struct AddLayerRequest {
    pub source_type: String,
    #[serde(default)]
    pub monitor_key: Option<String>,
    #[serde(default)]
    pub z_index: i32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default)]
    pub window_title: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Deserialize)]
pub struct UpdateLayerRequest {
    #[serde(default)]
    pub z_index: Option<i32>,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub visible: Option<bool>,
}

#[derive(Deserialize)]
pub struct SwitchQuery {
    pub preset: Option<String>,
    pub monitor: Option<String>,
}

#[derive(Deserialize)]
pub struct CanvasRequest {
    pub width: u32,
    pub height: u32,
}

// ---------- Helpers ----------

fn err_json(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({"error": msg}))).into_response()
}

fn build_source(
    state: &AppState,
    req: &AddLayerRequest,
) -> Result<Box<dyn StreamSource>, Response> {
    let ffmpeg = state.config.ffmpeg_path.clone();

    match req.source_type.as_str() {
        "desktop" => {
            let monitors = state.monitor_registry.get_all();
            let monitor = if let Some(key) = &req.monitor_key {
                monitors
                    .get(key)
                    .cloned()
                    .ok_or_else(|| err_json(StatusCode::BAD_REQUEST, &format!("unknown monitor: {key}")))?
            } else {
                state
                    .monitor_registry
                    .get_virtual()
                    .ok_or_else(|| err_json(StatusCode::BAD_REQUEST, "no virtual display found"))?
            };

            let mut src = DesktopCapture::new(
                ffmpeg,
                monitor.index,
                monitor.w,
                monitor.h,
                state.config.fps,
                true, // TODO: probe ddagrab at startup
                monitor.x,
                monitor.y,
                monitor.key.clone(),
            );
            src.start()
                .map_err(|e| err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            Ok(Box::new(src))
        }
        "image" => {
            let src = if let Some(path) = &req.image_path {
                ImageSource::from_file(path)
                    .map_err(|e| err_json(StatusCode::BAD_REQUEST, &e.to_string()))?
            } else {
                let w = req.width.unwrap_or(state.config.canvas_width);
                let h = req.height.unwrap_or(state.config.canvas_height);
                ImageSource::black(w, h)
            };
            Ok(Box::new(src))
        }
        "window" => {
            let title = req
                .window_title
                .as_ref()
                .ok_or_else(|| err_json(StatusCode::BAD_REQUEST, "window_title required"))?;
            let w = req.width.unwrap_or(state.config.canvas_width);
            let h = req.height.unwrap_or(state.config.canvas_height);
            let mut src = WindowCapture::new(ffmpeg, title.clone(), w, h, state.config.fps);
            src.start()
                .map_err(|e| err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            Ok(Box::new(src))
        }
        other => Err(err_json(
            StatusCode::BAD_REQUEST,
            &format!("unknown source_type: {other}"),
        )),
    }
}

// ---------- Handlers ----------

pub async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    let canvas = state.canvas.read().unwrap();
    let scene = state.scene.read().unwrap();

    let layers: Vec<LayerInfo> = scene
        .iter()
        .map(|l| {
            let meta = l.source.metadata();
            LayerInfo {
                id: l.id,
                z_index: l.z_index,
                x: l.x,
                y: l.y,
                visible: l.visible,
                source_type: format!("{:?}", meta.source_type),
                source_name: meta.name.clone(),
                running: l.source.is_running(),
            }
        })
        .collect();

    Json(StatusResponse {
        canvas: CanvasInfo {
            width: canvas.width,
            height: canvas.height,
            fps: canvas.fps,
        },
        layers,
    })
    .into_response()
}

pub async fn get_monitors(State(state): State<Arc<AppState>>) -> Response {
    let monitors = match state.monitor_registry.refresh() {
        Ok(m) => m,
        Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let data: Vec<MonitorInfoResponse> = monitors
        .values()
        .map(|m| MonitorInfoResponse {
            key: m.key.clone(),
            index: m.index,
            resolution: format!("{}x{}", m.w, m.h),
            is_virtual: m.is_virtual,
        })
        .collect();

    Json(data).into_response()
}

pub async fn get_presets(State(state): State<Arc<AppState>>) -> Response {
    let names: Vec<&String> = state.config.presets.keys().collect();
    Json(names).into_response()
}

pub async fn add_layer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddLayerRequest>,
) -> Response {
    let source = match build_source(&state, &req) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let id = state.scene.write().unwrap().add(source, req.z_index, req.x, req.y);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id}))).into_response()
}

pub async fn remove_layer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Response {
    let mut scene = state.scene.write().unwrap();
    if let Some(mut layer) = scene.remove(id) {
        let _ = layer.source.stop();
        Json(serde_json::json!({"removed": id})).into_response()
    } else {
        err_json(StatusCode::NOT_FOUND, &format!("layer {id} not found"))
    }
}

pub async fn update_layer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
    Json(req): Json<UpdateLayerRequest>,
) -> Response {
    let mut scene = state.scene.write().unwrap();
    let mut found = false;

    if let Some(z) = req.z_index {
        found = scene.reorder(id, z) || found;
    }
    if let (Some(x), Some(y)) = (req.x, req.y) {
        found = scene.set_position(id, x, y) || found;
    }
    if let Some(vis) = req.visible {
        found = scene.set_visible(id, vis) || found;
    }

    if found {
        Json(serde_json::json!({"updated": id})).into_response()
    } else {
        err_json(StatusCode::NOT_FOUND, &format!("layer {id} not found"))
    }
}

/// Legacy compat: clear all layers and apply a preset definition as layers.
pub async fn switch_preset(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SwitchQuery>,
) -> Response {
    let preset_name = match &q.preset {
        Some(n) => n.clone(),
        None => return err_json(StatusCode::BAD_REQUEST, "missing ?preset="),
    };

    let preset = match state.config.presets.get(&preset_name) {
        Some(p) => p.clone(),
        None => return err_json(StatusCode::BAD_REQUEST, &format!("unknown preset: {preset_name}")),
    };

    // Clear existing layers.
    {
        let mut scene = state.scene.write().unwrap();
        for mut layer in scene.clear() {
            let _ = layer.source.stop();
        }
    }

    let input_type = Config::preset_input_type(&preset);
    let monitor_key = q.monitor.clone();

    let req = match input_type {
        PresetInputType::Dxgi => AddLayerRequest {
            source_type: "desktop".into(),
            monitor_key,
            z_index: 0,
            x: 0,
            y: 0,
            image_path: None,
            window_title: None,
            width: None,
            height: None,
        },
        PresetInputType::Image => AddLayerRequest {
            source_type: "image".into(),
            monitor_key: None,
            z_index: 0,
            x: 0,
            y: 0,
            image_path: None,
            window_title: None,
            width: None,
            height: None,
        },
        PresetInputType::Window => AddLayerRequest {
            source_type: "window".into(),
            monitor_key: None,
            z_index: 0,
            x: 0,
            y: 0,
            image_path: None,
            window_title: preset.window_title.clone(),
            width: None,
            height: None,
        },
        PresetInputType::Custom => {
            return err_json(StatusCode::NOT_IMPLEMENTED, "custom presets not yet supported");
        }
    };

    let source = match build_source(&state, &req) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let id = state.scene.write().unwrap().add(source, req.z_index, req.x, req.y);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id, "preset": preset_name}))).into_response()
}

pub async fn get_canvas(State(state): State<Arc<AppState>>) -> Response {
    let c = state.canvas.read().unwrap();
    Json(CanvasInfo {
        width: c.width,
        height: c.height,
        fps: c.fps,
    })
    .into_response()
}

pub async fn set_canvas(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CanvasRequest>,
) -> Response {
    {
        let mut c = state.canvas.write().unwrap();
        c.width = req.width;
        c.height = req.height;
    }

    // Restart encoder with new resolution.
    {
        let mut enc = state.encoder.lock().unwrap();
        let _ = enc.stop();
        // TODO: recreate encoder with new dimensions
    }

    Json(serde_json::json!({"canvas": {"width": req.width, "height": req.height}})).into_response()
}
