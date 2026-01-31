// Player dimensions and physics
pub const PLAYER_HEIGHT: f32 = 72.0;
pub const PLAYER_WIDTH: f32 = 32.0;
pub const EYE_HEIGHT: f32 = 64.0;

// Movement
pub const MOVE_SPEED: f32 = 200.0;
pub const JUMP_VELOCITY: f32 = 300.0;
pub const GRAVITY: f32 = 800.0;
pub const MOUSE_SENSITIVITY: f32 = 0.002;
pub const FRICTION: f32 = 10.0;

// Map
pub const SPAWN_SCALE: f32 = 64.0;
pub const RESPAWN_MARGIN: f32 = 500.0;

// Collision ray distances
pub const GROUND_CHECK_OFFSET: f32 = 40.0;
pub const GROUND_CHECK_MAX: f32 = 100.0;
pub const GROUND_HIT_THRESHOLD: f32 = 50.0;
pub const CEILING_CHECK_OFFSET: f32 = 72.0;
pub const CEILING_CHECK_MAX: f32 = 50.0;
pub const CEILING_HIT_THRESHOLD: f32 = 10.0;
pub const WALL_CHECK_OFFSET: f32 = 36.0;
pub const WALL_CHECK_DIST: f32 = 20.0;
