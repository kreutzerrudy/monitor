use std::sync::{Arc, Mutex, RwLock, atomic::AtomicBool};

use monitor_core::canvas::Canvas;
use monitor_core::encoder::EncoderPipeline;
use monitor_core::scene::Scene;
use monitor_platform::display::MonitorRegistry;

use crate::config::Config;

pub struct AppState {
    pub scene: Arc<RwLock<Scene>>,
    pub canvas: Arc<RwLock<Canvas>>,
    pub encoder: Arc<Mutex<EncoderPipeline>>,
    pub monitor_registry: MonitorRegistry,
    pub config: Config,
    pub shutdown: Arc<AtomicBool>,
}
