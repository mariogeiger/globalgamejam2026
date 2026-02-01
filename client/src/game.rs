use glam::Vec3;
use rand::Rng;
use std::collections::HashMap;
use web_time::Instant;

use crate::collision::PhysicsWorld;
use crate::config::*;
use crate::input::InputState;
use crate::map::LoadedMap;
use crate::network::{NetworkEvent, PeerId};
use crate::player::{Player, RemotePlayer};
use crate::team::Team;

pub struct GameState {
    pub player: Player,
    pub remote_players: HashMap<PeerId, RemotePlayer>,
    pub physics: PhysicsWorld,
    pub spawn_points: Vec<Vec3>,
    pub map_bounds: (Vec3, Vec3),
    pub local_team: Option<Team>,
    pub is_dead: bool,
    pub killer_id: Option<PeerId>,
    last_update: Instant,
    pending_kills: Vec<PeerId>,
}

impl GameState {
    pub fn new(map: &LoadedMap, debug_mannequins: bool) -> Self {
        let spawn_idx = rand::rng().random_range(0..map.spawn_points.len());
        let initial_spawn = map.spawn_points[spawn_idx];

        let player = Player::new(initial_spawn);
        let physics = PhysicsWorld::new(&map.collision_vertices, &map.collision_indices)
            .expect("Failed to create physics world");

        let mut remote_players = HashMap::new();
        if debug_mannequins {
            let mannequin_a = RemotePlayer::new(Team::A);
            let mannequin_b = RemotePlayer::new(Team::B);
            log::info!(
                "Creating mannequins - A at {:?}, B at {:?}, player at {:?}",
                mannequin_a.position,
                mannequin_b.position,
                initial_spawn
            );
            remote_players.insert(u64::MAX, mannequin_a);
            remote_players.insert(u64::MAX - 1, mannequin_b);
        }

        Self {
            player,
            remote_players,
            physics,
            spawn_points: map.spawn_points.clone(),
            map_bounds: (map.bounds_min, map.bounds_max),
            local_team: None,
            is_dead: false,
            killer_id: None,
            last_update: Instant::now(),
            pending_kills: Vec::new(),
        }
    }

    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = (now - self.last_update).as_secs_f32().min(0.1);
        self.last_update = now;

        // Handle death/respawn timer
        self.update_death(dt);

        // Don't process movement while dead
        if self.is_dead {
            self.update_coordinates_display();
            return;
        }

        let prev_pos = self.player.position;
        self.player.update(dt, input);

        let desired_pos = self
            .physics
            .clamp_desired_to_path(prev_pos, self.player.position);
        self.player.position = desired_pos;

        let (new_pos, on_ground) = self
            .physics
            .move_player(self.player.position, self.player.velocity);
        self.player.position = new_pos;
        self.player.set_on_ground(on_ground, None);

        self.check_respawn();
        self.update_targeting(dt);
        self.update_coordinates_display();
    }

    fn check_respawn(&mut self) {
        let (bounds_min, bounds_max) = self.map_bounds;
        let pos = self.player.position;
        let outside = pos.x < bounds_min.x - RESPAWN_MARGIN
            || pos.x > bounds_max.x + RESPAWN_MARGIN
            || pos.y < bounds_min.y - RESPAWN_MARGIN
            || pos.y > bounds_max.y + RESPAWN_MARGIN
            || pos.z < bounds_min.z - RESPAWN_MARGIN
            || pos.z > bounds_max.z + RESPAWN_MARGIN;

        if outside {
            log::info!("Player fell out of map, respawning");
            self.respawn_player();
        }
    }

    pub fn respawn_player(&mut self) {
        if let Some(team) = self.local_team {
            let spawns = team.spawn_points();
            if !spawns.is_empty() {
                let idx = rand::rng().random_range(0..spawns.len());
                let spawn = spawns[idx];
                self.player.respawn(Vec3::new(spawn[0], spawn[1], spawn[2]));
            }
        } else if !self.spawn_points.is_empty() {
            let idx = rand::rng().random_range(0..self.spawn_points.len());
            self.player.respawn(self.spawn_points[idx]);
        }
    }

    fn update_targeting(&mut self, dt: f32) {
        let eye_pos = self.player.eye_position();
        let look_dir = self.player.look_direction();
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();

        let mut new_kills = Vec::new();

        for (&peer_id, remote) in self.remote_players.iter_mut() {
            if !remote.is_alive {
                remote.dead_time += dt;
                if remote.dead_time >= RESPAWN_DELAY {
                    remote.respawn();
                    log::info!("Enemy {} respawned!", peer_id);
                }
                continue;
            }

            if let Some(local_team) = self.local_team
                && remote.team == local_team
            {
                remote.targeted_time = 0.0;
                continue;
            }

            let enemy_center = remote.center_mass();
            let to_enemy = enemy_center - eye_pos;
            let distance = to_enemy.length();

            if distance < 1.0 {
                remote.targeted_time = 0.0;
                continue;
            }

            let to_enemy_normalized = to_enemy / distance;
            let dot = look_dir.dot(to_enemy_normalized).clamp(-1.0, 1.0);
            let angle = dot.acos();

            if angle < half_angle_rad && self.physics.is_visible(eye_pos, enemy_center) {
                remote.targeted_time += dt;
                if remote.targeted_time >= TARGETING_DURATION {
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                    new_kills.push(peer_id);
                    log::info!("Killed enemy {}!", peer_id);
                }
            } else {
                remote.targeted_time = 0.0;
            }
        }

        self.pending_kills.extend(new_kills);
    }

    /// Take pending kills to be sent over network
    pub fn take_pending_kills(&mut self) -> Vec<PeerId> {
        std::mem::take(&mut self.pending_kills)
    }

    pub fn get_targeting_info(&self) -> (f32, bool) {
        let mut max_progress = 0.0f32;
        let mut has_target = false;

        for remote in self.remote_players.values() {
            if !remote.is_alive {
                continue;
            }
            if let Some(local_team) = self.local_team
                && remote.team == local_team
            {
                continue;
            }
            if remote.targeted_time > 0.0 {
                has_target = true;
                max_progress = max_progress.max(remote.targeted_time / TARGETING_DURATION);
            }
        }

        (max_progress, has_target)
    }

    pub fn handle_network_event(&mut self, event: NetworkEvent, local_peer_id: Option<PeerId>) {
        match event {
            NetworkEvent::TeamAssigned(team) => {
                log::info!("Assigned to team: {:?}", team);
                self.local_team = Some(team);
                let spawns = team.spawn_points();
                if !spawns.is_empty() {
                    let idx = rand::rng().random_range(0..spawns.len());
                    let spawn = spawns[idx];
                    self.player.respawn(Vec3::new(spawn[0], spawn[1], spawn[2]));
                }
                self.update_team_counts_display();
            }
            NetworkEvent::PeerJoined { id, team } => {
                log::info!("Peer {} joined on team {:?}", id, team);
                let remote = RemotePlayer::new(team);
                self.remote_players.insert(id, remote);
                self.update_team_counts_display();
            }
            NetworkEvent::PeerLeft { id } => {
                log::info!("Peer {} left", id);
                self.remote_players.remove(&id);
                self.update_team_counts_display();
            }
            NetworkEvent::PlayerState { id, position, yaw } => {
                if let Some(remote) = self.remote_players.get_mut(&id) {
                    remote.position = position;
                    remote.yaw = yaw;
                }
            }
            NetworkEvent::PlayerKilled {
                killer_id,
                victim_id,
            } => {
                log::info!("Player {} was killed by {}", victim_id, killer_id);

                // Check if we are the victim
                if let Some(local_id) = local_peer_id
                    && victim_id == local_id
                {
                    self.is_dead = true;
                    self.killer_id = Some(killer_id);
                    show_death_overlay(killer_id);
                } else if let Some(remote) = self.remote_players.get_mut(&victim_id) {
                    // Another player was killed
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                }
            }
        }
    }

    fn update_coordinates_display(&self) {
        let pos = self.player.position;
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Some(e) = doc.get_element_by_id("local-pos")
        {
            e.set_text_content(Some(&format!("[{:.1}, {:.1}, {:.1}]", pos.x, pos.y, pos.z)));
        }
    }

    fn update_team_counts_display(&self) {
        let mut team_a = 0;
        let mut team_b = 0;

        if let Some(team) = self.local_team {
            match team {
                Team::A => team_a += 1,
                Team::B => team_b += 1,
            }
        }

        for remote in self.remote_players.values() {
            match remote.team {
                Team::A => team_a += 1,
                Team::B => team_b += 1,
            }
        }

        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(e) = doc.get_element_by_id("team-a-count") {
                e.set_text_content(Some(&team_a.to_string()));
            }
            if let Some(e) = doc.get_element_by_id("team-b-count") {
                e.set_text_content(Some(&team_b.to_string()));
            }
        }
    }

    /// Handle local player death timer and respawn
    pub fn update_death(&mut self, dt: f32) {
        if !self.is_dead {
            return;
        }

        // Use dead_time tracking - we'll reuse a simple approach
        // The death overlay is shown, wait for RESPAWN_DELAY then respawn
        static mut DEATH_TIMER: f32 = 0.0;

        unsafe {
            DEATH_TIMER += dt;
            if DEATH_TIMER >= RESPAWN_DELAY {
                DEATH_TIMER = 0.0;
                self.is_dead = false;
                self.killer_id = None;
                hide_death_overlay();
                self.respawn_player();
                log::info!("Respawned after death!");
            }
        }
    }
}

fn show_death_overlay(killer_id: PeerId) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(overlay) = doc.get_element_by_id("death-overlay") {
            let _ = overlay.set_attribute("style", "display: flex;");
        }
        if let Some(killer_elem) = doc.get_element_by_id("killer-id") {
            killer_elem.set_text_content(Some(&killer_id.to_string()));
        }
    }
}

fn hide_death_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("death-overlay")
    {
        let _ = overlay.set_attribute("style", "display: none;");
    }
}
