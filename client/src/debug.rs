use std::time::Duration;
use web_time::Instant;

/// Helper to set text content of an element by ID
fn set_element_text(id: &str, text: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id(id)
    {
        e.set_text_content(Some(text));
    }
}

/// Timing data for a single frame
#[derive(Default, Clone)]
pub struct FrameProfile {
    pub frame_time: Duration,
    pub update_time: Duration,
    pub physics_time: Duration,
    pub targeting_time: Duration,
    pub net_poll_time: Duration,
    pub net_send_time: Duration,
    pub render_time: Duration,
}

/// Delay before starting to track worst-case values (to skip startup spikes)
const WORST_TRACKING_DELAY_SECS: f32 = 10.0;

/// Rolling average for frame timings with worst-case tracking
struct ProfileStats {
    samples: Vec<FrameProfile>,
    max_samples: usize,
    index: usize,
    filled: bool,
    worst: FrameProfile,
    created_at: Instant,
}

impl ProfileStats {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: vec![FrameProfile::default(); max_samples],
            max_samples,
            index: 0,
            filled: false,
            worst: FrameProfile::default(),
            created_at: Instant::now(),
        }
    }

    fn push(&mut self, profile: FrameProfile) {
        // Only track worst values after warmup period to skip startup spikes
        if self.created_at.elapsed().as_secs_f32() >= WORST_TRACKING_DELAY_SECS {
            self.worst.frame_time = self.worst.frame_time.max(profile.frame_time);
            self.worst.update_time = self.worst.update_time.max(profile.update_time);
            self.worst.physics_time = self.worst.physics_time.max(profile.physics_time);
            self.worst.targeting_time = self.worst.targeting_time.max(profile.targeting_time);
            self.worst.net_poll_time = self.worst.net_poll_time.max(profile.net_poll_time);
            self.worst.net_send_time = self.worst.net_send_time.max(profile.net_send_time);
            self.worst.render_time = self.worst.render_time.max(profile.render_time);
        }

        self.samples[self.index] = profile;
        self.index = (self.index + 1) % self.max_samples;
        if self.index == 0 {
            self.filled = true;
        }
    }

    fn worst(&self) -> &FrameProfile {
        &self.worst
    }

    fn average(&self) -> FrameProfile {
        let count = if self.filled {
            self.max_samples
        } else {
            self.index
        };
        if count == 0 {
            return FrameProfile::default();
        }

        let mut sum = FrameProfile::default();
        for i in 0..count {
            sum.frame_time += self.samples[i].frame_time;
            sum.update_time += self.samples[i].update_time;
            sum.physics_time += self.samples[i].physics_time;
            sum.targeting_time += self.samples[i].targeting_time;
            sum.net_poll_time += self.samples[i].net_poll_time;
            sum.net_send_time += self.samples[i].net_send_time;
            sum.render_time += self.samples[i].render_time;
        }

        FrameProfile {
            frame_time: sum.frame_time / count as u32,
            update_time: sum.update_time / count as u32,
            physics_time: sum.physics_time / count as u32,
            targeting_time: sum.targeting_time / count as u32,
            net_poll_time: sum.net_poll_time / count as u32,
            net_send_time: sum.net_send_time / count as u32,
            render_time: sum.render_time / count as u32,
        }
    }
}

/// Debug overlay that handles all profiling and debug display
pub struct DebugOverlay {
    stats: ProfileStats,
    current: FrameProfile,
    last_frame_start: Instant,
    frame_start: Instant,
    last_display_update: Instant,
    /// Temporary storage for section start times
    section_start: Option<Instant>,
}

impl DebugOverlay {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            stats: ProfileStats::new(60), // ~1 second at 60fps
            current: FrameProfile::default(),
            last_frame_start: now,
            frame_start: now,
            last_display_update: now,
            section_start: None,
        }
    }

    /// Call at the start of each frame
    pub fn begin_frame(&mut self) {
        self.frame_start = Instant::now();
        let frame_time = self.frame_start - self.last_frame_start;
        self.last_frame_start = self.frame_start;

        // Push previous frame's profile
        self.current.frame_time = frame_time;
        self.stats.push(self.current.clone());
        self.current = FrameProfile::default();
    }

    /// Start timing a section
    pub fn begin_section(&mut self) {
        self.section_start = Some(Instant::now());
    }

    /// End timing and record to the specified field
    pub fn end_net_poll(&mut self) {
        if let Some(start) = self.section_start.take() {
            self.current.net_poll_time = start.elapsed();
        }
    }

    pub fn end_update(&mut self) {
        if let Some(start) = self.section_start.take() {
            self.current.update_time = start.elapsed();
        }
    }

    pub fn end_net_send(&mut self) {
        if let Some(start) = self.section_start.take() {
            self.current.net_send_time = start.elapsed();
        }
    }

    pub fn end_render(&mut self) {
        if let Some(start) = self.section_start.take() {
            self.current.render_time = start.elapsed();
        }
    }

    /// Record physics timing (called from game update)
    pub fn record_physics(&mut self, duration: Duration) {
        self.current.physics_time = duration;
    }

    /// Record targeting timing (called from game update)
    pub fn record_targeting(&mut self, duration: Duration) {
        self.current.targeting_time = duration;
    }

    /// Update the debug display (throttled to 10Hz)
    pub fn update_display(
        &mut self,
        player_pos: glam::Vec3,
        player_vel: glam::Vec3,
        physics_debug: &PhysicsDebugInfo,
    ) {
        const UPDATE_INTERVAL_SECS: f32 = 0.1;

        if self.last_display_update.elapsed().as_secs_f32() < UPDATE_INTERVAL_SECS {
            return;
        }
        self.last_display_update = Instant::now();

        // Ground info
        let ground_text = match physics_debug.ground_distance {
            Some(d) => format!(
                "{} ({:.1})",
                if physics_debug.on_ground { "Y" } else { "N" },
                d
            ),
            None => "N (-)".to_string(),
        };
        set_element_text("dbg-ground", &ground_text);

        // Velocity
        set_element_text(
            "dbg-velocity",
            &format!(
                "[{:.0}, {:.0}, {:.0}]",
                player_vel.x, player_vel.y, player_vel.z
            ),
        );

        // Wall distances
        let fmt_dist = |d: Option<f32>| match d {
            Some(v) => format!("{:.0}", v),
            None => "-".to_string(),
        };
        let walls = &physics_debug.wall_distances;
        set_element_text(
            "dbg-walls",
            &format!(
                "+X:{} -X:{} +Z:{} -Z:{}",
                fmt_dist(walls[0]),
                fmt_dist(walls[1]),
                fmt_dist(walls[2]),
                fmt_dist(walls[3])
            ),
        );

        // Position
        set_element_text(
            "local-pos",
            &format!(
                "[{:.1}, {:.1}, {:.1}]",
                player_pos.x, player_pos.y, player_pos.z
            ),
        );

        // Timing stats
        let avg = self.stats.average();
        let worst = self.stats.worst();
        let to_ms = |d: Duration| d.as_secs_f64() * 1000.0;

        let fmt2 = |a: Duration, w: Duration| format!("{:.2} ({:.2})", to_ms(a), to_ms(w));
        let fmt3 = |a: Duration, w: Duration| format!("{:.3} ({:.3})", to_ms(a), to_ms(w));

        set_element_text("dbg-frame-time", &fmt2(avg.frame_time, worst.frame_time));
        set_element_text("dbg-update-time", &fmt2(avg.update_time, worst.update_time));
        set_element_text(
            "dbg-physics-time",
            &fmt2(avg.physics_time, worst.physics_time),
        );
        set_element_text(
            "dbg-targeting-time",
            &fmt3(avg.targeting_time, worst.targeting_time),
        );
        set_element_text(
            "dbg-net-poll-time",
            &fmt3(avg.net_poll_time, worst.net_poll_time),
        );
        set_element_text(
            "dbg-net-send-time",
            &fmt3(avg.net_send_time, worst.net_send_time),
        );
        set_element_text("dbg-render-time", &fmt2(avg.render_time, worst.render_time));

        // FPS
        let fps_avg = if avg.frame_time.as_secs_f64() > 0.0 {
            1.0 / avg.frame_time.as_secs_f64()
        } else {
            0.0
        };
        let fps_min = if worst.frame_time.as_secs_f64() > 0.0 {
            1.0 / worst.frame_time.as_secs_f64()
        } else {
            0.0
        };
        set_element_text("dbg-fps", &format!("{:.0} ({:.0})", fps_avg, fps_min));
    }
}

/// Physics debug info passed to the overlay
pub struct PhysicsDebugInfo {
    pub on_ground: bool,
    pub ground_distance: Option<f32>,
    pub wall_distances: [Option<f32>; 4], // +X, -X, +Z, -Z
}

impl From<crate::collision::CollisionDebug> for PhysicsDebugInfo {
    fn from(debug: crate::collision::CollisionDebug) -> Self {
        Self {
            on_ground: debug.on_ground,
            ground_distance: debug.ground_distance,
            wall_distances: debug.wall_distances,
        }
    }
}
