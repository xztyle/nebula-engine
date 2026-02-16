//! Material system: PBR material definitions, GPU-friendly packed data, and material identifiers.

mod material;

pub use material::{MaterialDef, MaterialError, MaterialGpuData, MaterialId};
