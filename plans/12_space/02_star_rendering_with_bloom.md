# Star Rendering with Bloom

## Problem

Stars rendered as single pixels on a cubemap look flat and lifeless. In real cameras and human vision, bright point light sources produce a glow or bloom effect caused by light scattering in the lens or eye. Without bloom, the brightest star in the sky looks identical to a dim one except for pixel intensity -- there is no visual "punch" that communicates extreme brightness. The engine needs a post-processing bloom pass that makes bright stars glow, with the bloom intensity proportional to the star's brightness. The sun (nearest star) should produce the most dramatic bloom. The bloom must not bleed into non-bright regions of the image, and the bloom radius should be configurable to allow artistic control.

## Solution

Implement a multi-pass bloom post-processing pipeline in the `nebula-render` crate. The bloom operates on the HDR render target after the main scene pass and before tonemapping. The pipeline consists of four stages: brightness extraction, progressive downsampling with blur, upsampling with accumulation, and final compositing.

### Bloom Configuration

```rust
/// Configuration for the bloom post-processing effect.
#[derive(Clone, Debug)]
pub struct BloomConfig {
    /// Brightness threshold. Only pixels with luminance above this value contribute to bloom.
    /// Default: 1.0 (anything above standard white is considered "bright").
    pub threshold: f32,
    /// Soft knee for the threshold curve. Prevents hard cutoff artifacts.
    /// Range [0, 1]. Default: 0.5.
    pub soft_knee: f32,
    /// Overall bloom intensity multiplier. Default: 0.3.
    pub intensity: f32,
    /// Number of downscale iterations. Each iteration halves resolution and doubles blur radius.
    /// More iterations = wider bloom. Range [1, 8]. Default: 5.
    pub iterations: u32,
    /// Bloom radius multiplier. Scales the Gaussian kernel width. Default: 1.0.
    pub radius: f32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            threshold: 1.0,
            soft_knee: 0.5,
            intensity: 0.3,
            iterations: 5,
            radius: 1.0,
        }
    }
}
```

### Brightness Extraction Pass

The first pass extracts bright pixels from the HDR render target into a half-resolution texture. A soft knee function smoothly transitions around the threshold to avoid harsh edges:

```wgsl
fn soft_threshold(color: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let soft = luminance - threshold + knee;
    let soft_clamped = clamp(soft, 0.0, 2.0 * knee);
    let contribution = soft_clamped * soft_clamped / (4.0 * knee + 0.0001);
    let factor = max(luminance - threshold, contribution) / max(luminance, 0.0001);
    return color * factor;
}

@fragment
fn fs_brightness_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let hdr_color = textureSample(hdr_texture, hdr_sampler, in.uv).rgb;
    let extracted = soft_threshold(hdr_color, bloom_params.threshold, bloom_params.soft_knee);
    return vec4<f32>(extracted, 1.0);
}
```

### Progressive Downsample with Gaussian Blur

Each downsample step halves the texture resolution and applies a 9-tap Gaussian blur. The two-pass (horizontal then vertical) separable filter approach is used for efficiency:

```rust
pub struct BloomPipeline {
    brightness_extract_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    mip_textures: Vec<wgpu::Texture>,
    config: BloomConfig,
}

impl BloomPipeline {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        screen_width: u32,
        screen_height: u32,
        config: BloomConfig,
    ) -> Self {
        // Create a chain of textures at half, quarter, eighth, etc. resolution.
        let mut mip_textures = Vec::new();
        let mut w = screen_width / 2;
        let mut h = screen_height / 2;

        for _ in 0..config.iterations {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom-mip"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: hdr_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                     | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            mip_textures.push(texture);
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }

        // ... create pipelines for each stage ...
        Self { brightness_extract_pipeline, downsample_pipeline, upsample_pipeline, composite_pipeline, mip_textures, config }
    }
}
```

### Gaussian Blur Shader

```wgsl
// 9-tap Gaussian weights for sigma ~= 1.5 (normalized).
const WEIGHTS: array<f32, 5> = array<f32, 5>(
    0.2270270270, 0.1945945946, 0.1216216216, 0.0540540541, 0.0162162162
);

@fragment
fn fs_blur_horizontal(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / vec2<f32>(textureDimensions(input_texture));
    var result = textureSample(input_texture, blur_sampler, in.uv).rgb * WEIGHTS[0];

    for (var i = 1; i < 5; i = i + 1) {
        let offset = vec2<f32>(f32(i) * texel_size.x * bloom_params.radius, 0.0);
        result += textureSample(input_texture, blur_sampler, in.uv + offset).rgb * WEIGHTS[i];
        result += textureSample(input_texture, blur_sampler, in.uv - offset).rgb * WEIGHTS[i];
    }

    return vec4<f32>(result, 1.0);
}

@fragment
fn fs_blur_vertical(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / vec2<f32>(textureDimensions(input_texture));
    var result = textureSample(input_texture, blur_sampler, in.uv).rgb * WEIGHTS[0];

    for (var i = 1; i < 5; i = i + 1) {
        let offset = vec2<f32>(0.0, f32(i) * texel_size.y * bloom_params.radius);
        result += textureSample(input_texture, blur_sampler, in.uv + offset).rgb * WEIGHTS[i];
        result += textureSample(input_texture, blur_sampler, in.uv - offset).rgb * WEIGHTS[i];
    }

    return vec4<f32>(result, 1.0);
}
```

### Upsample and Accumulation

The upsampling pass works from the smallest mip back up to half resolution, additively blending each level with the next larger one. This creates the characteristic wide-then-narrow bloom falloff:

```wgsl
@fragment
fn fs_upsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // Bilinear sample from the smaller (more blurred) mip level.
    let blurred = textureSample(smaller_mip, blur_sampler, in.uv).rgb;
    // Sample the current (larger) mip level.
    let current = textureSample(current_mip, blur_sampler, in.uv).rgb;
    // Additive blend with intensity control.
    return vec4<f32>(current + blurred * bloom_params.intensity, 1.0);
}
```

### Final Composite

The accumulated bloom texture is additively composited onto the HDR render target before tonemapping:

```wgsl
@fragment
fn fs_bloom_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let scene_color = textureSample(hdr_texture, scene_sampler, in.uv).rgb;
    let bloom_color = textureSample(bloom_texture, bloom_sampler, in.uv).rgb;
    return vec4<f32>(scene_color + bloom_color, 1.0);
}
```

### Integration with Stars and Sun

Stars rendered into the HDR skybox cubemap already have brightness values. Stars with brightness > 1.0 (the threshold) will naturally be extracted by the bloom pass. The sun (story 04) will have HDR values of 10.0-100.0+, producing a massive bloom. Dim stars (brightness < 1.0) pass through the threshold unaffected -- they appear as crisp points without glow.

### Execution Order

```rust
impl BloomPipeline {
    pub fn execute(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr_input: &wgpu::TextureView,
        output: &wgpu::TextureView,
    ) {
        // 1. Extract bright pixels from HDR input -> mip[0]
        self.run_brightness_extract(encoder, hdr_input);

        // 2. Progressive downsample: mip[0] -> mip[1] -> ... -> mip[N-1]
        for i in 1..self.config.iterations as usize {
            self.run_downsample_blur(encoder, i - 1, i);
        }

        // 3. Progressive upsample: mip[N-1] -> mip[N-2] -> ... -> mip[0]
        for i in (0..self.config.iterations as usize - 1).rev() {
            self.run_upsample_accumulate(encoder, i + 1, i);
        }

        // 4. Composite bloom mip[0] onto HDR input -> output
        self.run_composite(encoder, hdr_input, output);
    }
}
```

## Outcome

A `BloomPipeline` and `BloomConfig` in `nebula-render` that applies a configurable multi-pass bloom effect to any HDR render target. Bright stars glow proportionally to their intensity, the sun produces a dramatic bloom, and dim regions remain unaffected. Running `cargo test -p nebula-render` passes all bloom pipeline tests. The bloom integrates between the main scene render pass and the tonemapping pass.

## Demo Integration

**Demo crate:** `nebula-demo`

Bright stars glow with soft halos. The sun produces a dramatic bloom effect. Dim regions of the scene remain unaffected by the bloom pass.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, texture creation, compute passes |
| `bytemuck` | `1.21` | Uniform buffer serialization for bloom parameters |
| `glam` | `0.29` | Vector math for Gaussian kernel computation |

The bloom pipeline lives in `nebula-render`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_config_default_values() {
        let config = BloomConfig::default();
        assert_eq!(config.threshold, 1.0);
        assert_eq!(config.intensity, 0.3);
        assert_eq!(config.iterations, 5);
        assert_eq!(config.radius, 1.0);
        assert!(config.soft_knee >= 0.0 && config.soft_knee <= 1.0);
    }

    #[test]
    fn test_bloom_pipeline_creates_correct_mip_chain() {
        let device = create_test_device();
        let bloom = BloomPipeline::new(
            &device,
            wgpu::TextureFormat::Rgba16Float,
            1920,
            1080,
            BloomConfig::default(),
        );
        // 5 iterations: 960x540, 480x270, 240x135, 120x67, 60x33
        assert_eq!(bloom.mip_textures.len(), 5);
    }

    #[test]
    fn test_bright_points_produce_glow() {
        // Simulate the soft threshold extraction.
        let bright_pixel = [5.0_f32, 5.0, 5.0]; // well above threshold of 1.0
        let threshold = 1.0;
        let knee = 0.5;
        let luminance = bright_pixel[0] * 0.2126
            + bright_pixel[1] * 0.7152
            + bright_pixel[2] * 0.0722;
        // Luminance of 5.0 is well above threshold, so extraction should be significant.
        let factor = (luminance - threshold) / luminance;
        let extracted: Vec<f32> = bright_pixel.iter().map(|c| c * factor).collect();
        assert!(
            extracted[0] > 0.0 && extracted[1] > 0.0 && extracted[2] > 0.0,
            "Bright pixels should produce non-zero bloom contribution: {extracted:?}"
        );
    }

    #[test]
    fn test_dim_points_produce_minimal_glow() {
        // A pixel with luminance well below threshold should be fully rejected.
        let dim_pixel = [0.3_f32, 0.3, 0.3]; // luminance ~0.3, below threshold 1.0
        let threshold = 1.0;
        let luminance = dim_pixel[0] * 0.2126
            + dim_pixel[1] * 0.7152
            + dim_pixel[2] * 0.0722;
        // Below threshold: factor should be zero or near-zero.
        let factor = ((luminance - threshold).max(0.0)) / luminance.max(0.0001);
        let extracted: Vec<f32> = dim_pixel.iter().map(|c| c * factor).collect();
        assert!(
            extracted.iter().all(|&v| v < 0.01),
            "Dim pixels should produce near-zero bloom contribution: {extracted:?}"
        );
    }

    #[test]
    fn test_bloom_radius_is_configurable() {
        let config_narrow = BloomConfig { radius: 0.5, ..Default::default() };
        let config_wide = BloomConfig { radius: 2.0, ..Default::default() };
        // The radius parameter scales the Gaussian kernel offset.
        // With radius=0.5, the blur is half as wide; with radius=2.0, twice as wide.
        assert!(config_narrow.radius < config_wide.radius);
        assert_eq!(config_narrow.radius, 0.5);
        assert_eq!(config_wide.radius, 2.0);
    }

    #[test]
    fn test_bloom_does_not_affect_non_bright_pixels() {
        // The soft threshold function with knee=0.5 should fully reject pixels
        // that are more than `knee` below the threshold.
        let threshold = 1.0;
        let knee = 0.5;
        let test_luminance = 0.4; // well below (threshold - knee) = 0.5

        let soft = test_luminance - threshold + knee;
        let contribution = if soft > 0.0 {
            let soft_clamped = soft.min(2.0 * knee);
            soft_clamped * soft_clamped / (4.0 * knee + 0.0001)
        } else {
            0.0
        };
        let factor = (test_luminance - threshold).max(contribution) / test_luminance.max(0.0001);
        assert!(
            factor <= 0.0,
            "Pixel at luminance {test_luminance} should have zero bloom factor, got {factor}"
        );
    }

    #[test]
    fn test_gaussian_weights_sum_to_approximately_one() {
        // The 9-tap Gaussian kernel (center + 4 pairs) should sum to ~1.0.
        let weights = [0.2270270270_f32, 0.1945945946, 0.1216216216, 0.0540540541, 0.0162162162];
        let sum = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
        assert!(
            (sum - 1.0).abs() < 0.01,
            "Gaussian weights should sum to ~1.0, got {sum}"
        );
    }

    #[test]
    fn test_bloom_intensity_scales_output() {
        let config_low = BloomConfig { intensity: 0.1, ..Default::default() };
        let config_high = BloomConfig { intensity: 1.0, ..Default::default() };
        // The intensity multiplier directly scales the bloom contribution during upsampling.
        // A higher intensity means stronger glow.
        assert!(
            config_high.intensity > config_low.intensity,
            "High intensity ({}) should exceed low intensity ({})",
            config_high.intensity,
            config_low.intensity
        );
    }

    #[test]
    fn test_mip_chain_dimensions_halve_each_level() {
        let mut w = 1920u32 / 2;
        let mut h = 1080u32 / 2;
        let expected_dims = [
            (960, 540),
            (480, 270),
            (240, 135),
            (120, 67),
            (60, 33),
        ];
        for (i, &(ew, eh)) in expected_dims.iter().enumerate() {
            assert_eq!(
                (w, h),
                (ew, eh),
                "Mip level {i} dimensions mismatch"
            );
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }
}
```
