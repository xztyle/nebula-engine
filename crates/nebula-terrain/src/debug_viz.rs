//! Terrain debug visualization: 2D image rendering of terrain generation data.
//!
//! Provides [`DebugImage`] and rendering functions for heightmaps, biome maps,
//! cave cross-sections, and ore distribution heatmaps. These are used by debug
//! overlays to visually diagnose terrain generation issues.

mod image;
mod renderers;

pub use self::image::DebugImage;
pub use renderers::{
    SliceParams, biome_color, height_to_color, render_biome_debug, render_cave_cross_section,
    render_heightmap_debug, render_ore_heatmap,
};

/// State for all terrain debug overlays.
///
/// Tracks which overlays are visible and whether cached textures need
/// regeneration (e.g., after parameter changes).
#[derive(Clone, Debug)]
pub struct TerrainDebugState {
    /// Whether the heightmap overlay is visible.
    pub show_heightmap: bool,
    /// Whether the biome map overlay is visible.
    pub show_biome_map: bool,
    /// Whether the cave cross-section overlay is visible.
    pub show_cave_section: bool,
    /// Whether the ore heatmap overlay is visible.
    pub show_ore_heatmap: bool,
    /// Whether the cached textures need regeneration.
    dirty: bool,
}

impl TerrainDebugState {
    /// Create a new state with all overlays hidden.
    pub fn new() -> Self {
        Self {
            show_heightmap: false,
            show_biome_map: false,
            show_cave_section: false,
            show_ore_heatmap: false,
            dirty: true,
        }
    }

    /// Mark cached textures as needing regeneration.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns `true` if cached textures need regeneration.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag after regenerating textures.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Returns `true` if any overlay is currently visible.
    pub fn any_visible(&self) -> bool {
        self.show_heightmap
            || self.show_biome_map
            || self.show_cave_section
            || self.show_ore_heatmap
    }
}

impl Default for TerrainDebugState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlays_can_be_toggled() {
        let mut state = TerrainDebugState::new();

        assert!(!state.show_heightmap);
        assert!(!state.show_biome_map);
        assert!(!state.any_visible());

        state.show_heightmap = true;
        assert!(state.any_visible());

        state.show_heightmap = false;
        assert!(!state.any_visible());

        state.show_biome_map = true;
        state.show_cave_section = true;
        state.show_ore_heatmap = true;
        assert!(state.any_visible());

        state.show_biome_map = false;
        state.show_cave_section = false;
        state.show_ore_heatmap = false;
        assert!(!state.any_visible());
    }

    #[test]
    fn test_dirty_flag() {
        let mut state = TerrainDebugState::new();
        assert!(state.is_dirty());

        state.clear_dirty();
        assert!(!state.is_dirty());

        state.mark_dirty();
        assert!(state.is_dirty());
    }
}
