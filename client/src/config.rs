// Player dimensions and physics
pub const PLAYER_HEIGHT: f32 = 80.0;
pub const PLAYER_WIDTH: f32 = 22.0;
pub const EYE_HEIGHT: f32 = 70.0;
pub const STEP_OVER_HEIGHT: f32 = 28.0; // can step over obstacles this tall
pub const GROUND_SNAP_MARGIN: f32 = 5.0; // extra distance for ground detection tolerance

// Movement
pub const MOVE_SPEED: f32 = 350.0;
pub const MOUSE_SENSITIVITY: f32 = 0.0025;

// Jump physics (derived: GRAVITY = 8*H/T², JUMP_VELOCITY = 4*H/T)
pub const JUMP_HEIGHT: f32 = 70.0; // peak height
pub const JUMP_DURATION: f32 = 0.4; // total air time in seconds
pub const GRAVITY: f32 = 8.0 * JUMP_HEIGHT / (JUMP_DURATION * JUMP_DURATION);
pub const JUMP_VELOCITY: f32 = 4.0 * JUMP_HEIGHT / JUMP_DURATION;

// Map
pub const SPAWN_SCALE: f32 = 64.0;
pub const RESPAWN_MARGIN: f32 = 500.0;
