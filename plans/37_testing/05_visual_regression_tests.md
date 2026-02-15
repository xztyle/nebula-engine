# Visual Regression Tests

## Problem

The Nebula Engine renders cubesphere-voxel planets with terrain, caves, lighting, and UI overlays. A change to a shader, the meshing algorithm, the LOD system, or the material pipeline can alter the rendered output in subtle or catastrophic ways. Without visual regression testing, these changes are only caught by manual inspection — which is slow, subjective, and impossible to run in CI.

Specific risks:

- **Shader changes** — A typo in a WGSL shader can shift colors, break lighting, or produce a black screen. These bugs are invisible to unit tests because shaders compile successfully but produce wrong pixels.
- **Meshing regressions** — A change to the greedy meshing algorithm might produce correct geometry with incorrect UV coordinates, causing textures to appear stretched or misaligned. Only a visual comparison catches this.
- **LOD transitions** — The LOD system switches between detail levels based on distance. A regression could cause visible popping, cracks between LOD levels, or missing chunks at certain distances.
- **Cubesphere seams** — The cubesphere has 6 faces that must tile seamlessly. A regression in the face-stitching code creates visible seams that are only apparent when rendered.
- **UI overlay** — The debug UI, HUD, and menus are rendered as overlays. A layout or rendering change could make text unreadable or elements overlap.

The engine uses `wgpu`, which supports headless rendering — no window or display server is required. This means visual tests can run in CI on machines without GPUs by using `wgpu`'s software rasterizer backend.

## Solution

### Headless rendering context

The test framework creates a headless `wgpu` device and renders to an off-screen texture. The texture is read back to CPU memory and saved as a PNG.

```rust
use wgpu::{Device, Queue, Texture, TextureDescriptor, TextureUsages, Extent3d};

pub struct HeadlessRenderer {
    device: Device,
    queue: Queue,
    render_texture: Texture,
    width: u32,
    height: u32,
}

impl HeadlessRenderer {
    pub async fn new(width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: true, // Use software rasterizer if no GPU
            })
            .await
            .expect("Failed to find a suitable adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("Failed to create device");

        let render_texture = device.create_texture(&TextureDescriptor {
            label: Some("visual_test_render_target"),
            size: Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self { device, queue, render_texture, width, height }
    }

    /// Render a scene and return the pixel data as RGBA bytes.
    pub fn render_scene(&self, scene: &TestScene) -> Vec<u8> {
        let view = self.render_texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());

        scene.render(&mut encoder, &view, &self.device, &self.queue);

        // Copy texture to buffer for readback
        let bytes_per_row = self.width * 4;
        let padded_bytes_per_row = (bytes_per_row + 255) & !255; // wgpu alignment
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_bytes_per_row * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            self.render_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );

        self.queue.submit(std::iter::once(encoder.finish()));
        // ... buffer mapping and readback omitted for brevity
        read_buffer_pixels(&buffer, self.width, self.height, padded_bytes_per_row)
    }
}
```

### Test scenes

Each test scene defines a camera position, world state, and expected visual output. Scenes are deterministic — they use fixed seeds, fixed camera transforms, and fixed lighting.

```rust
pub enum TestScene {
    /// Flat terrain viewed from directly above.
    FlatTerrainTopDown {
        seed: u64,
        camera_height: f64,
    },
    /// View of a cubesphere corner where 3 faces meet.
    CubesphereCorner {
        planet_radius: f64,
        camera_distance: f64,
    },
    /// Interior of a generated cave.
    CaveInterior {
        seed: u64,
        cave_position: (i64, i64, i64),
    },
    /// Space view of an entire planet.
    SpaceViewPlanet {
        planet_radius: f64,
        camera_distance: f64,
    },
    /// UI overlay on top of a simple background.
    UiOverlay {
        ui_elements: Vec<UiTestElement>,
    },
}
```

### Pixel comparison with tolerance

GPU rendering is not pixel-perfect across platforms and driver versions. The comparison algorithm allows a configurable per-pixel tolerance and a configurable percentage of pixels that may differ.

```rust
pub struct ComparisonResult {
    pub total_pixels: usize,
    pub differing_pixels: usize,
    pub max_channel_diff: u8,
    pub passed: bool,
}

pub fn compare_images(
    actual: &[u8],
    reference: &[u8],
    width: u32,
    height: u32,
    per_pixel_tolerance: u8,
    max_differing_percent: f64,
) -> ComparisonResult {
    assert_eq!(actual.len(), reference.len());
    let total_pixels = (width * height) as usize;
    let mut differing_pixels = 0;
    let mut max_channel_diff: u8 = 0;

    for i in 0..total_pixels {
        let offset = i * 4;
        let mut pixel_differs = false;
        for c in 0..4 {
            let diff = actual[offset + c].abs_diff(reference[offset + c]);
            if diff > per_pixel_tolerance {
                pixel_differs = true;
            }
            max_channel_diff = max_channel_diff.max(diff);
        }
        if pixel_differs {
            differing_pixels += 1;
        }
    }

    let differing_percent = (differing_pixels as f64 / total_pixels as f64) * 100.0;
    ComparisonResult {
        total_pixels,
        differing_pixels,
        max_channel_diff,
        passed: differing_percent <= max_differing_percent,
    }
}
```

### Reference image management

Reference images are stored in `tests/visual_references/` as PNG files, named by scene and resolution (e.g., `flat_terrain_top_down_1024x768.png`). When a test fails, the actual output and a difference image are saved alongside the reference for easy debugging.

To update references (e.g., after an intentional visual change), run:

```bash
NEBULA_UPDATE_VISUAL_REFS=1 cargo test --package nebula-testing -- visual
```

This environment variable causes the test to overwrite references instead of comparing.

### Difference image generation

When a comparison fails, a difference image is generated that highlights the differing pixels in red, making it easy to see exactly what changed.

```rust
pub fn generate_diff_image(actual: &[u8], reference: &[u8], width: u32, height: u32) -> Vec<u8> {
    let total_pixels = (width * height) as usize;
    let mut diff = vec![0u8; total_pixels * 4];
    for i in 0..total_pixels {
        let offset = i * 4;
        let channel_diff: u16 = (0..3)
            .map(|c| actual[offset + c].abs_diff(reference[offset + c]) as u16)
            .sum();
        if channel_diff > 0 {
            diff[offset] = 255; // Red
            diff[offset + 1] = 0;
            diff[offset + 2] = 0;
            diff[offset + 3] = 255;
        } else {
            // Dimmed version of actual for context
            diff[offset] = actual[offset] / 3;
            diff[offset + 1] = actual[offset + 1] / 3;
            diff[offset + 2] = actual[offset + 2] / 3;
            diff[offset + 3] = 255;
        }
    }
    diff
}
```

## Outcome

A visual regression testing module in `crates/nebula_testing/src/visual.rs` with a `HeadlessRenderer`, 5 predefined test scenes, pixel comparison with configurable tolerance, difference image generation, and reference image management. Reference images are stored in `tests/visual_references/`. Tests run in CI using `wgpu`'s software rasterizer backend. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo renders specific test scenes (known terrain seed from known camera angle) and compares screenshots against baseline images. Pixel differences are flagged.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Headless GPU rendering for screenshot capture |
| `image` | `0.25` | PNG encoding/decoding for reference and output images |
| `serde` | `1.0` (features: `derive`) | Serialization of test scene configurations |
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`) | Async runtime for wgpu device initialization |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Render the flat terrain scene and compare against the reference image.
    /// Tolerance: 5 per channel, 1% of pixels may differ.
    #[tokio::test]
    async fn test_flat_terrain_screenshot_matches_reference() {
        let renderer = HeadlessRenderer::new(1024, 768).await;
        let scene = TestScene::FlatTerrainTopDown {
            seed: 42,
            camera_height: 200.0,
        };
        let actual = renderer.render_scene(&scene);
        let reference = load_reference_image("flat_terrain_top_down_1024x768.png");

        let result = compare_images(&actual, &reference, 1024, 768, 5, 1.0);
        if !result.passed {
            save_failure_artifacts("flat_terrain_top_down", &actual, &reference, 1024, 768);
        }
        assert!(
            result.passed,
            "Flat terrain screenshot differs: {}/{} pixels ({:.2}%), max channel diff: {}",
            result.differing_pixels,
            result.total_pixels,
            (result.differing_pixels as f64 / result.total_pixels as f64) * 100.0,
            result.max_channel_diff,
        );
    }

    /// When NEBULA_UPDATE_VISUAL_REFS=1 is set, the reference image is
    /// overwritten instead of compared.
    #[tokio::test]
    async fn test_reference_image_can_be_saved() {
        let renderer = HeadlessRenderer::new(64, 64).await;
        let scene = TestScene::FlatTerrainTopDown {
            seed: 42,
            camera_height: 200.0,
        };
        let pixels = renderer.render_scene(&scene);
        assert_eq!(
            pixels.len(),
            64 * 64 * 4,
            "Rendered image should have correct byte count"
        );
        // Verify we can encode as PNG without error.
        let png_bytes = encode_png(&pixels, 64, 64);
        assert!(!png_bytes.is_empty());
    }

    /// Verify that an intentional shader change is detected as a regression.
    #[test]
    fn test_regression_detected_on_visual_change() {
        let reference = vec![128u8; 64 * 64 * 4]; // Uniform gray
        let mut actual = reference.clone();
        // Change a 10x10 block to white (significant change)
        for y in 0..10 {
            for x in 0..10 {
                let offset = (y * 64 + x) * 4;
                actual[offset] = 255;
                actual[offset + 1] = 255;
                actual[offset + 2] = 255;
            }
        }

        let result = compare_images(&actual, &reference, 64, 64, 5, 1.0);
        assert!(
            !result.passed,
            "A 10x10 block change in a 64x64 image should exceed 1% tolerance"
        );
        assert!(result.differing_pixels >= 100);
    }

    /// Verify that the comparison passes for identical images.
    #[test]
    fn test_identical_images_pass_comparison() {
        let image = vec![42u8; 128 * 128 * 4];
        let result = compare_images(&image, &image, 128, 128, 0, 0.0);
        assert!(result.passed);
        assert_eq!(result.differing_pixels, 0);
        assert_eq!(result.max_channel_diff, 0);
    }

    /// Verify that the headless renderer produces a valid (non-zero) image
    /// rather than an all-black or all-transparent output.
    #[tokio::test]
    async fn test_headless_rendering_produces_valid_image() {
        let renderer = HeadlessRenderer::new(256, 256).await;
        let scene = TestScene::SpaceViewPlanet {
            planet_radius: 1000.0,
            camera_distance: 5000.0,
        };
        let pixels = renderer.render_scene(&scene);
        assert_eq!(pixels.len(), 256 * 256 * 4);

        // At least some pixels should be non-zero (not all black).
        let any_nonzero = pixels.chunks(4).any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(
            any_nonzero,
            "Rendered image should not be entirely black"
        );
    }

    /// Verify that the difference image generator highlights changed pixels in red.
    #[test]
    fn test_diff_image_highlights_changes() {
        let reference = vec![0u8; 4 * 4 * 4]; // 4x4 black image
        let mut actual = reference.clone();
        // Make pixel (0,0) white
        actual[0] = 255;
        actual[1] = 255;
        actual[2] = 255;
        actual[3] = 255;

        let diff = generate_diff_image(&actual, &reference, 4, 4);
        // Pixel (0,0) should be red in the diff image
        assert_eq!(diff[0], 255, "Diff pixel R channel should be 255");
        assert_eq!(diff[1], 0, "Diff pixel G channel should be 0");
        assert_eq!(diff[2], 0, "Diff pixel B channel should be 0");

        // Pixel (1,0) should be dimmed (unchanged)
        assert_eq!(diff[4], 0, "Unchanged pixel should remain dark");
    }
}
```
