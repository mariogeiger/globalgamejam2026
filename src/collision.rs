use glam::Vec3;
use parry3d::math::{Pose3, Vector};
use parry3d::query::{Ray, RayCast};
use parry3d::shape::TriMesh;

use crate::config::*;

pub struct PhysicsWorld {
    trimesh: TriMesh,
}

impl PhysicsWorld {
    pub fn new(collision_vertices: &[Vec3], collision_indices: &[[u32; 3]]) -> Option<Self> {
        if collision_vertices.is_empty() || collision_indices.is_empty() {
            return None;
        }

        // parry3d 0.26 uses glam Vec3 directly
        let vertices: Vec<Vector> = collision_vertices
            .iter()
            .map(|v| Vector::new(v.x, v.y, v.z))
            .collect();

        let trimesh =
            TriMesh::new(vertices, collision_indices.to_vec()).expect("Failed to create trimesh");

        Some(Self { trimesh })
    }

    fn cast_ray(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<f32> {
        let ray = Ray::new(
            Vector::new(origin.x, origin.y, origin.z),
            Vector::new(dir.x, dir.y, dir.z),
        );
        // Pose3::IDENTITY is the identity transformation
        self.trimesh
            .cast_ray(&Pose3::IDENTITY, &ray, max_dist, true)
    }

    pub fn move_player(&self, desired_position: Vec3, velocity_y: f32) -> (Vec3, bool, bool) {
        let mut final_pos = desired_position;
        let mut on_ground = false;
        let mut hit_ceiling = false;

        // Ground check
        let ground_origin = desired_position + Vec3::new(0.0, GROUND_CHECK_OFFSET, 0.0);
        if let Some(toi) = self.cast_ray(ground_origin, Vec3::NEG_Y, GROUND_CHECK_MAX) {
            if toi < GROUND_HIT_THRESHOLD {
                on_ground = true;
                let ground_y = ground_origin.y - toi;
                if final_pos.y < ground_y {
                    final_pos.y = ground_y;
                }
            }
        }

        // Ceiling check (only when moving up)
        if velocity_y > 0.0 {
            let ceiling_origin = desired_position + Vec3::new(0.0, CEILING_CHECK_OFFSET, 0.0);
            if let Some(toi) = self.cast_ray(ceiling_origin, Vec3::Y, CEILING_CHECK_MAX) {
                if toi < CEILING_HIT_THRESHOLD {
                    hit_ceiling = true;
                    let ceiling_y = ceiling_origin.y + toi - PLAYER_HEIGHT - 1.0;
                    if final_pos.y > ceiling_y {
                        final_pos.y = ceiling_y;
                    }
                }
            }
        }

        // Wall checks (4 cardinal directions)
        let wall_origin = desired_position + Vec3::new(0.0, WALL_CHECK_OFFSET, 0.0);
        for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let dir = Vec3::new(dx, 0.0, dz);
            if let Some(toi) = self.cast_ray(wall_origin, dir, WALL_CHECK_DIST) {
                if toi < WALL_CHECK_DIST {
                    final_pos.x -= dx * (WALL_CHECK_DIST - toi);
                    final_pos.z -= dz * (WALL_CHECK_DIST - toi);
                }
            }
        }

        (final_pos, on_ground, hit_ceiling)
    }
}
