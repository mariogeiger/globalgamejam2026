use std::collections::HashMap;

use crate::gpu::{
    create_index_buffer, create_placeholder_bind_group, create_texture_with_bind_group,
    create_vertex_buffer, texture_bind_group_layout,
};
use crate::mesh::{Mesh, Vertex};

use super::traits::Renderable;

pub struct MapRenderData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bind_group: wgpu::BindGroup,
}

pub struct MapRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub meshes: Vec<MapRenderData>,
}

impl MapRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        mesh: &Mesh,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Map Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });

        let texture_layout = texture_bind_group_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Map Pipeline Layout"),
            bind_group_layouts: &[camera_layout, &texture_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Map Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Map Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let mut gpu_textures: HashMap<String, wgpu::BindGroup> = HashMap::new();
        for (name, tex_data) in &mesh.textures {
            let (_, _, bg) = create_texture_with_bind_group(
                device,
                queue,
                &texture_layout,
                &sampler,
                &tex_data.rgba,
                tex_data.width,
                tex_data.height,
                name,
            );
            gpu_textures.insert(name.clone(), bg);
        }

        let placeholder = create_placeholder_bind_group(device, queue, &texture_layout, &sampler);

        let meshes: Vec<_> = mesh
            .submeshes
            .iter()
            .filter(|s| !s.vertices.is_empty() && !s.indices.is_empty())
            .map(|submesh| MapRenderData {
                vertex_buffer: create_vertex_buffer(
                    device,
                    &submesh.vertices,
                    &submesh.texture_name,
                ),
                index_buffer: create_index_buffer(device, &submesh.indices, &submesh.texture_name),
                index_count: submesh.indices.len() as u32,
                bind_group: gpu_textures
                    .get(&submesh.texture_name)
                    .cloned()
                    .unwrap_or_else(|| placeholder.clone()),
            })
            .collect();

        Self { pipeline, meshes }
    }
}

impl Renderable for MapRenderer {
    fn render<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bind_group: &'a wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);

        for mesh in &self.meshes {
            pass.set_bind_group(1, &mesh.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}
