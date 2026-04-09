use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level configuration, loaded at startup.
pub struct Config {
    pub base_dir: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub rtsp_url: String,
    pub port: u16,
    pub fps: u32,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub presets: HashMap<String, Preset>,
}

/// A preset definition from presets.json.
#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub ffmpeg_input: serde_json::Value,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub app_args: Vec<String>,
    #[serde(default = "default_startup_delay")]
    pub startup_delay: f64,
    #[serde(default)]
    pub window_title: Option<String>,
    #[serde(default)]
    pub teardown_app: Option<String>,
    #[serde(default)]
    pub teardown_app_args: Vec<String>,
    #[serde(default = "default_teardown_delay")]
    pub teardown_delay: f64,
}

fn default_startup_delay() -> f64 {
    2.0
}
fn default_teardown_delay() -> f64 {
    1.0
}

impl Config {
    pub fn load(base_dir: &Path) -> Result<Self> {
        let presets_path = base_dir.join("presets.json");
        let presets_data = std::fs::read_to_string(&presets_path)
            .with_context(|| format!("failed to read {}", presets_path.display()))?;
        let presets: HashMap<String, Preset> = serde_json::from_str(&presets_data)
            .context("failed to parse presets.json")?;

        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            ffmpeg_path: base_dir.join("ffmpeg/bin/ffmpeg.exe"),
            rtsp_url: "rtsp://localhost:8554/display".into(),
            port: 9090,
            fps: 30,
            canvas_width: 1920,
            canvas_height: 1080,
            presets,
        })
    }

    /// Determine the input sentinel type for a preset.
    pub fn preset_input_type(preset: &Preset) -> PresetInputType {
        if let Some(arr) = preset.ffmpeg_input.as_array() {
            if arr.len() == 1 {
                if let Some(s) = arr[0].as_str() {
                    return match s {
                        "__dxgi__" => PresetInputType::Dxgi,
                        "__image__" => PresetInputType::Image,
                        "__window__" => PresetInputType::Window,
                        _ => PresetInputType::Custom,
                    };
                }
            }
        }
        PresetInputType::Custom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetInputType {
    Dxgi,
    Image,
    Window,
    Custom,
}
