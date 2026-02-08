use glam::Vec3;
use rand::Rng;
use std::collections::HashMap;
use web_time::Instant;

use crate::collision::PhysicsWorld;
use crate::combat;
use crate::config::*;
use crate::debug::PhysicsDebugInfo;
use crate::game_ui;
use crate::input::InputState;
use crate::mesh::Mesh;
use crate::network::{
    GamePhase, NetworkClient, NetworkEvent, PeerId, fetch_peer_stats, update_peer_stats_display,
};
use crate::player::{MaskType, Player, RemotePlayer};
use winit::keyboard::KeyCode;

// Re-exports so render/mod.rs and main.rs don't need import changes
pub use crate::combat::DeathMarker;
pub use crate::game_ui::init_mask_images;

/// State captured at moment of death for grace period targeting
struct DeathState {
    time: Instant,
    position: Vec3,
    yaw: f32,
    pitch: f32,
    mask: MaskType,
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
    /// Kill feed for the current round: (killer_name, victim_name)
    kill_feed: Vec<(String, String)>,
}

impl GameState {
    pub fn new(mesh: &Mesh, debug_mannequins: bool) -> Self {
        let spawn_idx = rand::rng().random_range(0..SPAWN_POINTS.len());
        let initial_spawn = Self::get_spawn_point(spawn_idx);

        let player = Player::new(initial_spawn);

        let (collision_vertices, collision_indices, bounds) = Self::extract_collision_data(mesh);
        let physics = PhysicsWorld::new(&collision_vertices, &collision_indices)
            .expect("Failed to create physics world");

        let mut remote_players = HashMap::new();
        if debug_mannequins && SPAWN_POINTS.len() >= 2 {
            let mut mannequin1 = RemotePlayer::new();
            mannequin1.position = Self::get_spawn_point((spawn_idx + 1) % SPAWN_POINTS.len());
            mannequin1.mask = MaskType::Hunter;
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
            kill_feed: Vec::new(),
        }
    }

    pub fn set_local_name(&mut self, name: String) {
        self.local_name = Some(name.clone());
        game_ui::update_player_name(&name);
    }

    // -----------------------------------------------------------------------
    // Map / spawn helpers
    // -----------------------------------------------------------------------

    fn extract_collision_data(mesh: &Mesh) -> (Vec<Vec3>, Vec<[u32; 3]>, (Vec3, Vec3)) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for submesh in &mesh.submeshes {
            let base_idx = vertices.len() as u32;
            for v in &submesh.vertices {
                vertices.push(Vec3::from_array(v.position));
            }
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

    pub fn respawn_player(&mut self) {
        self.player.respawn(Self::random_spawn_point());
    }

    // -----------------------------------------------------------------------
    // Per-frame update
    // -----------------------------------------------------------------------

    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = (now - self.last_update).as_secs_f32().min(0.1);
        self.last_update = now;
        self.time += dt;

        if self.phase_timer > 0.0 {
            self.phase_timer = (self.phase_timer - dt).max(0.0);
        }

        self.update_hud_display();
        self.update_mask_input(input);

        let is_spectator =
            self.is_dead || self.phase == GamePhase::Victory || self.phase == GamePhase::Spectating;
        if is_spectator {
            self.player.spectator_update(dt, input);

            // Grace-period targeting from frozen death state
            if self.is_dead
                && let Some(ref death) = self.death_state
                && death.time.elapsed().as_secs_f32() <= DEATH_GRACE_PERIOD
            {
                let result = combat::update_targeting(
                    &mut self.remote_players,
                    dt,
                    death.position,
                    death.yaw,
                    death.pitch,
                    death.mask,
                    &self.physics,
                );
                self.pending_kills.extend(result.kills);
            }
            return;
        }

        // Coward dash
        if self.player.can_dash() && input.is_pressed(KeyCode::Space) {
            let look_dir = self.player.look_direction();
            let origin = self.player.eye_position();
            let target = self.physics.dash_target(origin, look_dir, DASH_DISTANCE);
            let target = target - Vec3::new(0.0, EYE_HEIGHT, 0.0);
            self.player.start_dash(target);
        }

        // Physics
        let prev_pos = self.player.position;
        self.player.update(dt, input);

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

        // Targeting via combat module
        if (self.phase == GamePhase::Playing || self.phase == GamePhase::WaitingForPlayers)
            && !self.is_dead
        {
            let result = combat::update_targeting(
                &mut self.remote_players,
                dt,
                self.player.eye_position(),
                self.player.yaw,
                self.player.pitch,
                self.player.mask,
                &self.physics,
            );
            if !result.kills.is_empty() {
                self.pending_kills.extend(result.kills);
                self.update_player_count_display();
            }
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

        if self.player.mask != old_mask {
            self.mask_change_time = Some(self.time);
            game_ui::update_mask_selector(self.player.mask);
        }
    }

    fn check_respawn(&mut self) {
        let (bounds_min, bounds_max) = self.map_bounds;
        let pos = self.player.position;
        let vel = self.player.velocity;

        let outside = pos.x < bounds_min.x - RESPAWN_MARGIN
            || pos.x > bounds_max.x + RESPAWN_MARGIN
            || pos.y < bounds_min.y - RESPAWN_MARGIN
            || pos.y > bounds_max.y + RESPAWN_MARGIN
            || pos.z < bounds_min.z - RESPAWN_MARGIN
            || pos.z > bounds_max.z + RESPAWN_MARGIN;

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

    // -----------------------------------------------------------------------
    // Public queries (delegated to combat module)
    // -----------------------------------------------------------------------

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
        combat::get_targeting_progress(&self.remote_players, self.player.mask)
    }

    pub fn get_threats(&self) -> Vec<(PeerId, Vec3)> {
        if self.is_dead {
            return Vec::new();
        }
        combat::get_threats(
            &self.remote_players,
            self.player.eye_position(),
            &self.physics,
        )
    }

    /// Get physics debug info for the debug overlay
    pub fn get_physics_debug(&self) -> PhysicsDebugInfo {
        self.physics.get_debug_info(self.player.position).into()
    }

    // -----------------------------------------------------------------------
    // Network event handling (single unified handler for local + remote events)
    // -----------------------------------------------------------------------

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
                game_ui::update_peer_id(id);
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
                    remote.update_position(position, 0.05);
                    remote.yaw = yaw;
                    remote.pitch = pitch;
                    remote.mask = MaskType::from_u8(mask);
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

                // Resolve names for kill feed
                let killer_name = self.resolve_player_name(killer_id, local_peer_id);
                let victim_name = self.resolve_player_name(victim_id, local_peer_id);
                self.kill_feed.push((killer_name, victim_name));

                // Record death location
                let death_pos = if Some(victim_id) == local_peer_id {
                    Some(self.player.position)
                } else {
                    self.remote_players.get(&victim_id).map(|r| r.position)
                };
                if let Some(pos) = death_pos {
                    self.death_locations.push(DeathMarker::new(pos));
                }

                // Increment killer's kill count
                if Some(killer_id) == local_peer_id {
                    self.local_kills += 1;
                } else if let Some(killer) = self.remote_players.get_mut(&killer_id) {
                    killer.kills += 1;
                }

                // Handle victim-specific effects
                if Some(victim_id) == local_peer_id {
                    self.handle_local_death(killer_id);
                } else if let Some(remote) = self.remote_players.get_mut(&victim_id) {
                    // is_alive already set to false by combat for local kills, but
                    // remote kills also arrive here, so ensure it's marked dead.
                    remote.is_alive = false;
                    remote.targeted_time = 0.0;
                }

                self.pending_death_sounds += 1;
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

    /// Handle the local player dying (called from PlayerKilled handler).
    fn handle_local_death(&mut self, killer_id: PeerId) {
        self.is_dead = true;
        self.just_died = true;
        self.death_state = Some(DeathState {
            time: Instant::now(),
            position: self.player.eye_position(),
            yaw: self.player.yaw,
            pitch: self.player.pitch,
            mask: self.player.mask,
        });
        let killer_name = self
            .remote_players
            .get(&killer_id)
            .and_then(|p| p.name.as_deref());
        game_ui::show_death(killer_name);
    }

    /// Resolve a peer ID to a display name.
    fn resolve_player_name(&self, peer_id: PeerId, local_peer_id: Option<PeerId>) -> String {
        if local_peer_id == Some(peer_id) {
            self.local_name.clone().unwrap_or_else(|| "You".to_string())
        } else {
            self.remote_players
                .get(&peer_id)
                .and_then(|p| p.name.clone())
                .unwrap_or_else(|| format!("Player {}", peer_id))
        }
    }

    // -----------------------------------------------------------------------
    // Phase transitions
    // -----------------------------------------------------------------------

    fn set_phase(&mut self, phase: GamePhase, time_remaining: f32) {
        let old_phase = self.phase;
        self.phase = phase;
        self.phase_timer = time_remaining;

        match phase {
            GamePhase::WaitingForPlayers => {
                game_ui::hide_countdown();
                game_ui::hide_round_end();
                game_ui::hide_spectating();
                game_ui::show_waiting();
            }
            GamePhase::GracePeriod => {
                if old_phase != GamePhase::GracePeriod {
                    self.reset_round();
                }
                game_ui::hide_waiting();
                game_ui::show_countdown_timer(time_remaining.ceil() as u32);
            }
            GamePhase::Playing => {
                game_ui::hide_countdown();
                game_ui::hide_waiting();
                game_ui::hide_spectating();
            }
            GamePhase::Victory => {
                self.enter_victory();
            }
            GamePhase::Spectating => {
                game_ui::hide_countdown();
                game_ui::hide_round_end();
                game_ui::hide_death();
                game_ui::show_spectating();
            }
        }
    }

    fn reset_round(&mut self) {
        self.is_dead = false;
        self.death_state = None;
        self.winner_id = None;
        self.pending_kills.clear();
        self.kill_feed.clear();
        self.respawn_player();
        for remote in self.remote_players.values_mut() {
            remote.is_alive = true;
            remote.targeted_time = 0.0;
        }
        game_ui::hide_death();
        game_ui::hide_round_end();
        game_ui::hide_spectating();
    }

    fn enter_victory(&mut self) {
        if !self.is_dead {
            self.winner_id = self.local_peer_id;
        } else {
            self.winner_id = self
                .remote_players
                .iter()
                .find(|(_, p)| p.is_alive)
                .map(|(&id, _)| id);
        }

        let local_survived = !self.is_dead;
        let local_name = self.local_name.clone().unwrap_or_else(|| "You".to_string());

        let mut scores = Vec::new();
        scores.push(game_ui::ScoreEntry {
            name: local_name.clone(),
            kills: self.local_kills,
            is_local: true,
            is_survivor: local_survived,
        });

        let mut survivor_name: Option<String> = None;
        for (&id, player) in &self.remote_players {
            let name = player
                .name
                .clone()
                .unwrap_or_else(|| format!("Player {}", id));
            let is_survivor = player.is_alive;
            if is_survivor {
                survivor_name = Some(name.clone());
            }
            scores.push(game_ui::ScoreEntry {
                name,
                kills: player.kills,
                is_local: false,
                is_survivor,
            });
        }
        if local_survived {
            survivor_name = Some(local_name);
        }

        scores.sort_by(|a, b| b.kills.cmp(&a.kills));

        let outcome = game_ui::RoundOutcome {
            local_survived,
            survivor_name,
            scores,
            kill_feed: self.kill_feed.clone(),
        };
        game_ui::show_round_end(&outcome);
    }

    // -----------------------------------------------------------------------
    // HUD display helpers
    // -----------------------------------------------------------------------

    fn update_hud_display(&self) {
        game_ui::update_position(self.player.position);

        match self.phase {
            GamePhase::GracePeriod => {
                game_ui::update_countdown_timer(self.phase_timer.ceil() as u32)
            }
            GamePhase::Victory => game_ui::update_round_end_timer(self.phase_timer.ceil() as u32),
            _ => {}
        }
    }

    fn update_player_count_display(&self) {
        let is_real_player = |id: u64| id != u64::MAX && id != u64::MAX - 1;

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

        let is_spectating = self.phase == GamePhase::Spectating;
        let local_alive = if !self.is_dead && !is_spectating {
            1
        } else {
            0
        };
        let local_dead = if self.is_dead { 1 } else { 0 };
        let local_spectating = if is_spectating { 1 } else { 0 };

        game_ui::update_player_counts(
            remote_alive + local_alive,
            remote_dead + local_dead,
            local_spectating,
        );
    }

    /// Update peer connection stats if enough time has passed
    pub fn update_peer_stats(&mut self, network: &NetworkClient) {
        const STATS_UPDATE_INTERVAL_SECS: f32 = 2.0;

        if self.last_stats_update.elapsed().as_secs_f32() >= STATS_UPDATE_INTERVAL_SECS {
            self.last_stats_update = Instant::now();

            let peer_connections = network.get_peer_connections();
            if peer_connections.is_empty() {
                update_peer_stats_display(&[]);
                return;
            }

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
}
