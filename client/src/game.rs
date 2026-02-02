use base64::Engine;
use glam::{Mat4, Vec3};
use rand::Rng;
use std::collections::HashMap;
use web_time::Instant;

use crate::assets::{COWARD_IMAGE, GHOST_IMAGE, HUNTER_IMAGE};
use crate::collision::PhysicsWorld;
use crate::config::*;
use crate::debug::{DebugOverlay, PhysicsDebugInfo};
use crate::input::InputState;
use crate::mesh::Mesh;
use crate::network::{
    GamePhase, NetworkClient, NetworkEvent, PeerId, fetch_peer_stats, update_peer_stats_display,
};
use crate::player::{MaskType, Player, RemotePlayer, look_direction_from_angles};
use winit::keyboard::KeyCode;

/// State captured at moment of death for grace period targeting
struct DeathState {
    time: Instant,
    position: Vec3,
    yaw: f32,
    pitch: f32,
    mask: MaskType,
}

/// A death location with position and random tilt rotation
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
    pending_death_sounds: u32,
    local_peer_id: Option<PeerId>,
    just_died: bool,
    pub time: f32,
    /// Time when the mask was last changed (for mask change animation)
    pub mask_change_time: Option<f32>,
    /// Locations where players have died (persists across rounds)
    pub death_locations: Vec<DeathMarker>,
    /// Death state for grace period targeting (frozen position/orientation)
    death_state: Option<DeathState>,
    /// Local player's name
    pub local_name: Option<String>,
    /// Local player's total kills
    pub local_kills: u32,
    /// Last time peer stats were updated
    last_stats_update: Instant,
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
            pending_death_sounds: 0,
            local_peer_id: None,
            just_died: false,
            time: 0.0,
            mask_change_time: None,
            death_locations: Vec::new(),
            death_state: None,
            local_name: None,
            local_kills: 0,
            last_stats_update: Instant::now(),
        }
    }

    pub fn set_local_name(&mut self, name: String) {
        self.local_name = Some(name.clone());
        update_player_name_display(&name);
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
        self.time += dt;

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

            // Continue targeting from frozen death position during grace period
            if self.is_dead
                && let Some(ref death) = self.death_state
                && death.time.elapsed().as_secs_f32() <= DEATH_GRACE_PERIOD
            {
                self.update_targeting_from_state(
                    dt,
                    death.position,
                    death.yaw,
                    death.pitch,
                    death.mask,
                );
            }
            return;
        }

        // Handle coward dash trigger before normal update
        if self.player.can_dash() && input.is_pressed(KeyCode::Space) {
            let look_dir = self.player.look_direction();
            let origin = self.player.eye_position();
            let target = self.physics.dash_target(origin, look_dir, DASH_DISTANCE);
            // Adjust target to be at player feet level (origin was at eye)
            let target = target - Vec3::new(0.0, EYE_HEIGHT, 0.0);
            self.player.start_dash(target);
        }

        // Physics
        let prev_pos = self.player.position;
        self.player.update(dt, input);

        // During dash, skip path clamping (we pre-calculated safe target)
        // but always run ground/wall detection to prevent clipping through floor
        if !self.player.is_dashing() {
            let desired_pos = self
                .physics
                .clamp_desired_to_path(prev_pos, self.player.position);
            self.player.position = desired_pos;
        }

        let (new_pos, on_ground) = self
            .physics
            .move_player(self.player.position, self.player.velocity);
        self.player.position = new_pos;
        self.player.set_on_ground(on_ground, None);

        self.check_respawn();

        // Targeting - allow during Playing phase and WaitingForPlayers (for testing mannequins)
        if self.phase == GamePhase::Playing || self.phase == GamePhase::WaitingForPlayers {
            self.update_targeting(dt);
        }
    }

    fn update_mask_input(&mut self, input: &mut InputState) {
        let old_mask = self.player.mask;

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

        // Trigger mask change animation if mask changed
        if self.player.mask != old_mask {
            self.mask_change_time = Some(self.time);
            update_mask_selector(self.player.mask);
        }
    }

    fn check_respawn(&mut self) {
        let (bounds_min, bounds_max) = self.map_bounds;
        let pos = self.player.position;
        let vel = self.player.velocity;

        // Check if outside map bounds
        let outside = pos.x < bounds_min.x - RESPAWN_MARGIN
            || pos.x > bounds_max.x + RESPAWN_MARGIN
            || pos.y < bounds_min.y - RESPAWN_MARGIN
            || pos.y > bounds_max.y + RESPAWN_MARGIN
            || pos.z < bounds_min.z - RESPAWN_MARGIN
            || pos.z > bounds_max.z + RESPAWN_MARGIN;

        // Check for extreme falling velocity (stuck under map)
        let extreme_velocity = vel.y < -MAX_FALL_VELOCITY;

        if outside {
            log::info!("Player fell out of map, respawning");
            self.respawn_player();
        } else if extreme_velocity {
            log::info!(
                "Player stuck with extreme velocity ({:.0}), respawning",
                vel.y
            );
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

        let eye_pos = self.player.eye_position();
        let yaw = self.player.yaw;
        let pitch = self.player.pitch;
        let mask = self.player.mask;

        self.update_targeting_from_state(dt, eye_pos, yaw, pitch, mask);
    }

    /// Targeting logic that can use either live player state or frozen death state
    fn update_targeting_from_state(
        &mut self,
        dt: f32,
        eye_pos: Vec3,
        yaw: f32,
        pitch: f32,
        mask: MaskType,
    ) {
        // Coward mask cannot kill
        let can_kill = mask != MaskType::Coward;

        // Hunter mask kills faster
        let kill_duration = match mask {
            MaskType::Hunter => HUNTER_KILL_DURATION,
            _ => TARGETING_DURATION,
        };

        let look_dir = look_direction_from_angles(yaw, pitch);
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();

        let mut new_kills = Vec::new();

        for (&peer_id, remote) in self.remote_players.iter_mut() {
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

            if angle < half_angle_rad && self.physics.is_visible(eye_pos, enemy_head) {
                // Only accumulate targeting time if we can kill (Coward can't charge up)
                if can_kill {
                    remote.targeted_time += dt;
                    if remote.targeted_time >= kill_duration {
                        // Record death location for tombstone
                        self.death_locations.push(DeathMarker::new(remote.position));
                        remote.is_alive = false;
                        remote.targeted_time = 0.0;
                        new_kills.push(peer_id);
                        self.pending_death_sounds += 1;
                        self.local_kills += 1;
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

        if !new_kills.is_empty() {
            self.pending_kills.extend(new_kills);
            self.update_player_count_display();
        }
    }

    /// Take pending kills to be sent over network
    pub fn take_pending_kills(&mut self) -> Vec<PeerId> {
        std::mem::take(&mut self.pending_kills)
    }

    /// Take count of deaths that need sound effects
    pub fn take_death_sounds(&mut self) -> u32 {
        std::mem::take(&mut self.pending_death_sounds)
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

    /// Get list of enemies who are currently looking at us (potential threats)
    /// Returns Vec of (peer_id, enemy_position) for each threat
    pub fn get_threats(&self) -> Vec<(PeerId, Vec3)> {
        if self.is_dead {
            return Vec::new();
        }

        let my_head = self.player.eye_position();
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();
        let mut threats = Vec::new();

        for (&peer_id, remote) in &self.remote_players {
            // Skip dead players (mannequins allowed for testing)
            if !remote.is_alive {
                continue;
            }

            // Coward mask cannot kill, so not a threat
            if remote.mask == MaskType::Coward {
                continue;
            }

            let enemy_eye = remote.eye_position();
            let enemy_look_dir = look_direction_from_angles(remote.yaw, remote.pitch);

            // Vector from enemy to us
            let to_us = my_head - enemy_eye;
            let distance = to_us.length();

            if distance < 1.0 {
                continue;
            }

            let to_us_normalized = to_us / distance;
            let dot = enemy_look_dir.dot(to_us_normalized).clamp(-1.0, 1.0);
            let angle = dot.acos();

            // Check if we're within their targeting cone
            if angle < half_angle_rad {
                // Optionally check line-of-sight (enemy can see us)
                if self.physics.is_visible(enemy_eye, my_head) {
                    threats.push((peer_id, remote.head_position()));
                }
            }
        }

        threats
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
                update_peer_id_display(id);
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
                log::info!(
                    "Peer {} joined, existing remote_players: {:?}",
                    id,
                    self.remote_players.keys().collect::<Vec<_>>()
                );
                // Remove mannequins when a real player connects
                self.remote_players.remove(&u64::MAX);
                self.remote_players.remove(&(u64::MAX - 1));
                let remote = RemotePlayer::new();
                log::info!(
                    "Created RemotePlayer for peer {} at pos=[{:.1}, {:.1}, {:.1}], is_alive={}",
                    id,
                    remote.position.x,
                    remote.position.y,
                    remote.position.z,
                    remote.is_alive
                );
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
                self.update_player_count_display();
            }
            NetworkEvent::PlayerState {
                id,
                position,
                yaw,
                pitch,
                mask,
            } => {
                if let Some(remote) = self.remote_players.get_mut(&id) {
                    // Assume ~20 Hz network updates for velocity estimation
                    remote.update_position(position, 0.05);
                    remote.yaw = yaw;
                    remote.pitch = pitch;
                    remote.mask = MaskType::from_u8(mask);
                    log::debug!(
                        "Updated remote player {}: pos=[{:.1}, {:.1}, {:.1}], alive={}",
                        id,
                        position.x,
                        position.y,
                        position.z,
                        remote.is_alive
                    );
                } else {
                    log::warn!(
                        "PlayerState for unknown peer {}, known peers: {:?}",
                        id,
                        self.remote_players.keys().collect::<Vec<_>>()
                    );
                }
            }
            NetworkEvent::PlayerKilled {
                killer_id,
                victim_id,
            } => {
                log::info!("Player {} was killed by {}", victim_id, killer_id);

                // Increment killer's kill count
                if let Some(killer) = self.remote_players.get_mut(&killer_id) {
                    killer.kills += 1;
                }

                // Check if we are the victim
                if let Some(local_id) = local_peer_id
                    && victim_id == local_id
                {
                    // Record death location before marking as dead
                    self.death_locations
                        .push(DeathMarker::new(self.player.position));
                    self.is_dead = true;
                    self.just_died = true;
                    // Record death state for grace period targeting
                    self.death_state = Some(DeathState {
                        time: Instant::now(),
                        position: self.player.eye_position(),
                        yaw: self.player.yaw,
                        pitch: self.player.pitch,
                        mask: self.player.mask,
                    });
                    self.pending_death_sounds += 1;
                    // Get killer name if available
                    let killer_name = self
                        .remote_players
                        .get(&killer_id)
                        .and_then(|p| p.name.clone());
                    show_death_overlay(killer_name);
                } else if let Some(remote) = self.remote_players.get_mut(&victim_id) {
                    // Record death location
                    self.death_locations.push(DeathMarker::new(remote.position));
                    // Another player was killed (not by us - our kills already triggered sound)
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                    if local_peer_id != Some(killer_id) {
                        self.pending_death_sounds += 1;
                    }
                }
                self.update_player_count_display();
            }
            NetworkEvent::PeerIntroduction { id, name } => {
                log::info!("Peer {} is named '{}'", id, name);
                if let Some(remote) = self.remote_players.get_mut(&id) {
                    remote.name = Some(name);
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
                    self.death_state = None;
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
                // Determine winner (survivor)
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

                // Build scoreboard
                let mut scores: Vec<(String, u32, bool, bool)> = Vec::new();

                // Add local player
                let local_name = self.local_name.clone().unwrap_or_else(|| "You".to_string());
                let local_is_survivor = self.winner_id == self.local_peer_id;
                scores.push((local_name, self.local_kills, true, local_is_survivor));

                // Add remote players
                for (&id, player) in &self.remote_players {
                    let name = player
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Player {}", id));
                    let is_survivor = self.winner_id == Some(id);
                    scores.push((name, player.kills, false, is_survivor));
                }

                // Sort by kills descending
                scores.sort_by(|a, b| b.1.cmp(&a.1));

                show_victory_overlay(is_local_winner, scores);
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

        // Update countdown timers
        match self.phase {
            GamePhase::GracePeriod => update_countdown_display(self.phase_timer.ceil() as u32),
            GamePhase::Victory => update_victory_countdown(self.phase_timer.ceil() as u32),
            _ => {}
        }
    }

    /// Get physics debug info for the debug overlay
    pub fn get_physics_debug(&self) -> PhysicsDebugInfo {
        self.physics.get_debug_info(self.player.position).into()
    }

    /// Update peer connection stats if enough time has passed
    pub fn update_peer_stats(&mut self, network: &NetworkClient) {
        const STATS_UPDATE_INTERVAL_SECS: f32 = 2.0;

        if self.last_stats_update.elapsed().as_secs_f32() >= STATS_UPDATE_INTERVAL_SECS {
            self.last_stats_update = Instant::now();

            let peer_connections = network.get_peer_connections();
            if peer_connections.is_empty() {
                // Update UI to show no peers
                update_peer_stats_display(&[]);
                return;
            }

            // Collect peer names from remote players
            let peer_data: Vec<_> = peer_connections
                .into_iter()
                .map(|(peer_id, pc)| {
                    let name = self
                        .remote_players
                        .get(&peer_id)
                        .and_then(|p| p.name.clone());
                    (peer_id, name, pc)
                })
                .collect();

            // Spawn async task to fetch stats
            wasm_bindgen_futures::spawn_local(async move {
                let mut all_stats = Vec::new();
                for (peer_id, name, pc) in peer_data {
                    if let Some(stats) = fetch_peer_stats(peer_id, name, pc).await {
                        all_stats.push(stats);
                    }
                }
                update_peer_stats_display(&all_stats);
            });
        }
    }

    fn update_player_count_display(&self) {
        // Filter out mannequins (debug entities with special IDs)
        let is_real_player = |id: u64| id != u64::MAX && id != u64::MAX - 1;

        // Count alive/dead remote players
        let remote_alive = self
            .remote_players
            .iter()
            .filter(|(id, p)| is_real_player(**id) && p.is_alive)
            .count();
        let remote_dead = self
            .remote_players
            .iter()
            .filter(|(id, p)| is_real_player(**id) && !p.is_alive)
            .count();

        // Local player state
        let is_spectating = self.phase == GamePhase::Spectating;
        let local_alive = if !self.is_dead && !is_spectating {
            1
        } else {
            0
        };
        let local_dead = if self.is_dead { 1 } else { 0 };
        let local_spectating = if is_spectating { 1 } else { 0 };

        let alive = remote_alive + local_alive;
        let dead = remote_dead + local_dead;
        let spectating = local_spectating; // Only local player can be spectating (we don't track remote)

        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(e) = doc.get_element_by_id("count-alive") {
                e.set_text_content(Some(&alive.to_string()));
            }
            if let Some(e) = doc.get_element_by_id("count-dead") {
                e.set_text_content(Some(&dead.to_string()));
            }
            if let Some(e) = doc.get_element_by_id("count-spectator") {
                e.set_text_content(Some(&spectating.to_string()));
            }
        }
    }
}

fn update_peer_id_display(id: PeerId) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id("local-peer-id")
    {
        e.set_text_content(Some(&id.to_string()));
    }
}

fn update_player_name_display(name: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id("local-player-name")
    {
        e.set_text_content(Some(name));
    }
}

fn show_death_overlay(killer_name: Option<String>) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(overlay) = doc.get_element_by_id("death-overlay") {
            let _ = overlay.set_attribute("style", "display: block;");
        }
        if let Some(killer_elem) = doc.get_element_by_id("killer-id") {
            let display_name = killer_name.unwrap_or_else(|| "Unknown".to_string());
            killer_elem.set_text_content(Some(&display_name));
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

fn update_victory_countdown(seconds: u32) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id("victory-countdown")
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

/// Show victory overlay with scoreboard
/// scores: Vec of (name, kills, is_local, is_survivor)
fn show_victory_overlay(is_local_winner: bool, scores: Vec<(String, u32, bool, bool)>) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };

    if let Some(overlay) = doc.get_element_by_id("victory-overlay") {
        let _ = overlay.set_attribute("style", "display: block;");
    }
    if let Some(title) = doc.get_element_by_id("victory-title") {
        title.set_text_content(Some(if is_local_winner {
            "VICTORY!"
        } else {
            "DEFEATED"
        }));
    }

    // Build scoreboard HTML
    if let Some(scoreboard) = doc.get_element_by_id("scoreboard") {
        let mut html = String::new();
        for (name, kills, is_local, is_survivor) in scores {
            let mut classes = Vec::new();
            if is_local {
                classes.push("local");
            }
            if is_survivor {
                classes.push("survivor");
            }
            let class_str = classes.join(" ");
            html.push_str(&format!(
                r#"<div class="score-row {}"><span class="name">{}</span><span class="kills">{}</span></div>"#,
                class_str, name, kills
            ));
        }
        scoreboard.set_inner_html(&html);
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

fn update_mask_selector(mask: MaskType) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };

    let mask_ids = ["mask-ghost", "mask-coward", "mask-hunter"];
    let active_id = match mask {
        MaskType::Ghost => "mask-ghost",
        MaskType::Coward => "mask-coward",
        MaskType::Hunter => "mask-hunter",
    };

    for id in mask_ids {
        if let Some(elem) = doc.get_element_by_id(id) {
            if id == active_id {
                let _ = elem.set_attribute("class", "mask-slot active");
            } else {
                let _ = elem.set_attribute("class", "mask-slot");
            }
        }
    }
}

/// Initialize mask selector images with embedded data URLs
pub fn init_mask_images() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };

    let engine = base64::engine::general_purpose::STANDARD;

    let masks = [
        ("mask-ghost", GHOST_IMAGE),
        ("mask-coward", COWARD_IMAGE),
        ("mask-hunter", HUNTER_IMAGE),
    ];

    for (id, image_data) in masks {
        if let Some(elem) = doc.get_element_by_id(id)
            && let Some(img) = elem.query_selector("img").ok().flatten()
        {
            let b64 = engine.encode(image_data);
            let data_url = format!("data:image/png;base64,{}", b64);
            let _ = img.set_attribute("src", &data_url);
        }
    }
}
