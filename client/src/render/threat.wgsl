struct ThreatUniform {
    screen_size: vec2<f32>,  // offset 0, 8 bytes
    arrow_angle: f32,        // offset 8, 4 bytes
    arrow_alpha: f32,        // offset 12, 4 bytes
    time: f32,               // offset 16, 4 bytes
    _pad1: f32,              // offset 20, 4 bytes
    _pad2: f32,              // offset 24, 4 bytes
    _pad3: f32,              // offset 28, 4 bytes
    // Total: 32 bytes, 16-byte aligned
}

@group(0) @binding(0)
var<uniform> threat: ThreatUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
}

// Arrow size constants
const ARROW_SIZE: f32 = 0.06;      // Size relative to screen
const EDGE_MARGIN: f32 = 0.12;     // Distance from edge

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Rotate vertex by arrow angle
    let cos_a = cos(threat.arrow_angle);
    let sin_a = sin(threat.arrow_angle);
    let rotated = vec2<f32>(
        in.position.x * cos_a - in.position.y * sin_a,
        in.position.x * sin_a + in.position.y * cos_a
    );
    
    // Scale arrow
    let scaled = rotated * ARROW_SIZE;
    
    // Position at edge of screen in direction of threat
    // Arrow points outward toward the threat
    let edge_dist = 1.0 - EDGE_MARGIN;
    let center = vec2<f32>(
        cos(threat.arrow_angle) * edge_dist,
        sin(threat.arrow_angle) * edge_dist
    );
    
    // Correct for aspect ratio
    let aspect = threat.screen_size.x / threat.screen_size.y;
    var pos = center + scaled;
    pos.x = pos.x / aspect;
    
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    out.local_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Pulsing effect
    let pulse = 0.7 + 0.3 * sin(threat.time * 4.0);
    
    // Red/orange warning color
    let color = vec3<f32>(1.0, 0.3, 0.1);
    
    // Smooth edges
    let dist_from_center = length(in.local_pos);
    let edge_alpha = 1.0 - smoothstep(0.6, 1.0, dist_from_center);
    
    let alpha = threat.arrow_alpha * pulse * edge_alpha;
    
    if alpha < 0.01 { discard; }
    return vec4<f32>(color, alpha);
}
