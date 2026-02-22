//! 3D renderer for DualSense spatial state visualization
//!
//! Uses wgpu to render the controller orientation as a 3D box,
//! with velocity and acceleration vectors displayed as arrows.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

use crate::spatial::SpatialState;

/// Vertex format for 3D rendering
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
    ];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Uniform buffer for transformation matrices
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
}

/// 3D Renderer state
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    // Controller box
    box_vertex_buffer: wgpu::Buffer,
    box_index_buffer: wgpu::Buffer,
    box_num_indices: u32,
    // Velocity arrow
    arrow_vertex_buffer: wgpu::Buffer,
    arrow_num_vertices: u32,
    // Acceleration arrow
    accel_arrow_vertex_buffer: wgpu::Buffer,
    accel_arrow_num_vertices: u32,
    // Grid lines
    grid_vertex_buffer: wgpu::Buffer,
    grid_num_vertices: u32,
    // Uniforms
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    // Window reference
    window: Arc<Window>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: None,
                },
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Create uniform buffer
        let uniforms = Uniforms {
            view_proj: identity_matrix(),
            model: identity_matrix(),
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("uniform_bind_group_layout"),
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
            label: Some("uniform_bind_group"),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&uniform_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Disable culling to see all faces
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        // Create controller box vertices (colored cube)
        let (box_vertices, box_indices) = create_box_mesh();
        let box_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Box Vertex Buffer"),
            contents: bytemuck::cast_slice(&box_vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let box_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Box Index Buffer"),
            contents: bytemuck::cast_slice(&box_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let box_num_indices = box_indices.len() as u32;

        // Create velocity arrow (green)
        let arrow_vertices = create_arrow_mesh([0.0, 0.8, 0.2]);
        let arrow_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Arrow Vertex Buffer"),
            contents: bytemuck::cast_slice(&arrow_vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let arrow_num_vertices = arrow_vertices.len() as u32;

        // Create acceleration arrow (red/orange)
        let accel_arrow_vertices = create_arrow_mesh([1.0, 0.4, 0.1]);
        let accel_arrow_vertex_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Accel Arrow Vertex Buffer"),
                contents: bytemuck::cast_slice(&accel_arrow_vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let accel_arrow_num_vertices = accel_arrow_vertices.len() as u32;

        // Create grid lines
        let grid_vertices = create_grid_mesh();
        let grid_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Grid Vertex Buffer"),
            contents: bytemuck::cast_slice(&grid_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let grid_num_vertices = grid_vertices.len() as u32;

        Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            box_vertex_buffer,
            box_index_buffer,
            box_num_indices,
            arrow_vertex_buffer,
            arrow_num_vertices,
            accel_arrow_vertex_buffer,
            accel_arrow_num_vertices,
            grid_vertex_buffer,
            grid_num_vertices,
            uniform_buffer,
            uniform_bind_group,
            window,
        }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn render(&mut self, spatial: &SpatialState) -> Result<(), wgpu::SurfaceError> {
        // Ensure surface is configured with current size
        let current_size = self.window.inner_size();
        if current_size.width != self.size.width || current_size.height != self.size.height {
            self.resize(current_size);
        }

        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Create model matrix from quaternion orientation
        let quat = spatial.orientation();
        let model = quaternion_to_matrix(quat.w, quat.x, quat.y, quat.z);

        // For now, use identity view-proj to verify orientation works
        // The model matrix rotates the box based on controller orientation
        let view_proj = identity_matrix();

        let uniforms = Uniforms { view_proj, model };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Update velocity arrow based on spatial velocity
        let vel = spatial.velocity;
        let vel_mag = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        let vel_scale = (vel_mag / 200.0).min(2.0); // Scale based on max speed
        let vel_dir = if vel_mag > 0.1 {
            [vel[0] / vel_mag, vel[1] / vel_mag, vel[2] / vel_mag]
        } else {
            [0.0, 1.0, 0.0]
        };
        let arrow_vertices = create_oriented_arrow_mesh([0.0, 0.8, 0.2], vel_dir, vel_scale);
        self.queue.write_buffer(
            &self.arrow_vertex_buffer,
            0,
            bytemuck::cast_slice(&arrow_vertices),
        );

        // Update acceleration arrow based on linear acceleration
        let accel = spatial.linear_accel;
        let accel_mag = (accel[0] * accel[0] + accel[1] * accel[1] + accel[2] * accel[2]).sqrt();
        let accel_scale = (accel_mag / 2.0).min(2.0); // Scale based on typical G range
        let accel_dir = if accel_mag > 0.01 {
            [
                accel[0] / accel_mag,
                accel[1] / accel_mag,
                accel[2] / accel_mag,
            ]
        } else {
            [0.0, -1.0, 0.0]
        };
        let accel_arrow_vertices =
            create_oriented_arrow_mesh([1.0, 0.4, 0.1], accel_dir, accel_scale);
        self.queue.write_buffer(
            &self.accel_arrow_vertex_buffer,
            0,
            bytemuck::cast_slice(&accel_arrow_vertices),
        );

        // Update uniforms with view-projection and model matrices
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.1,
                            b: 0.15,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);

            // Draw controller box with orientation
            render_pass.set_vertex_buffer(0, self.box_vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.box_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.box_num_indices, 0, 0..1);

            // Draw velocity arrow
            if vel_mag > 0.5 {
                render_pass.set_vertex_buffer(0, self.arrow_vertex_buffer.slice(..));
                render_pass.draw(0..self.arrow_num_vertices, 0..1);
            }

            // Draw acceleration arrow
            if accel_mag > 0.05 {
                render_pass.set_vertex_buffer(0, self.accel_arrow_vertex_buffer.slice(..));
                render_pass.draw(0..self.accel_arrow_num_vertices, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

/// Create a colored box mesh representing the controller
fn create_box_mesh() -> (Vec<Vertex>, Vec<u16>) {
    // Controller-like proportions: wider than tall, thin depth
    let w = 0.8; // width (X)
    let h = 0.4; // height (Y)
    let d = 0.2; // depth (Z)

    // Colors for each face (distinct to show orientation)
    let front_color = [0.2, 0.5, 1.0]; // Blue - front
    let back_color = [0.3, 0.3, 0.4]; // Gray - back
    let top_color = [0.1, 0.8, 0.3]; // Green - top
    let bottom_color = [0.8, 0.2, 0.2]; // Red - bottom
    let right_color = [0.9, 0.6, 0.1]; // Orange - right
    let left_color = [0.7, 0.2, 0.8]; // Purple - left

    let vertices = vec![
        // Front face (Z+)
        Vertex { position: [-w, -h, d], color: front_color },
        Vertex { position: [w, -h, d], color: front_color },
        Vertex { position: [w, h, d], color: front_color },
        Vertex { position: [-w, h, d], color: front_color },
        // Back face (Z-)
        Vertex { position: [w, -h, -d], color: back_color },
        Vertex { position: [-w, -h, -d], color: back_color },
        Vertex { position: [-w, h, -d], color: back_color },
        Vertex { position: [w, h, -d], color: back_color },
        // Top face (Y+)
        Vertex { position: [-w, h, d], color: top_color },
        Vertex { position: [w, h, d], color: top_color },
        Vertex { position: [w, h, -d], color: top_color },
        Vertex { position: [-w, h, -d], color: top_color },
        // Bottom face (Y-)
        Vertex { position: [-w, -h, -d], color: bottom_color },
        Vertex { position: [w, -h, -d], color: bottom_color },
        Vertex { position: [w, -h, d], color: bottom_color },
        Vertex { position: [-w, -h, d], color: bottom_color },
        // Right face (X+)
        Vertex { position: [w, -h, d], color: right_color },
        Vertex { position: [w, -h, -d], color: right_color },
        Vertex { position: [w, h, -d], color: right_color },
        Vertex { position: [w, h, d], color: right_color },
        // Left face (X-)
        Vertex { position: [-w, -h, -d], color: left_color },
        Vertex { position: [-w, -h, d], color: left_color },
        Vertex { position: [-w, h, d], color: left_color },
        Vertex { position: [-w, h, -d], color: left_color },
    ];

    let indices: Vec<u16> = vec![
        0, 1, 2, 2, 3, 0, // front
        4, 5, 6, 6, 7, 4, // back
        8, 9, 10, 10, 11, 8, // top
        12, 13, 14, 14, 15, 12, // bottom
        16, 17, 18, 18, 19, 16, // right
        20, 21, 22, 22, 23, 20, // left
    ];

    (vertices, indices)
}

/// Create an arrow mesh pointing in +Y direction (will be rotated)
fn create_arrow_mesh(color: [f32; 3]) -> Vec<Vertex> {
    create_oriented_arrow_mesh(color, [0.0, 1.0, 0.0], 1.0)
}

/// Create an arrow mesh pointing in a given direction with scale
fn create_oriented_arrow_mesh(color: [f32; 3], direction: [f32; 3], scale: f32) -> Vec<Vertex> {
    let len = 0.8 * scale;
    let shaft_radius = 0.03;
    let head_radius = 0.08;
    let head_len = 0.15 * scale.min(1.0);

    // Normalize direction
    let mag = (direction[0] * direction[0]
        + direction[1] * direction[1]
        + direction[2] * direction[2])
    .sqrt();
    let dir = if mag > 0.001 {
        [
            direction[0] / mag,
            direction[1] / mag,
            direction[2] / mag,
        ]
    } else {
        [0.0, 1.0, 0.0]
    };

    // Create orthonormal basis
    let up = if dir[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };

    let right = [
        up[1] * dir[2] - up[2] * dir[1],
        up[2] * dir[0] - up[0] * dir[2],
        up[0] * dir[1] - up[1] * dir[0],
    ];
    let right_mag = (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt();
    let right = [
        right[0] / right_mag,
        right[1] / right_mag,
        right[2] / right_mag,
    ];

    let up = [
        dir[1] * right[2] - dir[2] * right[1],
        dir[2] * right[0] - dir[0] * right[2],
        dir[0] * right[1] - dir[1] * right[0],
    ];

    let mut vertices = Vec::new();
    let segments = 8;

    // Arrow shaft (cylinder along direction)
    for i in 0..segments {
        let angle1 = (i as f32) * 2.0 * std::f32::consts::PI / segments as f32;
        let angle2 = ((i + 1) as f32) * 2.0 * std::f32::consts::PI / segments as f32;

        let (c1, s1) = (angle1.cos(), angle1.sin());
        let (c2, s2) = (angle2.cos(), angle2.sin());

        // Bottom ring point 1
        let b1 = [
            right[0] * c1 * shaft_radius + up[0] * s1 * shaft_radius,
            right[1] * c1 * shaft_radius + up[1] * s1 * shaft_radius,
            right[2] * c1 * shaft_radius + up[2] * s1 * shaft_radius,
        ];
        // Bottom ring point 2
        let b2 = [
            right[0] * c2 * shaft_radius + up[0] * s2 * shaft_radius,
            right[1] * c2 * shaft_radius + up[1] * s2 * shaft_radius,
            right[2] * c2 * shaft_radius + up[2] * s2 * shaft_radius,
        ];

        let shaft_end = len - head_len;
        // Top ring point 1
        let t1 = [
            b1[0] + dir[0] * shaft_end,
            b1[1] + dir[1] * shaft_end,
            b1[2] + dir[2] * shaft_end,
        ];
        // Top ring point 2
        let t2 = [
            b2[0] + dir[0] * shaft_end,
            b2[1] + dir[1] * shaft_end,
            b2[2] + dir[2] * shaft_end,
        ];

        // Shaft triangles
        vertices.push(Vertex { position: b1, color });
        vertices.push(Vertex { position: b2, color });
        vertices.push(Vertex { position: t1, color });

        vertices.push(Vertex { position: t1, color });
        vertices.push(Vertex { position: b2, color });
        vertices.push(Vertex { position: t2, color });
    }

    // Arrow head (cone)
    let head_base = [
        dir[0] * (len - head_len),
        dir[1] * (len - head_len),
        dir[2] * (len - head_len),
    ];
    let tip = [dir[0] * len, dir[1] * len, dir[2] * len];

    for i in 0..segments {
        let angle1 = (i as f32) * 2.0 * std::f32::consts::PI / segments as f32;
        let angle2 = ((i + 1) as f32) * 2.0 * std::f32::consts::PI / segments as f32;

        let (c1, s1) = (angle1.cos(), angle1.sin());
        let (c2, s2) = (angle2.cos(), angle2.sin());

        let p1 = [
            head_base[0] + right[0] * c1 * head_radius + up[0] * s1 * head_radius,
            head_base[1] + right[1] * c1 * head_radius + up[1] * s1 * head_radius,
            head_base[2] + right[2] * c1 * head_radius + up[2] * s1 * head_radius,
        ];
        let p2 = [
            head_base[0] + right[0] * c2 * head_radius + up[0] * s2 * head_radius,
            head_base[1] + right[1] * c2 * head_radius + up[1] * s2 * head_radius,
            head_base[2] + right[2] * c2 * head_radius + up[2] * s2 * head_radius,
        ];

        // Cone triangle
        vertices.push(Vertex { position: p1, color });
        vertices.push(Vertex { position: p2, color });
        vertices.push(Vertex { position: tip, color });

        // Base cap
        vertices.push(Vertex {
            position: head_base,
            color,
        });
        vertices.push(Vertex { position: p1, color });
        vertices.push(Vertex { position: p2, color });
    }

    vertices
}

/// Create a simple grid for reference
fn create_grid_mesh() -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let color = [0.3, 0.3, 0.35];
    let grid_size = 3.0;
    let step = 0.5;
    let y = -1.5; // Below the controller

    // Grid lines along X
    let mut x = -grid_size;
    while x <= grid_size {
        // Each line as two thin triangles (line rendering not great in basic wgpu)
        let thickness = 0.01;
        vertices.push(Vertex {
            position: [x - thickness, y, -grid_size],
            color,
        });
        vertices.push(Vertex {
            position: [x + thickness, y, -grid_size],
            color,
        });
        vertices.push(Vertex {
            position: [x + thickness, y, grid_size],
            color,
        });

        vertices.push(Vertex {
            position: [x - thickness, y, -grid_size],
            color,
        });
        vertices.push(Vertex {
            position: [x + thickness, y, grid_size],
            color,
        });
        vertices.push(Vertex {
            position: [x - thickness, y, grid_size],
            color,
        });

        x += step;
    }

    // Grid lines along Z
    let mut z = -grid_size;
    while z <= grid_size {
        let thickness = 0.01;
        vertices.push(Vertex {
            position: [-grid_size, y, z - thickness],
            color,
        });
        vertices.push(Vertex {
            position: [-grid_size, y, z + thickness],
            color,
        });
        vertices.push(Vertex {
            position: [grid_size, y, z + thickness],
            color,
        });

        vertices.push(Vertex {
            position: [-grid_size, y, z - thickness],
            color,
        });
        vertices.push(Vertex {
            position: [grid_size, y, z + thickness],
            color,
        });
        vertices.push(Vertex {
            position: [grid_size, y, z - thickness],
            color,
        });

        z += step;
    }

    vertices
}

/// Create identity 4x4 matrix
fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0001 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Create view-projection matrix with simple orbit camera
fn create_view_proj_matrix(aspect: f32, distance: f32, pitch: f32, yaw: f32) -> [[f32; 4]; 4] {
    // Camera position (orbit around origin, looking at center)
    // Camera sits at positive Z, looking toward origin
    let cam_x = distance * yaw.sin() * pitch.cos();
    let cam_y = distance * pitch.sin();
    let cam_z = distance * yaw.cos() * pitch.cos();

    // Simple orthographic-like projection that just scales the scene
    // This gives us a predictable result
    let scale = 0.5; // Scale factor to fit the box in view

    // Translation to move camera back
    let tx = -cam_x * scale;
    let ty = -cam_y * scale;
    let tz = -cam_z * scale;

    // Combined view-projection: scale and translate
    // This is a simple approach that works for visualization
    [
        [scale / aspect, 0.0, 0.0, 0.0],
        [0.0, scale, 0.0, 0.0],
        [0.0, 0.0, scale * 0.1, 0.0], // Compress Z for visibility
        [tx, ty, tz, 1.0],
    ]
}

/// Convert quaternion to rotation matrix
fn quaternion_to_matrix(w: f32, x: f32, y: f32, z: f32) -> [[f32; 4]; 4] {
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let wx = w * x;
    let wy = w * y;
    let wz = w * z;

    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy + wz), 2.0 * (xz - wy), 0.0],
        [2.0 * (xy - wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz + wx), 0.0],
        [2.0 * (xz + wy), 2.0 * (yz - wx), 1.0 - 2.0 * (xx + yy), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Multiply two 4x4 matrices
fn multiply_matrices(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                result[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    result
}

/// Run the 3D visualization window
pub fn run_3d_visualization(
    controller_receiver: std::sync::mpsc::Receiver<SpatialState>,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("DualSense 3D Visualization")
            .with_inner_size(PhysicalSize::new(800, 600))
            .build(&event_loop)
            .unwrap(),
    );

    let mut renderer = pollster::block_on(Renderer::new(window.clone()));
    let mut spatial_state = SpatialState::new(crate::spatial::IntegrationConfig::default());

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);

            // Try to receive updated spatial state
            while let Ok(state) = controller_receiver.try_recv() {
                spatial_state = state;
            }

            match event {
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == renderer.window().id() => match event {
                    WindowEvent::CloseRequested => {
                        elwt.exit();
                    }
                    WindowEvent::Resized(physical_size) => {
                        renderer.resize(*physical_size);
                    }
                    WindowEvent::RedrawRequested => {
                        match renderer.render(&spatial_state) {
                            Ok(_) => {}
                            Err(wgpu::SurfaceError::Lost) => renderer.resize(renderer.size),
                            Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                            Err(e) => eprintln!("Render error: {:?}", e),
                        }
                    }
                    _ => {}
                },
                Event::AboutToWait => {
                    renderer.window().request_redraw();
                }
                _ => {}
            }
        })
        .unwrap();

    Ok(())
}
