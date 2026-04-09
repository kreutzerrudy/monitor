use crate::frame::Frame;

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }

    /// Create a fresh black frame at the canvas resolution.
    pub fn blank_frame(&self) -> Frame {
        Frame::new_black(self.width, self.height)
    }
}
