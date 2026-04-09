pub mod desktop;
pub mod image;
pub mod window;

use crate::frame::Frame;
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    Desktop,
    Image,
    Window,
    Media,
}

#[derive(Debug, Clone)]
pub struct SourceMetadata {
    pub name: String,
    pub source_type: SourceType,
    pub native_width: u32,
    pub native_height: u32,
}

/// Trait for all capture/playback sources.
///
/// Each implementation spawns its own reader thread (if applicable) and
/// exposes the latest frame via `frame()` without blocking.
pub trait StreamSource: Send + Sync {
    /// Start capture or playback. May spawn FFmpeg subprocess + reader thread.
    fn start(&mut self) -> Result<()>;

    /// Stop capture. Kill subprocess, join reader thread, release resources.
    fn stop(&mut self) -> Result<()>;

    /// Return the most recent frame, or `None` if no frame has been captured yet.
    /// Must be non-blocking.
    fn frame(&self) -> Option<Frame>;

    fn metadata(&self) -> &SourceMetadata;

    fn is_running(&self) -> bool;
}
