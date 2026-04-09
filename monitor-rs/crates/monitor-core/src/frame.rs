/// Raw BGRA pixel buffer.
#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    pub fn new_black(width: u32, height: u32) -> Self {
        Self {
            data: vec![0u8; (width * height * 4) as usize],
            width,
            height,
        }
    }

    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// Blit `src` onto `self` at the given canvas position, clipping to bounds.
    pub fn blit(&mut self, src: &Frame, dst_x: i32, dst_y: i32) {
        let dst_w = self.width as i32;
        let dst_h = self.height as i32;
        let src_w = src.width as i32;
        let src_h = src.height as i32;

        // Compute the overlapping rectangle in canvas coords.
        let x0 = dst_x.max(0);
        let y0 = dst_y.max(0);
        let x1 = (dst_x + src_w).min(dst_w);
        let y1 = (dst_y + src_h).min(dst_h);

        if x0 >= x1 || y0 >= y1 {
            return; // No overlap.
        }

        let copy_w = (x1 - x0) as usize * 4;
        let src_start_x = (x0 - dst_x) as usize;
        let src_stride = src.stride();
        let dst_stride = self.stride();

        for row in 0..(y1 - y0) as usize {
            let src_row = (row + (y0 - dst_y) as usize) * src_stride + src_start_x * 4;
            let dst_row = (row + y0 as usize) * dst_stride + x0 as usize * 4;
            self.data[dst_row..dst_row + copy_w]
                .copy_from_slice(&src.data[src_row..src_row + copy_w]);
        }
    }
}
