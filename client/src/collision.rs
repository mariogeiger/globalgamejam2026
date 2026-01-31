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

        let vertices: Vec<Vector> = collision_vertices
            .iter()
            .map(|v| Vector::new(v.x, v.y, v.z))
            .collect();

        let trimesh = TriMesh::new(vertices, collision_indices.to_vec()).ok()?;
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

    /// Raycast from previous position toward next; if geometry is hit along the segment,
    /// return a position clamped to just before the hit (anti-tunnelling).
    /// Uses two rays (step height and head height) and clamps to the earliest hit.
    pub fn clamp_desired_to_path(&self, prev_pos: Vec3, next_pos: Vec3) -> Vec3 {
        let delta = next_pos - prev_pos;
        let len = delta.length();
        if len <= 1e-6 {
            return next_pos;
        }
        let dir = delta / len;
        let max_dist = len;

        // Ignore hits very close to origin (already inside or on surface)
        const MIN_TOI: f32 = 0.5;

        let mut min_hit = max_dist + 1.0;
        for height in [STEP_OVER_HEIGHT, PLAYER_HEIGHT] {
            let origin = prev_pos + Vec3::new(0.0, height, 0.0);
            if let Some(toi) = self.cast_ray(origin, dir, max_dist) {
                if toi > MIN_TOI && toi < min_hit {
                    min_hit = toi;
                }
            }
        }

        if min_hit <= max_dist {
            let safe_dist = (min_hit - PATH_HIT_MARGIN).max(0.0);
            prev_pos + dir * safe_dist
        } else {
            next_pos
        }
    }

    pub fn move_player(&self, desired_position: Vec3, velocity: Vec3) -> (Vec3, bool) {
        let mut final_pos = desired_position;
        let mut on_ground = false;
        let half_width = PLAYER_WIDTH / 2.0;

        // Ground check
        let ground_origin = desired_position + Vec3::new(0.0, STEP_OVER_HEIGHT, 0.0);
        if let Some(toi) = self.cast_ray(ground_origin, Vec3::NEG_Y, PLAYER_HEIGHT)
            && toi < STEP_OVER_HEIGHT + GROUND_SNAP_MARGIN
        {
            on_ground = true;
            let ground_y = ground_origin.y - toi;
            if final_pos.y < ground_y {
                final_pos.y = ground_y;
            }
        }

        // Wall checks (4 directions, 2 heights: step-over and head)
        for height in [STEP_OVER_HEIGHT, PLAYER_HEIGHT] {
            let wall_origin = final_pos + Vec3::new(0.0, height, 0.0);
            for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let dir = Vec3::new(dx, 0.0, dz);
                if let Some(toi) = self.cast_ray(wall_origin, dir, half_width)
                    && toi < half_width
                {
                    final_pos.x -= dx * (half_width - toi);
                    final_pos.z -= dz * (half_width - toi);
                }
            }
        }

        // Ceiling check (from eye position, only when moving up)
        if velocity.y > 0.0 {
            let head_clearance = PLAYER_HEIGHT - EYE_HEIGHT;
            let eye_origin = desired_position + Vec3::new(0.0, EYE_HEIGHT, 0.0);
            if let Some(toi) = self.cast_ray(eye_origin, Vec3::Y, head_clearance)
                && toi < head_clearance
            {
                final_pos.y -= head_clearance - toi;
            }
        }

        (final_pos, on_ground)
    }
}
