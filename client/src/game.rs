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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GamePhase {
    Countdown,
    Playing,
    Victory,
}

pub struct GameState {
    pub player: Player,
    pub remote_players: HashMap<PeerId, RemotePlayer>,
    pub physics: PhysicsWorld,
    pub spawn_points: Vec<Vec3>,
    pub map_bounds: (Vec3, Vec3),
    pub is_dead: bool,
    pub phase: GamePhase,
    pub phase_timer: f32,
    pub winner_id: Option<PeerId>,
    last_update: Instant,
    pending_kills: Vec<PeerId>,
    local_peer_id: Option<PeerId>,
}

impl GameState {
    pub fn new(map: &LoadedMap, _debug_mannequins: bool) -> Self {
        let spawn_idx = rand::rng().random_range(0..map.spawn_points.len());
        let initial_spawn = map.spawn_points[spawn_idx];

        let player = Player::new(initial_spawn);
        let physics = PhysicsWorld::new(&map.collision_vertices, &map.collision_indices)
            .expect("Failed to create physics world");

        Self {
            player,
            remote_players: HashMap::new(),
            physics,
            spawn_points: map.spawn_points.clone(),
            map_bounds: (map.bounds_min, map.bounds_max),
            is_dead: false,
            phase: GamePhase::Countdown,
            phase_timer: COUNTDOWN_DURATION,
            winner_id: None,
            last_update: Instant::now(),
            pending_kills: Vec::new(),
            local_peer_id: None,
        }
    }

    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = (now - self.last_update).as_secs_f32().min(0.1);
        self.last_update = now;

        self.update_phase(dt);
        self.update_hud_display();

        // Don't process movement while dead or in victory phase
        if self.is_dead || self.phase == GamePhase::Victory {
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

        // Only update targeting during Playing phase
        if self.phase == GamePhase::Playing {
            self.update_targeting(dt);
        }
    }

    fn update_phase(&mut self, dt: f32) {
        match self.phase {
            GamePhase::Countdown => {
                self.phase_timer -= dt;
                if self.phase_timer <= 0.0 {
                    self.phase = GamePhase::Playing;
                    self.phase_timer = 0.0;
                    log::info!("Game started! Fight!");
                    hide_countdown_overlay();
                }
                update_countdown_display(self.phase_timer.ceil() as u32);
            }
            GamePhase::Playing => {
                self.check_victory();
            }
            GamePhase::Victory => {
                self.phase_timer -= dt;
                if self.phase_timer <= 0.0 {
                    self.restart_game();
                }
            }
        }
    }

    fn check_victory(&mut self) {
        // Count alive players (including self)
        let alive_count = if self.is_dead { 0 } else { 1 }
            + self.remote_players.values().filter(|p| p.is_alive).count();

        // Need at least 2 players to have a winner
        let total_players = 1 + self.remote_players.len();
        if total_players < 2 {
            return;
        }

        if alive_count <= 1 {
            // Find the winner
            if !self.is_dead {
                self.winner_id = self.local_peer_id;
            } else {
                self.winner_id = self
                    .remote_players
                    .iter()
                    .find(|(_, p)| p.is_alive)
                    .map(|(&id, _)| id);
            }

            self.phase = GamePhase::Victory;
            self.phase_timer = VICTORY_DURATION;

            let is_local_winner =
                self.winner_id == self.local_peer_id && self.local_peer_id.is_some();
            show_victory_overlay(self.winner_id, is_local_winner);
            log::info!("Victory! Winner: {:?}", self.winner_id);
        }
    }

    fn restart_game(&mut self) {
        log::info!("Restarting game...");

        // Reset local player
        self.is_dead = false;
        self.respawn_player();

        // Reset all remote players
        for remote in self.remote_players.values_mut() {
            remote.is_alive = true;
            remote.targeted_time = 0.0;
        }

        // Reset phase
        self.phase = GamePhase::Countdown;
        self.phase_timer = COUNTDOWN_DURATION;
        self.winner_id = None;
        self.pending_kills.clear();

        hide_victory_overlay();
        hide_death_overlay();
        show_countdown_overlay();
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
        if !self.spawn_points.is_empty() {
            let idx = rand::rng().random_range(0..self.spawn_points.len());
            self.player.respawn(self.spawn_points[idx]);
        }
    }

    fn update_targeting(&mut self, dt: f32) {
        if self.is_dead {
            return;
        }

        let eye_pos = self.player.eye_position();
        let look_dir = self.player.look_direction();
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();

        let mut new_kills = Vec::new();

        for (&peer_id, remote) in self.remote_players.iter_mut() {
            if !remote.is_alive {
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
        if self.phase != GamePhase::Playing || self.is_dead {
            return (0.0, false);
        }

        let mut max_progress = 0.0f32;
        let mut has_target = false;

        for remote in self.remote_players.values() {
            if !remote.is_alive {
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
            NetworkEvent::Connected { id } => {
                log::info!("Connected with ID: {}", id);
                self.local_peer_id = Some(id);
                self.update_player_count_display();
            }
            NetworkEvent::PeerJoined { id } => {
                log::info!("Peer {} joined", id);
                let remote = RemotePlayer::new();
                self.remote_players.insert(id, remote);
                self.update_player_count_display();
            }
            NetworkEvent::PeerLeft { id } => {
                log::info!("Peer {} left", id);
                self.remote_players.remove(&id);
                self.update_player_count_display();
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
                    show_death_overlay(killer_id);
                } else if let Some(remote) = self.remote_players.get_mut(&victim_id) {
                    // Another player was killed
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                }
            }
        }
    }

    fn update_hud_display(&self) {
        let pos = self.player.position;
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Some(e) = doc.get_element_by_id("local-pos")
        {
            e.set_text_content(Some(&format!("[{:.1}, {:.1}, {:.1}]", pos.x, pos.y, pos.z)));
        }
    }

    fn update_player_count_display(&self) {
        let total = 1 + self.remote_players.len();
        let alive = if self.is_dead { 0 } else { 1 }
            + self.remote_players.values().filter(|p| p.is_alive).count();

        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Some(e) = doc.get_element_by_id("player-count")
        {
            e.set_text_content(Some(&format!("{} / {}", alive, total)));
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

fn show_countdown_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("countdown-overlay")
    {
        let _ = overlay.set_attribute("style", "display: flex;");
    }
}

fn hide_countdown_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("countdown-overlay")
    {
        let _ = overlay.set_attribute("style", "display: none;");
    }
}

fn update_countdown_display(seconds: u32) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id("countdown-timer")
    {
        e.set_text_content(Some(&seconds.to_string()));
    }
}

fn show_victory_overlay(winner_id: Option<PeerId>, is_local_winner: bool) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(overlay) = doc.get_element_by_id("victory-overlay") {
            let _ = overlay.set_attribute("style", "display: flex;");
        }
        if let Some(title) = doc.get_element_by_id("victory-title") {
            if is_local_winner {
                title.set_text_content(Some("VICTORY!"));
            } else {
                title.set_text_content(Some("DEFEATED"));
            }
        }
        if let Some(winner_elem) = doc.get_element_by_id("winner-id") {
            if let Some(id) = winner_id {
                winner_elem.set_text_content(Some(&format!("Player {} wins!", id)));
            } else {
                winner_elem.set_text_content(Some("Draw!"));
            }
        }
    }
}

fn hide_victory_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("victory-overlay")
    {
        let _ = overlay.set_attribute("style", "display: none;");
    }
}
