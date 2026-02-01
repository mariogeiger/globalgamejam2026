use gltf::image::Format;
use std::collections::HashMap;

use crate::mesh::{Mesh, SubMesh, TextureData, Vertex};

/// Coordinate transform function type
pub type CoordTransform = fn([f32; 3]) -> [f32; 3];

/// Load a mesh from GLB bytes with optional coordinate transform
pub fn load_mesh_from_bytes(
    data: &[u8],
    transform: Option<CoordTransform>,
) -> Result<Mesh, String> {
    let (document, buffers, images) =
        gltf::import_slice(data).map_err(|e| format!("Failed to load GLB: {}", e))?;

    let mut submeshes = Vec::new();
    let mut textures: HashMap<String, TextureData> = HashMap::new();

    // Load all images as textures
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

    // Load all mesh primitives as submeshes
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let Some(positions) = reader.read_positions().map(|i| i.collect::<Vec<_>>()) else {
                continue;
            };
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

            let vertices: Vec<Vertex> = positions
                .iter()
                .zip(&tex_coords)
                .zip(&normals)
                .map(|((pos, tex), norm)| {
                    let (position, normal) = match transform {
                        Some(f) => (f(*pos), f(*norm)),
                        None => (*pos, *norm),
                    };
                    Vertex {
                        position,
                        tex_coord: *tex,
                        normal,
                    }
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

            submeshes.push(SubMesh {
                vertices,
                indices,
                texture_name,
            });
        }
    }

    log::info!(
        "Loaded mesh: {} submeshes, {} textures, transform: {}",
        submeshes.len(),
        textures.len(),
        transform.is_some()
    );

    Ok(Mesh {
        submeshes,
        textures,
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
            log::warn!(
                "Unsupported image format {:?}, using placeholder",
                image.format
            );
            [255, 0, 255, 255].repeat((image.width * image.height) as usize)
        }
    }
}
