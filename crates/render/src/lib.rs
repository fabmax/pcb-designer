//! GPU canvas renderer for the schematic and PCB editors.
//!
//! The CPU side ([`Scene`]) holds instanced primitives grouped into render layers and
//! tracks what changed. The GPU side ([`gpu::Renderer`], [`gpu::GpuScene`]) uploads only
//! the dirty parts and draws every primitive as an instanced quad that is shaded with a
//! signed distance function. Edges are therefore exact at every zoom level and panning or
//! zooming only touches a single uniform.

pub mod camera;
pub mod font;
pub mod gpu;
pub mod offscreen;
pub mod primitive;
pub mod scene;

pub use camera::Camera;
pub use primitive::{Color, Instance};
pub use scene::{ItemId, LayerId, Scene, SceneUpload};
