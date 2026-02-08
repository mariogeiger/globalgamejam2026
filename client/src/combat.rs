use glam::{Mat4, Vec3};
use rand::Rng;
use std::collections::HashMap;

use crate::collision::PhysicsWorld;
use crate::config::*;
use crate::network::PeerId;
use crate::player::{MaskType, RemotePlayer, look_direction_from_angles};

/// Result of running targeting for one frame.
pub struct TargetingResult {
    /// Peer IDs of players we just killed.
    pub kills: Vec<PeerId>,
}

/// A death location with position and random tilt rotation.
pub struct DeathMarker {
    pub position: Vec3,
    /// Rotation around X axis (pitch tilt)
    pub rot_x: f32,
    /// Rotation around Z axis (roll tilt)
    pub rot_z: f32,
}

impl DeathMarker {
    /// Create a new death marker with random tilt (up to 20 degrees on X and Z axes)
    pub fn new(position: Vec3) -> Self {
        let max_tilt = 20.0_f32.to_radians();
        let mut rng = rand::rng();
        Self {
            position,
            rot_x: rng.random_range(-max_tilt..max_tilt),
            rot_z: rng.random_range(-max_tilt..max_tilt),
        }
    }

    /// Get the model matrix for this death marker
    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position)
            * Mat4::from_rotation_x(self.rot_x)
            * Mat4::from_rotation_z(self.rot_z)
    }
}

/// Advance targeting timers and kill enemies whose timer expires.
/// Mutates remote_players (targeted_time, is_alive).
/// Returns which peers were killed.
pub fn update_targeting(
    remote_players: &mut HashMap<PeerId, RemotePlayer>,
    dt: f32,
    eye_pos: Vec3,
    yaw: f32,
    pitch: f32,
    mask: MaskType,
    physics: &PhysicsWorld,
) -> TargetingResult {
    let can_kill = mask != MaskType::Coward;

    let kill_duration = match mask {
        MaskType::Hunter => HUNTER_KILL_DURATION,
        _ => TARGETING_DURATION,
    };

    let look_dir = look_direction_from_angles(yaw, pitch);
    let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();

    let mut kills = Vec::new();

    for (&peer_id, remote) in remote_players.iter_mut() {
        if !remote.is_alive {
            continue;
        }

        let enemy_head = remote.head_position();
        let to_enemy = enemy_head - eye_pos;
        let distance = to_enemy.length();

        if distance < 1.0 {
            remote.targeted_time = 0.0;
            continue;
        }

        let to_enemy_normalized = to_enemy / distance;
        let dot = look_dir.dot(to_enemy_normalized).clamp(-1.0, 1.0);
        let angle = dot.acos();

        if angle < half_angle_rad && physics.is_visible(eye_pos, enemy_head) {
            if can_kill {
                remote.targeted_time += dt;
                if remote.targeted_time >= kill_duration {
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                    kills.push(peer_id);
                    log::info!("Killed enemy {}!", peer_id);
                }
            } else {
                remote.targeted_time = 0.0;
            }
        } else {
            remote.targeted_time = 0.0;
        }
    }

    TargetingResult { kills }
}

/// Query current targeting progress for HUD crosshair.
/// Returns (max_progress 0..1, has_any_target).
pub fn get_targeting_progress(
    remote_players: &HashMap<PeerId, RemotePlayer>,
    mask: MaskType,
) -> (f32, bool) {
    let kill_duration = match mask {
        MaskType::Hunter => HUNTER_KILL_DURATION,
        _ => TARGETING_DURATION,
    };

    let mut max_progress = 0.0f32;
    let mut has_target = false;

    for remote in remote_players.values() {
        if !remote.is_alive {
            continue;
        }
        if remote.targeted_time > 0.0 {
            has_target = true;
            max_progress = max_progress.max(remote.targeted_time / kill_duration);
        }
    }

    (max_progress, has_target)
}

/// Get enemies currently aiming at our position.
/// Returns Vec of (peer_id, enemy_head_position).
pub fn get_threats(
    remote_players: &HashMap<PeerId, RemotePlayer>,
    my_head: Vec3,
    physics: &PhysicsWorld,
) -> Vec<(PeerId, Vec3)> {
    let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();
    let mut threats = Vec::new();

    for (&peer_id, remote) in remote_players {
        if !remote.is_alive {
            continue;
        }

        if remote.mask == MaskType::Coward {
            continue;
        }

        let enemy_eye = remote.eye_position();
        let enemy_look_dir = look_direction_from_angles(remote.yaw, remote.pitch);

        let to_us = my_head - enemy_eye;
        let distance = to_us.length();

        if distance < 1.0 {
            continue;
        }

        let to_us_normalized = to_us / distance;
        let dot = enemy_look_dir.dot(to_us_normalized).clamp(-1.0, 1.0);
        let angle = dot.acos();

        if angle < half_angle_rad && physics.is_visible(enemy_eye, my_head) {
            threats.push((peer_id, remote.head_position()));
        }
    }

    threats
}
