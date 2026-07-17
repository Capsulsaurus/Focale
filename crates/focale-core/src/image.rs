//! The working image buffer: interleaved RGB f32.
//!
//! This is the only pixel container on the processing path. Working space is
//! linear Rec.2020, f32, unbounded (architecture.md §3) — but the container itself is
//! space-agnostic; stages document what they expect.

/// Interleaved RGB f32 image, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRgbF32 {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl ImageRgbF32 {
    /// Creates a zero-filled image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; width as usize * height as usize * 3],
        }
    }

    /// Wraps existing interleaved RGB data.
    ///
    /// # Panics
    /// If `data.len() != width * height * 3`.
    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize * 3,
            "pixel buffer length must be width*height*3"
        );
        Self {
            width,
            height,
            data,
        }
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The interleaved pixel data.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Mutable access to the interleaved pixel data.
    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Consumes the image, returning the pixel buffer.
    pub fn into_data(self) -> Vec<f32> {
        self.data
    }

    /// Reads one pixel. Debug-asserted bounds.
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 3] {
        debug_assert!(x < self.width && y < self.height);
        let i = (y as usize * self.width as usize + x as usize) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    /// Writes one pixel. Debug-asserted bounds.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        debug_assert!(x < self.width && y < self.height);
        let i = (y as usize * self.width as usize + x as usize) * 3;
        self.data[i] = rgb[0];
        self.data[i + 1] = rgb[1];
        self.data[i + 2] = rgb[2];
    }

    /// One image row as an interleaved slice.
    #[inline]
    pub fn row(&self, y: u32) -> &[f32] {
        let w = self.width as usize * 3;
        let start = y as usize * w;
        &self.data[start..start + w]
    }

    /// Iterates over rows mutably — the building block for deterministic
    /// parallelism: disjoint rows may be processed on any thread because
    /// each output value depends only on its own input (HARD-DET).
    pub fn rows_mut(&mut self) -> std::slice::ChunksMut<'_, f32> {
        self.data.chunks_mut(self.width as usize * 3)
    }

    /// Bilinearly samples the image at continuous coordinates (pixel-centre
    /// convention: integer coordinates land on pixel centres). Out-of-bounds
    /// coordinates clamp to the edge.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 3] {
        let max_x = (self.width - 1) as f32;
        let max_y = (self.height - 1) as f32;
        let x = x.clamp(0.0, max_x);
        let y = y.clamp(0.0, max_y);
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let x0 = x0 as u32;
        let y0 = y0 as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let p00 = self.pixel(x0, y0);
        let p10 = self.pixel(x1, y0);
        let p01 = self.pixel(x0, y1);
        let p11 = self.pixel(x1, y1);
        let mut out = [0.0; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let top = p00[c] * (1.0 - fx) + p10[c] * fx;
            let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
            *o = top * (1.0 - fy) + bottom * fy;
        }
        out
    }
}

/// Single-channel f32 image (mask coverage, luminance planes), row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageGrayF32 {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl ImageGrayF32 {
    /// Creates a zero-filled plane.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; width as usize * height as usize],
        }
    }

    /// Wraps existing data.
    ///
    /// # Panics
    /// If `data.len() != width * height`.
    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize,
            "plane buffer length must be width*height"
        );
        Self {
            width,
            height,
            data,
        }
    }

    /// Plane width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Plane height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The sample data.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Mutable access to the sample data.
    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Reads one sample. Debug-asserted bounds.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> f32 {
        debug_assert!(x < self.width && y < self.height);
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Writes one sample. Debug-asserted bounds.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: f32) {
        debug_assert!(x < self.width && y < self.height);
        self.data[y as usize * self.width as usize + x as usize] = v;
    }

    /// Iterates over rows mutably (disjoint-row parallelism primitive).
    pub fn rows_mut(&mut self) -> std::slice::ChunksMut<'_, f32> {
        self.data.chunks_mut(self.width as usize)
    }

    /// Bilinearly samples at continuous coordinates, clamping at edges
    /// (pixel-centre convention, like [`ImageRgbF32::sample_bilinear`]).
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let max_x = (self.width - 1) as f32;
        let max_y = (self.height - 1) as f32;
        let x = x.clamp(0.0, max_x);
        let y = y.clamp(0.0, max_y);
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let x0 = x0 as u32;
        let y0 = y0 as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let top = self.get(x0, y0) * (1.0 - fx) + self.get(x1, y0) * fx;
        let bottom = self.get(x0, y1) * (1.0 - fx) + self.get(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_roundtrip_and_sampling() {
        let mut g = ImageGrayF32::new(2, 1);
        g.set(1, 0, 1.0);
        assert_eq!(g.get(0, 0), 0.0);
        assert_eq!(g.sample_bilinear(0.5, 0.0), 0.5);
    }

    #[test]
    fn pixel_roundtrip() {
        let mut img = ImageRgbF32::new(4, 3);
        img.set_pixel(2, 1, [0.1, 0.2, 0.3]);
        assert_eq!(img.pixel(2, 1), [0.1, 0.2, 0.3]);
        assert_eq!(img.pixel(0, 0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn bilinear_interpolates_midpoint() {
        let mut img = ImageRgbF32::new(2, 1);
        img.set_pixel(0, 0, [0.0, 0.0, 0.0]);
        img.set_pixel(1, 0, [1.0, 2.0, 4.0]);
        assert_eq!(img.sample_bilinear(0.5, 0.0), [0.5, 1.0, 2.0]);
    }

    #[test]
    fn bilinear_clamps_at_edges() {
        let mut img = ImageRgbF32::new(2, 2);
        img.set_pixel(1, 1, [1.0, 1.0, 1.0]);
        assert_eq!(img.sample_bilinear(5.0, 5.0), [1.0, 1.0, 1.0]);
        assert_eq!(img.sample_bilinear(-3.0, -3.0), [0.0, 0.0, 0.0]);
    }
}
