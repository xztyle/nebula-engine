//! A 2D debug image represented as a flat array of RGBA pixels.

/// A 2D debug image for terrain visualization, stored as row-major RGBA pixels.
#[derive(Clone, Debug)]
pub struct DebugImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Pixel data in row-major RGBA format. Length = `width * height * 4`.
    pub pixels: Vec<u8>,
}

impl DebugImage {
    /// Create a new black (all-zero) image with the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    /// Set a single pixel's RGBA value.
    ///
    /// # Panics
    ///
    /// Panics if `x >= width` or `y >= height`.
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        let idx = ((y * self.width + x) * 4) as usize;
        self.pixels[idx] = r;
        self.pixels[idx + 1] = g;
        self.pixels[idx + 2] = b;
        self.pixels[idx + 3] = a;
    }

    /// Get a pixel's RGBA value.
    ///
    /// # Panics
    ///
    /// Panics if `x >= width` or `y >= height`.
    pub fn get_pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let idx = ((y * self.width + x) * 4) as usize;
        (
            self.pixels[idx],
            self.pixels[idx + 1],
            self.pixels[idx + 2],
            self.pixels[idx + 3],
        )
    }

    /// Returns `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns the number of pixels in the image.
    pub fn pixel_count(&self) -> u32 {
        self.width * self.height
    }

    /// Count the number of unique colors (ignoring alpha) in the image.
    pub fn unique_color_count(&self) -> usize {
        let mut colors = std::collections::HashSet::new();
        for chunk in self.pixels.chunks_exact(4) {
            colors.insert((chunk[0], chunk[1], chunk[2]));
        }
        colors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_image_correct_dimensions() {
        let image = DebugImage::new(256, 128);
        assert_eq!(image.dimensions(), (256, 128));
        assert_eq!(image.pixels.len(), 256 * 128 * 4);
    }

    #[test]
    fn test_debug_image_set_pixel() {
        let mut image = DebugImage::new(10, 10);
        image.set_pixel(3, 5, 255, 128, 64, 255);

        let idx = ((5 * 10 + 3) * 4) as usize;
        assert_eq!(image.pixels[idx], 255);
        assert_eq!(image.pixels[idx + 1], 128);
        assert_eq!(image.pixels[idx + 2], 64);
        assert_eq!(image.pixels[idx + 3], 255);
    }

    #[test]
    fn test_get_pixel_roundtrip() {
        let mut image = DebugImage::new(8, 8);
        image.set_pixel(2, 3, 10, 20, 30, 40);
        assert_eq!(image.get_pixel(2, 3), (10, 20, 30, 40));
    }

    #[test]
    fn test_unique_color_count() {
        let mut image = DebugImage::new(4, 1);
        image.set_pixel(0, 0, 255, 0, 0, 255);
        image.set_pixel(1, 0, 0, 255, 0, 255);
        image.set_pixel(2, 0, 255, 0, 0, 255); // duplicate
        image.set_pixel(3, 0, 0, 0, 255, 255);
        // (255,0,0), (0,255,0), (255,0,0), (0,0,255) = 3 unique
        assert_eq!(image.unique_color_count(), 3);
    }
}
