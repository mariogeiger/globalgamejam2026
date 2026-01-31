use std::collections::HashMap;
use glam::Vec3;
use gltf::image::Format;

use crate::map::{MapVertex, MapMesh, TextureData, LoadedMap};

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
    let (document, buffers, images) = gltf::import_slice(data)
        .map_err(|e| format!("Failed to load GLB from bytes: {}", e))?;
    
    load_glb_data(document, buffers, images)
}

fn load_glb_data(
    document: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
    images: Vec<gltf::image::Data>,
) -> Result<LoadedMap, String> {
    
    let mut meshes: Vec<MapMesh> = Vec::new();
    let mut textures: HashMap<String, TextureData> = HashMap::new();
    let mut collision_vertices: Vec<Vec3> = Vec::new();
    let mut collision_indices: Vec<[u32; 3]> = Vec::new();
    
    // Load all images as textures
    for (idx, image) in images.iter().enumerate() {
        let tex_name = format!("texture_{}", idx);
        let rgba = convert_image_to_rgba(image);
        textures.insert(tex_name, TextureData {
            width: image.width,
            height: image.height,
            rgba,
        });
    }
    
    // Process all meshes
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            
            // Get positions
            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .map(|iter| iter.collect())
                .unwrap_or_default();
            
            if positions.is_empty() {
                continue;
            }
            
            // Get normals (or generate default)
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|iter| iter.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            
            // Get texture coordinates
            let tex_coords: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|iter| iter.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
            
            // Get indices
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|iter| iter.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            
            // Build vertices
            let vertices: Vec<MapVertex> = positions
                .iter()
                .zip(tex_coords.iter())
                .zip(normals.iter())
                .map(|((pos, tex), norm)| {
                    // Flip Y axis (upside-down fix) and X axis (mirror fix)
                    MapVertex {
                        position: [-pos[0], -pos[1], pos[2]],
                        tex_coord: *tex,
                        normal: [-norm[0], -norm[1], norm[2]],
                    }
                })
                .collect();
            
            // Get material/texture name
            let texture_name = if let Some(material) = primitive.material().pbr_metallic_roughness()
                .base_color_texture()
            {
                format!("texture_{}", material.texture().source().index())
            } else if let Some(mat) = primitive.material().index() {
                format!("material_{}", mat)
            } else {
                "default".to_string()
            };
            
            meshes.push(MapMesh {
                vertices: vertices.clone(),
                indices: indices.clone(),
                texture_name,
            });
            
            // Add to collision geometry (with Y and X flip)
            let base_idx = collision_vertices.len() as u32;
            collision_vertices.extend(positions.iter().map(|p| Vec3::new(-p[0], -p[1], p[2])));
            
            for chunk in indices.chunks(3) {
                if chunk.len() == 3 {
                    collision_indices.push([
                        base_idx + chunk[0],
                        base_idx + chunk[1],
                        base_idx + chunk[2],
                    ]);
                }
            }
        }
    }
    
    // Calculate map bounds
    let mut bounds_min = Vec3::splat(f32::MAX);
    let mut bounds_max = Vec3::splat(f32::MIN);
    for v in &collision_vertices {
        bounds_min = bounds_min.min(*v);
        bounds_max = bounds_max.max(*v);
    }
    log::info!("Map bounds: min={:?}, max={:?}", bounds_min, bounds_max);
    
    // Build spawn points from both teams
    // spawn.json: [x, forward, height] at 1/64 scale of GLB units
    // We flip X and Y in the loader
    const SPAWN_SCALE: f32 = 64.0;
    let spawn_points: Vec<Vec3> = SPAWNS_TEAM_A.iter()
        .chain(SPAWNS_TEAM_B.iter())
        .map(|spawn| Vec3::new(
            -spawn[0] * SPAWN_SCALE,  // Flip X to match mirrored map
            spawn[2] * SPAWN_SCALE,
            spawn[1] * SPAWN_SCALE,
        ))
        .collect();
    
    log::info!("Loaded GLB: {} meshes, {} textures, {} collision triangles, {} spawn points",
        meshes.len(), textures.len(), collision_indices.len(), spawn_points.len());
    
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
        Format::R8G8B8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() / 3 * 4);
            for chunk in image.pixels.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        Format::R8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
            for &gray in &image.pixels {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
            rgba
        }
        Format::R8G8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() * 2);
            for chunk in image.pixels.chunks(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            rgba
        }
        Format::R16 | Format::R16G16 | Format::R16G16B16 | Format::R16G16B16A16 => {
            let bytes_per_channel = 2;
            let channels = match image.format {
                Format::R16 => 1,
                Format::R16G16 => 2,
                Format::R16G16B16 => 3,
                Format::R16G16B16A16 => 4,
                _ => 4,
            };
            let pixel_count = image.pixels.len() / (bytes_per_channel * channels);
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            
            for i in 0..pixel_count {
                let base = i * bytes_per_channel * channels;
                for c in 0..3.min(channels) {
                    let val = u16::from_le_bytes([
                        image.pixels[base + c * 2],
                        image.pixels[base + c * 2 + 1],
                    ]);
                    rgba.push((val >> 8) as u8);
                }
                for _ in channels..3 {
                    rgba.push(if channels == 1 { rgba[rgba.len() - 1] } else { 0 });
                }
                if channels == 4 {
                    let val = u16::from_le_bytes([
                        image.pixels[base + 6],
                        image.pixels[base + 7],
                    ]);
                    rgba.push((val >> 8) as u8);
                } else {
                    rgba.push(255);
                }
            }
            rgba
        }
        Format::R32G32B32FLOAT | Format::R32G32B32A32FLOAT => {
            let channels = if matches!(image.format, Format::R32G32B32FLOAT) { 3 } else { 4 };
            let pixel_count = image.pixels.len() / (4 * channels);
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            
            for i in 0..pixel_count {
                let base = i * 4 * channels;
                for c in 0..3.min(channels) {
                    let bytes = [
                        image.pixels[base + c * 4],
                        image.pixels[base + c * 4 + 1],
                        image.pixels[base + c * 4 + 2],
                        image.pixels[base + c * 4 + 3],
                    ];
                    let val = f32::from_le_bytes(bytes);
                    rgba.push((val.clamp(0.0, 1.0) * 255.0) as u8);
                }
                for _ in channels..3 {
                    rgba.push(rgba[rgba.len() - 1]);
                }
                if channels == 4 {
                    let bytes = [
                        image.pixels[base + 12],
                        image.pixels[base + 13],
                        image.pixels[base + 14],
                        image.pixels[base + 15],
                    ];
                    let val = f32::from_le_bytes(bytes);
                    rgba.push((val.clamp(0.0, 1.0) * 255.0) as u8);
                } else {
                    rgba.push(255);
                }
            }
            rgba
        }
    }
}
