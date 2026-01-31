use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::gpu::{camera_bind_group_layout, create_uniform_buffer};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
}

pub struct CameraState {
    pub uniform_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

impl CameraState {
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        };
        let uniform_buffer = create_uniform_buffer(device, &uniform, "Camera Uniform");

        let layout = camera_bind_group_layout(device);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            uniform_buffer,
            bind_group,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform {
                view_proj: view_proj.to_cols_array_2d(),
            }]),
        );
    }
}
