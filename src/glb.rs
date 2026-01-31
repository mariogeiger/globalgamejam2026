use glam::Vec3;
use gltf::image::Format;
use std::collections::HashMap;

use crate::config::SPAWN_SCALE;
use crate::map::{LoadedMap, MapMesh, MapVertex, TextureData};

pub const SPAWNS_TEAM_A: &[[f32; 3]] = &[
    [-8.0, -10.0, 4.0],
    [-10.0, -10.0, 4.0],
    [-14.0, -10.0, 4.0],
    [-16.0, -10.0, 4.0],
    [-18.0, -10.0, 4.0],
    [-8.0, -12.0, 4.0],
    [-10.0, -12.0, 4.0],
    [-14.0, -12.0, 4.0],
    [-16.0, -12.0, 4.0],
    [-18.0, -12.0, 4.0],
];

pub const SPAWNS_TEAM_B: &[[f32; 3]] = &[
    [5.5, 32.0, 0.0],
    [3.5, 32.0, 0.0],
    [5.5, 30.0, 0.0],
    [3.5, 30.0, 0.0],
    [0.5, 29.0, 0.0],
    [-1.5, 29.0, 0.0],
    [7.5, 29.0, 0.0],
    [9.5, 29.0, 1.0],
    [11.5, 29.0, 2.0],
    [13.5, 29.0, 2.0],
];

pub fn load_glb_from_bytes(data: &[u8]) -> Result<LoadedMap, String> {
    let (document, buffers, images) =
        gltf::import_slice(data).map_err(|e| format!("Failed to load GLB: {}", e))?;

    let mut meshes: Vec<MapMesh> = Vec::new();
    let mut textures: HashMap<String, TextureData> = HashMap::new();
    let mut collision_vertices: Vec<Vec3> = Vec::new();
    let mut collision_indices: Vec<[u32; 3]> = Vec::new();

    // Load textures
    for (idx, image) in images.iter().enumerate() {
        textures.insert(
            format!("texture_{}", idx),
            TextureData {
                width: image.width,
                height: image.height,
                rgba: convert_image_to_rgba(image),
            },
        );
    }

    // Process meshes
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .map(|i| i.collect())
                .unwrap_or_default();
            if positions.is_empty() {
                continue;
            }

            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|i| i.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);

            let tex_coords: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|i| i.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|i| i.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            // Build vertices with coordinate flip
            let vertices: Vec<MapVertex> = positions
                .iter()
                .zip(&tex_coords)
                .zip(&normals)
                .map(|((pos, tex), norm)| MapVertex {
                    position: [-pos[0], -pos[1], pos[2]],
                    tex_coord: *tex,
                    normal: [-norm[0], -norm[1], norm[2]],
                })
                .collect();

            let texture_name = primitive
                .material()
                .pbr_metallic_roughness()
                .base_color_texture()
                .map(|t| format!("texture_{}", t.texture().source().index()))
                .or_else(|| {
                    primitive
                        .material()
                        .index()
                        .map(|i| format!("material_{}", i))
                })
                .unwrap_or_else(|| "default".to_string());

            meshes.push(MapMesh {
                vertices: vertices.clone(),
                indices: indices.clone(),
                texture_name,
            });

            // Collision geometry
            let base_idx = collision_vertices.len() as u32;
            collision_vertices.extend(positions.iter().map(|p| Vec3::new(-p[0], -p[1], p[2])));
            collision_indices.extend(indices.chunks(3).filter_map(|c| {
                (c.len() == 3).then(|| [base_idx + c[0], base_idx + c[1], base_idx + c[2]])
            }));
        }
    }

    // Calculate bounds
    let (bounds_min, bounds_max) = collision_vertices.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(min, max), v| (min.min(*v), max.max(*v)),
    );

    // Build spawn points
    let spawn_points: Vec<Vec3> = SPAWNS_TEAM_A
        .iter()
        .chain(SPAWNS_TEAM_B.iter())
        .map(|s| Vec3::new(-s[0] * SPAWN_SCALE, s[2] * SPAWN_SCALE, s[1] * SPAWN_SCALE))
        .collect();

    log::info!(
        "Loaded GLB: {} meshes, {} textures, {} triangles, {} spawns",
        meshes.len(),
        textures.len(),
        collision_indices.len(),
        spawn_points.len()
    );

    Ok(LoadedMap {
        meshes,
        textures,
        spawn_points,
        collision_vertices,
        collision_indices,
        bounds_min,
        bounds_max,
    })
}

fn convert_image_to_rgba(image: &gltf::image::Data) -> Vec<u8> {
    match image.format {
        Format::R8G8B8A8 => image.pixels.clone(),
        Format::R8G8B8 => image
            .pixels
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        Format::R8 => image.pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        Format::R8G8 => image
            .pixels
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        _ => {
            // Fallback: magenta placeholder for unsupported formats
            log::warn!(
                "Unsupported image format {:?}, using placeholder",
                image.format
            );
            vec![255, 0, 255, 255].repeat((image.width * image.height) as usize)
        }
    }
}
