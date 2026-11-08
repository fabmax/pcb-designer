//! GPU side: pipelines ([`Renderer`]) and per-canvas buffers ([`GpuScene`]).

use bytemuck::{Pod, Zeroable};

use crate::camera::Camera;
use crate::primitive::{Color, Instance};
use crate::scene::{LayerData, SceneUpload};

/// Background and grid settings for a canvas.
#[derive(Clone, Copy, Debug)]
pub struct Background {
    pub color: Color,
    pub grid_color: Color,
    pub axis_color: Color,
    /// Smallest grid spacing (world units). The displayed grid is a multiple of it so that
    /// grid points stay at least `min_grid_px` apart.
    pub grid: f32,
    pub min_grid_px: f32,
    pub show_grid: bool,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            color: Color::rgb(0x16, 0x18, 0x1d),
            grid_color: Color::rgba(0x5a, 0x60, 0x70, 0xb0),
            axis_color: Color::rgba(0x80, 0x88, 0x98, 0x90),
            grid: 1.0,
            min_grid_px: 14.0,
            show_grid: true,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ViewUniform {
    center: [f32; 2],
    ndc_scale: [f32; 2],
    px: f32,
    grid: f32,
    grid_major: f32,
    _pad: f32,
    bg: [f32; 4],
    grid_color: [f32; 4],
    axis_color: [f32; 4],
}

impl ViewUniform {
    pub fn new(camera: &Camera, viewport_px: [f32; 2], bg: &Background) -> Self {
        let zoom = camera.zoom as f32;
        let grid = if bg.show_grid {
            adaptive_grid(bg.grid, zoom, bg.min_grid_px)
        } else {
            0.0
        };
        Self {
            center: [camera.center[0] as f32, camera.center[1] as f32],
            ndc_scale: [
                2.0 * zoom / viewport_px[0].max(1.0),
                2.0 * zoom / viewport_px[1].max(1.0),
            ],
            px: 1.0 / zoom,
            grid,
            grid_major: 10.0,
            _pad: 0.0,
            bg: bg.color.to_f32(),
            grid_color: bg.grid_color.to_f32(),
            axis_color: bg.axis_color.to_f32(),
        }
    }

    /// Displayed minor grid spacing in world units (0 if hidden).
    pub fn grid(&self) -> f32 {
        self.grid
    }
}

/// Multiply `base` by 2, 5, 10, 20, … until grid points are at least `min_px` apart.
pub fn adaptive_grid(base: f32, zoom: f32, min_px: f32) -> f32 {
    let mut g = base;
    let steps = [2.0, 2.5, 2.0];
    let mut i = 0;
    while g * zoom < min_px && g < 1e7 {
        g *= steps[i % 3];
        i += 1;
    }
    g
}

/// Shared pipelines. One per wgpu device and target format.
pub struct Renderer {
    prim_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let instance_attrs = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32,
            3 => Float32,
            4 => Uint32,
            5 => Unorm8x4,
        ];

        let prim_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas primitives"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_prim"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_prim"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas grid"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            prim_pipeline,
            grid_pipeline,
            bind_group_layout,
        }
    }
}

struct GpuLayer {
    buffer: Option<wgpu::Buffer>,
    capacity: usize,
    len: usize,
}

/// GPU copy of one [`crate::Scene`] plus its view uniform.
pub struct GpuScene {
    layers: Vec<GpuLayer>,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GpuScene {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas view uniform"),
            size: size_of::<ViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas view"),
            layout: &renderer.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            layers: Vec::new(),
            uniform,
            bind_group,
        }
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, upload: &SceneUpload) {
        if self.layers.len() < upload.layers.len() {
            self.layers.resize_with(upload.layers.len(), || GpuLayer {
                buffer: None,
                capacity: 0,
                len: 0,
            });
        }
        for (layer, up) in self.layers.iter_mut().zip(&upload.layers) {
            match &up.data {
                LayerData::Unchanged => {}
                LayerData::Full { capacity, instances } => {
                    if layer.capacity < *capacity || layer.buffer.is_none() {
                        layer.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("canvas layer"),
                            size: (*capacity * size_of::<Instance>()) as u64,
                            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }));
                        layer.capacity = *capacity;
                    }
                    if !instances.is_empty() {
                        queue.write_buffer(layer.buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(instances));
                    }
                }
                LayerData::Partial(ranges) => {
                    if let Some(buffer) = &layer.buffer {
                        for (offset, instances) in ranges.iter().filter(|(_, i)| !i.is_empty()) {
                            let byte_offset = (*offset * size_of::<Instance>()) as u64;
                            queue.write_buffer(buffer, byte_offset, bytemuck::cast_slice(instances));
                        }
                    }
                }
            }
            layer.len = up.len;
        }
    }

    pub fn set_view(&self, queue: &wgpu::Queue, view: &ViewUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(view));
    }

    /// Draw background, grid and all visible layers into `pass`.
    pub fn draw(&self, renderer: &Renderer, pass: &mut wgpu::RenderPass<'_>, visible: &[bool]) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_pipeline(&renderer.grid_pipeline);
        pass.draw(0..3, 0..1);

        pass.set_pipeline(&renderer.prim_pipeline);
        for (i, layer) in self.layers.iter().enumerate() {
            if layer.len == 0 || !visible.get(i).copied().unwrap_or(true) {
                continue;
            }
            if let Some(buffer) = &layer.buffer {
                let bytes = (layer.len * size_of::<Instance>()) as u64;
                pass.set_vertex_buffer(0, buffer.slice(0..bytes));
                pass.draw(0..4, 0..layer.len as u32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_grid_keeps_spacing() {
        assert_eq!(adaptive_grid(1.0, 20.0, 14.0), 1.0);
        let g = adaptive_grid(1.0, 1.0, 14.0);
        assert!((14.0..=50.0).contains(&g), "{g}");
    }
}
