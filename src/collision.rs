use glam::Vec3;
use rapier3d::prelude::*;

pub struct PhysicsWorld {
    pub rigid_body_set: RigidBodySet,
    pub collider_set: ColliderSet,
    pub query_pipeline: QueryPipeline,
    
    #[allow(dead_code)]
    pub player_body_handle: RigidBodyHandle,
    pub player_collider_handle: ColliderHandle,
}

impl PhysicsWorld {
    pub fn new(
        collision_vertices: &[Vec3],
        collision_indices: &[[u32; 3]],
        player_spawn: Vec3,
    ) -> Self {
        let mut rigid_body_set = RigidBodySet::new();
        let mut collider_set = ColliderSet::new();
        
        // Create static map collider from triangles
        if !collision_vertices.is_empty() && !collision_indices.is_empty() {
            let vertices: Vec<Point<f32>> = collision_vertices
                .iter()
                .map(|v| Point::new(v.x, v.y, v.z))
                .collect();
            
            let indices: Vec<[u32; 3]> = collision_indices.to_vec();
            
            let trimesh = TriMesh::new(vertices, indices);
            let map_collider = ColliderBuilder::new(SharedShape::new(trimesh))
                .friction(0.8)
                .build();
            
            collider_set.insert(map_collider);
        }
        
        // Create player rigid body (kinematic character controller)
        let player_body = RigidBodyBuilder::kinematic_position_based()
            .translation(vector![player_spawn.x, player_spawn.y, player_spawn.z])
            .build();
        
        let player_body_handle = rigid_body_set.insert(player_body);
        
        // Player capsule collider
        let player_collider = ColliderBuilder::capsule_y(36.0, 16.0)
            .friction(0.0)
            .build();
        
        let player_collider_handle = collider_set.insert_with_parent(
            player_collider,
            player_body_handle,
            &mut rigid_body_set,
        );
        
        Self {
            rigid_body_set,
            collider_set,
            query_pipeline: QueryPipeline::new(),
            player_body_handle,
            player_collider_handle,
        }
    }
    
    /// Move player with simple collision detection using ray casting
    pub fn move_player(&mut self, desired_position: Vec3) -> (Vec3, bool) {
        self.query_pipeline.update(&self.collider_set);
        
        let filter = QueryFilter::default()
            .exclude_collider(self.player_collider_handle);
        
        let mut final_pos = desired_position;
        let mut on_ground = false;
        
        // Ground check - cast ray downward from player feet
        let ray = Ray::new(
            Point::new(desired_position.x, desired_position.y + 40.0, desired_position.z),
            vector![0.0, -1.0, 0.0],
        );
        
        if let Some((_, toi)) = self.query_pipeline.cast_ray(
            &self.rigid_body_set,
            &self.collider_set,
            &ray,
            100.0,
            true,
            filter,
        ) {
            // If we hit ground within reasonable distance
            if toi < 50.0 {
                on_ground = true;
                let ground_y = desired_position.y + 40.0 - toi;
                // Keep player above ground
                if final_pos.y < ground_y {
                    final_pos.y = ground_y;
                }
            }
        }
        
        // Simple horizontal collision checks
        for &(dx, dz) in &[(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let ray = Ray::new(
                Point::new(desired_position.x, desired_position.y + 36.0, desired_position.z),
                vector![dx, 0.0, dz],
            );
            
            if let Some((_, toi)) = self.query_pipeline.cast_ray(
                &self.rigid_body_set,
                &self.collider_set,
                &ray,
                20.0,
                true,
                filter,
            ) {
                if toi < 20.0 {
                    // Push back from wall
                    final_pos.x -= dx * (20.0 - toi);
                    final_pos.z -= dz * (20.0 - toi);
                }
            }
        }
        
        // Update player body position
        let player_body = &mut self.rigid_body_set[self.player_body_handle];
        player_body.set_translation(vector![final_pos.x, final_pos.y, final_pos.z], true);
        
        (final_pos, on_ground)
    }
}
