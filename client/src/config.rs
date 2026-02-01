// Player dimensions and physics
pub const PLAYER_HEIGHT: f32 = 80.0;
pub const PLAYER_WIDTH: f32 = 22.0;
pub const EYE_HEIGHT: f32 = 70.0;
pub const STEP_OVER_HEIGHT: f32 = 28.0;
pub const GROUND_SNAP_MARGIN: f32 = 5.0;

// Movement
pub const MOVE_SPEED: f32 = 350.0;
pub const MOUSE_SENSITIVITY: f32 = 0.0025;

// Jump physics (derived: GRAVITY = 8*H/T², JUMP_VELOCITY = 4*H/T)
pub const JUMP_HEIGHT: f32 = 70.0;
pub const JUMP_DURATION: f32 = 0.4;
pub const GRAVITY: f32 = 8.0 * JUMP_HEIGHT / (JUMP_DURATION * JUMP_DURATION);
pub const JUMP_VELOCITY: f32 = 4.0 * JUMP_HEIGHT / JUMP_DURATION;

// Collision: path check (anti-tunnelling)
pub const PATH_HIT_MARGIN: f32 = 2.0;

// Map
pub const RESPAWN_MARGIN: f32 = 500.0;
pub const SPAWN_POINTS: &[[f32; 3]] = &[
    [-408.5, -127.0, 2414.2],
    [-196.2, -127.0, 2417.7],
    [-277.4, -127.0, 2204.3],
    [299.0, 0.0, 498.4],
    [657.3, 0.0, 412.4],
    [-58.9, 0.0, 347.5],
];

// Targeting system
pub const TARGETING_ANGLE: f32 = 60.0;
pub const TARGETING_DURATION: f32 = 1.0;
pub const DEATH_GRACE_PERIOD: f32 = 0.05; // 50ms for continued targeting after death

// Mask system
pub const COWARD_SPEED_MULTIPLIER: f32 = 1.5;
pub const COWARD_JUMP_BOOST: f32 = 10000.0; // Horizontal velocity boost 
pub const HUNTER_KILL_DURATION: f32 = 0.7;
pub const HUNTER_CONE_LENGTH: f32 = 5000.0;
pub const HUNTER_CONE_ALPHA: f32 = 0.3;

// AFK timeout
pub const AFK_TIMEOUT_SECONDS: f32 = 600.0; // 10 minutes

// Debug options
pub const DEBUG_MANNEQUINS: bool = true;
