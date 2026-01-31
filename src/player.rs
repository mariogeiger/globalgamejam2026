use glam::{Mat4, Vec3};
use winit::keyboard::KeyCode;
use std::collections::HashSet;

pub struct Player {
    pub position: Vec3,
    pub yaw: f32,   // Horizontal rotation (radians)
    pub pitch: f32, // Vertical rotation (radians)
    pub velocity: Vec3,
    pub on_ground: bool,
    
    // Input state
    pressed_keys: HashSet<KeyCode>,
    mouse_delta: (f32, f32),
}

impl Player {
    pub fn new(spawn_position: Vec3) -> Self {
        Self {
            position: spawn_position,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            on_ground: false,
            pressed_keys: HashSet::new(),
            mouse_delta: (0.0, 0.0),
        }
    }
    
    pub fn handle_key_press(&mut self, key: KeyCode) {
        self.pressed_keys.insert(key);
    }
    
    pub fn handle_key_release(&mut self, key: KeyCode) {
        self.pressed_keys.remove(&key);
    }
    
    pub fn handle_mouse_move(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }
    
    pub fn update(&mut self, dt: f32) {
        const MOUSE_SENSITIVITY: f32 = 0.002;
        const MOVE_SPEED: f32 = 200.0; // Units per second
        const GRAVITY: f32 = 800.0;
        const JUMP_VELOCITY: f32 = 300.0;
        const FRICTION: f32 = 10.0;
        
        // Apply mouse look (positive dx = look right)
        self.yaw += self.mouse_delta.0 * MOUSE_SENSITIVITY;
        self.pitch -= self.mouse_delta.1 * MOUSE_SENSITIVITY;
        self.pitch = self.pitch.clamp(-1.5, 1.5); // Limit vertical look
        self.mouse_delta = (0.0, 0.0);
        
        // Calculate forward and right vectors
        let forward = Vec3::new(
            self.yaw.sin(),
            0.0,
            -self.yaw.cos(),
        ).normalize();
        
        let right = Vec3::new(
            self.yaw.cos(),
            0.0,
            self.yaw.sin(),
        ).normalize();
        
        // Movement input
        let mut move_dir = Vec3::ZERO;
        
        if self.pressed_keys.contains(&KeyCode::KeyW) {
            move_dir += forward;
        }
        if self.pressed_keys.contains(&KeyCode::KeyS) {
            move_dir -= forward;
        }
        if self.pressed_keys.contains(&KeyCode::KeyD) {
            move_dir += right;
        }
        if self.pressed_keys.contains(&KeyCode::KeyA) {
            move_dir -= right;
        }
        
        // Normalize diagonal movement
        if move_dir.length_squared() > 0.0 {
            move_dir = move_dir.normalize();
        }
        
        // Apply movement
        if self.on_ground {
            // Ground movement with friction
            self.velocity.x = move_dir.x * MOVE_SPEED;
            self.velocity.z = move_dir.z * MOVE_SPEED;
            
            // Jump
            if self.pressed_keys.contains(&KeyCode::Space) {
                self.velocity.y = JUMP_VELOCITY;
                self.on_ground = false;
            }
        } else {
            // Air control (reduced)
            self.velocity.x += move_dir.x * MOVE_SPEED * 0.1 * dt;
            self.velocity.z += move_dir.z * MOVE_SPEED * 0.1 * dt;
        }
        
        // Apply gravity
        if !self.on_ground {
            self.velocity.y -= GRAVITY * dt;
        }
        
        // Apply velocity to position
        self.position += self.velocity * dt;
        
        // Ground friction when on ground
        if self.on_ground {
            let friction_factor = 1.0 - FRICTION * dt;
            self.velocity.x *= friction_factor.max(0.0);
            self.velocity.z *= friction_factor.max(0.0);
        }
    }
    
    /// Get the view matrix for rendering
    pub fn view_matrix(&self) -> Mat4 {
        let eye = self.position + Vec3::new(0.0, 64.0, 0.0); // Eye height
        
        // Calculate look direction from yaw and pitch
        let look_dir = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        ).normalize();
        
        let target = eye + look_dir;
        
        Mat4::look_at_rh(eye, target, Vec3::Y)
    }
    
    /// Get player forward direction (horizontal only)
    #[allow(dead_code)]
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos()).normalize()
    }
    
    /// Set ground state and handle landing
    pub fn set_on_ground(&mut self, on_ground: bool, ground_y: Option<f32>) {
        if on_ground && !self.on_ground {
            // Just landed
            self.velocity.y = 0.0;
        }
        self.on_ground = on_ground;
        
        if let Some(y) = ground_y {
            if self.on_ground {
                self.position.y = y;
            }
        }
    }
    
    /// Get the player's bounding box for collision
    #[allow(dead_code)]
    pub fn get_bounds(&self) -> (Vec3, Vec3) {
        const PLAYER_WIDTH: f32 = 32.0;
        const PLAYER_HEIGHT: f32 = 72.0;
        
        let min = self.position - Vec3::new(PLAYER_WIDTH / 2.0, 0.0, PLAYER_WIDTH / 2.0);
        let max = self.position + Vec3::new(PLAYER_WIDTH / 2.0, PLAYER_HEIGHT, PLAYER_WIDTH / 2.0);
        (min, max)
    }
}
