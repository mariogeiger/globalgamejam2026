use glam::Vec3;
use parry3d::math::{Pose3, Vector};
use parry3d::query::{Ray, RayCast};
use parry3d::shape::TriMesh;

use crate::config::*;

/// Debug information from collision detection
#[derive(Clone, Default)]
pub struct CollisionDebug {
    pub on_ground: bool,
    pub ground_distance: Option<f32>,
    pub wall_distances: [Option<f32>; 4], // +X, -X, +Z, -Z
}

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
        self.trimesh
            .cast_ray(&Pose3::IDENTITY, &ray, max_dist, true)
    }

    /// Calculate dash target: raycast in direction, return safe destination
    pub fn dash_target(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Vec3 {
        const WALL_MARGIN: f32 = 20.0; // Stop before hitting wall

        match self.cast_ray(origin, direction, max_distance) {
            Some(hit_dist) => {
                // Hit something, stop before it
                let safe_dist = (hit_dist - WALL_MARGIN).max(0.0);
                origin + direction * safe_dist
            }
            None => {
                // No hit, go full distance
                origin + direction * max_distance
            }
        }
    }

    pub fn is_visible(&self, from: Vec3, to: Vec3) -> bool {
        let dir = to - from;
        let distance = dir.length();
        if distance < 0.001 {
            return true;
        }
        let dir_normalized = dir / distance;
        match self.cast_ray(from, dir_normalized, distance) {
            Some(hit_dist) => hit_dist >= distance - 1.0,
            None => true,
        }
    }

    pub fn clamp_desired_to_path(&self, prev_pos: Vec3, next_pos: Vec3) -> Vec3 {
        let delta = next_pos - prev_pos;
        let len = delta.length();
        if len <= 1e-6 {
            return next_pos;
        }
        let dir = delta / len;
        let max_dist = len;

        const MIN_TOI: f32 = 0.5;

        let mut min_hit = max_dist + 1.0;
        for height in [STEP_OVER_HEIGHT, PLAYER_HEIGHT] {
            let origin = prev_pos + Vec3::new(0.0, height, 0.0);
            if let Some(toi) = self.cast_ray(origin, dir, max_dist)
                && toi > MIN_TOI
                && toi < min_hit
            {
                min_hit = toi;
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

    /// Get debug information about collision state at a position
    pub fn get_debug_info(&self, position: Vec3) -> CollisionDebug {
        let half_width = PLAYER_WIDTH / 2.0;

        // Ground check
        let ground_origin = position + Vec3::new(0.0, STEP_OVER_HEIGHT, 0.0);
        let ground_distance = self.cast_ray(ground_origin, Vec3::NEG_Y, PLAYER_HEIGHT);
        let on_ground = ground_distance
            .map(|d| d < STEP_OVER_HEIGHT + GROUND_SNAP_MARGIN)
            .unwrap_or(false);

        // Wall distances in 4 directions: +X, -X, +Z, -Z
        let wall_origin = position + Vec3::new(0.0, STEP_OVER_HEIGHT, 0.0);
        let directions = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
        ];
        let wall_distances =
            directions.map(|dir| self.cast_ray(wall_origin, dir, half_width * 2.0));

        CollisionDebug {
            on_ground,
            ground_distance,
            wall_distances,
        }
    }
}
