use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use crate::frame::Frame;
use super::{SourceMetadata, SourceType, StreamSource};

/// Static image source. Decodes an image file once and holds it as a Frame.
/// If no image path is given, generates a black frame at the specified resolution.
pub struct ImageSource {
    frame: Option<Frame>,
    width: u32,
    height: u32,
    path: Option<String>,
    metadata: SourceMetadata,
}

impl ImageSource {
    /// Create from an image file path. Resolution is read from the image.
    pub fn from_file(path: &str) -> Result<Self> {
        let img = image::open(path)
            .with_context(|| format!("failed to open image: {path}"))?
            .to_rgba8();
        let width = img.width();
        let height = img.height();

        // Convert RGBA → BGRA (swap R and B channels).
        let mut data = img.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        Ok(Self {
            frame: Some(Frame { data, width, height }),
            width,
            height,
            path: Some(path.to_string()),
            metadata: SourceMetadata {
                name: format!("image:{path}"),
                source_type: SourceType::Image,
                native_width: width,
                native_height: height,
            },
        })
    }

    /// Create a solid black frame at the given resolution.
    pub fn black(width: u32, height: u32) -> Self {
        Self {
            frame: Some(Frame::new_black(width, height)),
            width,
            height,
            path: None,
            metadata: SourceMetadata {
                name: "image:black".into(),
                source_type: SourceType::Image,
                native_width: width,
                native_height: height,
            },
        }
    }
}

impl StreamSource for ImageSource {
    fn start(&mut self) -> Result<()> {
        info!("image source ready: {}x{}", self.width, self.height);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn frame(&self) -> Option<Frame> {
        self.frame.clone()
    }

    fn metadata(&self) -> &SourceMetadata {
        &self.metadata
    }

    fn is_running(&self) -> bool {
        self.frame.is_some()
    }
}
