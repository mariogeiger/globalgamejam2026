use glam::Vec3;
use rand::Rng;
use std::collections::HashMap;
use web_time::Instant;

use crate::collision::PhysicsWorld;
use crate::config::*;
use crate::input::InputState;
use crate::mesh::Mesh;
use crate::network::{GamePhase, NetworkEvent, PeerId};
use crate::player::{MaskType, Player, RemotePlayer};
use winit::keyboard::KeyCode;

pub struct GameState {
    pub player: Player,
    pub remote_players: HashMap<PeerId, RemotePlayer>,
    pub physics: PhysicsWorld,
    pub map_bounds: (Vec3, Vec3),
    pub is_dead: bool,
    pub phase: GamePhase,
    pub phase_timer: f32,
    pub winner_id: Option<PeerId>,
    last_update: Instant,
    pending_kills: Vec<PeerId>,
    local_peer_id: Option<PeerId>,
    just_died: bool,
}

impl GameState {
    pub fn new(mesh: &Mesh, debug_mannequins: bool) -> Self {
        let spawn_idx = rand::rng().random_range(0..SPAWN_POINTS.len());
        let initial_spawn = Self::get_spawn_point(spawn_idx);

        let player = Player::new(initial_spawn);

        // Extract collision data from mesh vertices
        let (collision_vertices, collision_indices, bounds) = Self::extract_collision_data(mesh);
        let physics = PhysicsWorld::new(&collision_vertices, &collision_indices)
            .expect("Failed to create physics world");

        let mut remote_players = HashMap::new();
        if debug_mannequins && SPAWN_POINTS.len() >= 2 {
            // Create mannequins at different spawn points for testing
            let mut mannequin1 = RemotePlayer::new();
            mannequin1.position = Self::get_spawn_point((spawn_idx + 1) % SPAWN_POINTS.len());
            mannequin1.mask = MaskType::Hunter; // Test Hunter cone rendering
            remote_players.insert(u64::MAX, mannequin1);

            let mut mannequin2 = RemotePlayer::new();
            mannequin2.position = Self::get_spawn_point((spawn_idx + 2) % SPAWN_POINTS.len());
            remote_players.insert(u64::MAX - 1, mannequin2);

            log::info!(
                "Created debug mannequins at spawn points, player at {:?}",
                initial_spawn
            );
        }

        Self {
            player,
            remote_players,
            physics,
            map_bounds: bounds,
            is_dead: false,
            phase: GamePhase::WaitingForPlayers,
            phase_timer: 0.0,
            winner_id: None,
            last_update: Instant::now(),
            pending_kills: Vec::new(),
            local_peer_id: None,
            just_died: false,
        }
    }

    fn extract_collision_data(mesh: &Mesh) -> (Vec<Vec3>, Vec<[u32; 3]>, (Vec3, Vec3)) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for submesh in &mesh.submeshes {
            let base_idx = vertices.len() as u32;

            // Transform already applied at load time
            for v in &submesh.vertices {
                vertices.push(Vec3::from_array(v.position));
            }

            // Convert to triangle indices
            for chunk in submesh.indices.chunks(3) {
                if chunk.len() == 3 {
                    indices.push([
                        base_idx + chunk[0],
                        base_idx + chunk[1],
                        base_idx + chunk[2],
                    ]);
                }
            }
        }

        // Compute bounds
        let (bounds_min, bounds_max) = vertices.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(min, max), v| (min.min(*v), max.max(*v)),
        );

        (vertices, indices, (bounds_min, bounds_max))
    }

    fn get_spawn_point(idx: usize) -> Vec3 {
        let p = SPAWN_POINTS[idx];
        Vec3::new(p[0], p[1], p[2])
    }

    fn random_spawn_point() -> Vec3 {
        let idx = rand::rng().random_range(0..SPAWN_POINTS.len());
        Self::get_spawn_point(idx)
    }

    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = (now - self.last_update).as_secs_f32().min(0.1);
        self.last_update = now;

        // Update local phase timer for display
        if self.phase_timer > 0.0 {
            self.phase_timer = (self.phase_timer - dt).max(0.0);
        }

        self.update_hud_display();

        // Handle mask switching input (always available, even in spectator)
        self.update_mask_input(input);

        // Spectator mode: when dead, during victory, or waiting to join mid-game
        let is_spectator =
            self.is_dead || self.phase == GamePhase::Victory || self.phase == GamePhase::Spectating;
        if is_spectator {
            self.player.spectator_update(dt, input);
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

        // Allow targeting during Playing phase and WaitingForPlayers (for testing mannequins)
        if self.phase == GamePhase::Playing || self.phase == GamePhase::WaitingForPlayers {
            self.update_targeting(dt);
        }
    }

    fn update_mask_input(&mut self, input: &mut InputState) {
        if input.just_pressed(KeyCode::Digit1) {
            self.player.set_mask(MaskType::Ghost);
        }
        if input.just_pressed(KeyCode::Digit2) {
            self.player.set_mask(MaskType::Coward);
        }
        if input.just_pressed(KeyCode::Digit3) {
            self.player.set_mask(MaskType::Hunter);
        }
        if input.just_pressed(KeyCode::KeyE) {
            self.player.swap_to_last_mask();
        }

        let scroll = input.consume_scroll();
        if scroll > 0.0 {
            self.player.cycle_mask_next();
        } else if scroll < 0.0 {
            self.player.cycle_mask_prev();
        }
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
        self.player.respawn(Self::random_spawn_point());
    }

    fn update_targeting(&mut self, dt: f32) {
        if self.is_dead {
            return;
        }

        // Coward mask cannot kill
        let can_kill = self.player.mask != MaskType::Coward;

        // Hunter mask kills faster
        let kill_duration = match self.player.mask {
            MaskType::Hunter => HUNTER_KILL_DURATION,
            _ => TARGETING_DURATION,
        };

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
                // Only accumulate targeting time if we can kill (Coward can't charge up)
                if can_kill {
                    remote.targeted_time += dt;
                    if remote.targeted_time >= kill_duration {
                        remote.is_alive = false;
                        remote.targeted_time = 0.0;
                        new_kills.push(peer_id);
                        log::info!("Killed enemy {}!", peer_id);
                    }
                } else {
                    // Coward mask: reset any accumulated time to prevent exploit
                    remote.targeted_time = 0.0;
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

    /// Check if local player just died (needs to notify server)
    pub fn take_death_notification(&mut self) -> bool {
        if self.just_died {
            self.just_died = false;
            true
        } else {
            false
        }
    }

    pub fn get_targeting_info(&self) -> (f32, bool) {
        let can_target =
            self.phase == GamePhase::Playing || self.phase == GamePhase::WaitingForPlayers;
        if !can_target || self.is_dead {
            return (0.0, false);
        }

        // Use same kill duration as update_targeting
        let kill_duration = match self.player.mask {
            MaskType::Hunter => HUNTER_KILL_DURATION,
            _ => TARGETING_DURATION,
        };

        let mut max_progress = 0.0f32;
        let mut has_target = false;

        for remote in self.remote_players.values() {
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

    pub fn handle_network_event(&mut self, event: NetworkEvent, local_peer_id: Option<PeerId>) {
        match event {
            NetworkEvent::Connected {
                id,
                phase,
                phase_time_remaining,
            } => {
                log::info!(
                    "Connected with ID: {}, phase: {:?}, time: {}",
                    id,
                    phase,
                    phase_time_remaining
                );
                self.local_peer_id = Some(id);
                // If joining mid-game, become spectator instead of playing
                let actual_phase = if phase == GamePhase::Playing {
                    log::info!("Joined mid-game, entering spectator mode");
                    GamePhase::Spectating
                } else {
                    phase
                };
                self.set_phase(actual_phase, phase_time_remaining);
                self.update_player_count_display();
            }
            NetworkEvent::PeerJoined { id } => {
                log::info!("Peer {} joined", id);
                // Remove mannequins when a real player connects
                self.remote_players.remove(&u64::MAX);
                self.remote_players.remove(&(u64::MAX - 1));
                let remote = RemotePlayer::new();
                self.remote_players.insert(id, remote);
                self.update_player_count_display();
            }
            NetworkEvent::PeerLeft { id } => {
                log::info!("Peer {} left", id);
                self.remote_players.remove(&id);
                self.update_player_count_display();
            }
            NetworkEvent::GamePhaseChanged {
                phase,
                time_remaining,
            } => {
                log::info!(
                    "Game phase changed to {:?}, time: {}",
                    phase,
                    time_remaining
                );
                // Stay spectating until new round (GracePeriod) starts
                let actual_phase =
                    if self.phase == GamePhase::Spectating && phase == GamePhase::Playing {
                        GamePhase::Spectating
                    } else {
                        phase
                    };
                self.set_phase(actual_phase, time_remaining);
            }
            NetworkEvent::PlayerState {
                id,
                position,
                yaw,
                mask,
            } => {
                if let Some(remote) = self.remote_players.get_mut(&id) {
                    remote.position = position;
                    remote.yaw = yaw;
                    remote.mask = MaskType::from_u8(mask);
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
                    self.just_died = true;
                    show_death_overlay(killer_id);
                } else if let Some(remote) = self.remote_players.get_mut(&victim_id) {
                    // Another player was killed
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                }
            }
        }
    }

    fn set_phase(&mut self, phase: GamePhase, time_remaining: f32) {
        let old_phase = self.phase;
        self.phase = phase;
        self.phase_timer = time_remaining;

        match phase {
            GamePhase::WaitingForPlayers => {
                hide_countdown_overlay();
                hide_victory_overlay();
                hide_spectating_overlay();
                show_waiting_overlay();
            }
            GamePhase::GracePeriod => {
                // New round starting - reset state
                if old_phase != GamePhase::GracePeriod {
                    self.is_dead = false;
                    self.winner_id = None;
                    self.pending_kills.clear();
                    self.respawn_player();

                    // Reset all remote players
                    for remote in self.remote_players.values_mut() {
                        remote.is_alive = true;
                        remote.targeted_time = 0.0;
                    }

                    hide_death_overlay();
                    hide_victory_overlay();
                    hide_spectating_overlay();
                }
                hide_waiting_overlay();
                show_countdown_overlay();
                update_countdown_display(time_remaining.ceil() as u32);
            }
            GamePhase::Playing => {
                hide_countdown_overlay();
                hide_waiting_overlay();
                hide_spectating_overlay();
            }
            GamePhase::Victory => {
                // Determine winner
                if !self.is_dead {
                    self.winner_id = self.local_peer_id;
                } else {
                    self.winner_id = self
                        .remote_players
                        .iter()
                        .find(|(_, p)| p.is_alive)
                        .map(|(&id, _)| id);
                }

                let is_local_winner =
                    self.winner_id == self.local_peer_id && self.local_peer_id.is_some();
                show_victory_overlay(self.winner_id, is_local_winner);
            }
            GamePhase::Spectating => {
                // Joined mid-game, show spectating message
                hide_countdown_overlay();
                hide_victory_overlay();
                hide_death_overlay();
                show_spectating_overlay();
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

        // Update countdown timer during grace period
        if self.phase == GamePhase::GracePeriod {
            update_countdown_display(self.phase_timer.ceil() as u32);
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
            let _ = overlay.set_attribute("style", "display: block;");
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
        let _ = overlay.set_attribute("style", "display: block;");
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

fn show_waiting_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("waiting-overlay")
    {
        let _ = overlay.set_attribute("style", "display: block;");
    }
}

fn hide_waiting_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("waiting-overlay")
    {
        let _ = overlay.set_attribute("style", "display: none;");
    }
}

fn show_victory_overlay(winner_id: Option<PeerId>, is_local_winner: bool) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(overlay) = doc.get_element_by_id("victory-overlay") {
            let _ = overlay.set_attribute("style", "display: block;");
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

fn show_spectating_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("spectating-overlay")
    {
        let _ = overlay.set_attribute("style", "display: block;");
    }
}

fn hide_spectating_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("spectating-overlay")
    {
        let _ = overlay.set_attribute("style", "display: none;");
    }
}
