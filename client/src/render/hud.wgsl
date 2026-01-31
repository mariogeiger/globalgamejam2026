struct HudUniform {
    projection: mat4x4<f32>,
    targeting_progress: f32,
    has_target: f32,
    reticle_distance: f32,
    reticle_radius: f32,
}

@group(0) @binding(0)
var<uniform> hud: HudUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Position in view space: circle at distance along -Z axis
    let view_pos = vec4<f32>(
        in.position.x * hud.reticle_radius,
        in.position.y * hud.reticle_radius,
        -hud.reticle_distance,
        1.0
    );
    
    // Apply projection to get clip space position
    out.clip_position = hud.projection * view_pos;
    out.local_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(in.local_pos);
    
    if dist > 1.0 {
        discard;
    }
    
    let ring_outer = 1.0;
    let ring_inner = 0.85;
    let ring = smoothstep(ring_inner, ring_inner + 0.05, dist) * smoothstep(ring_outer + 0.05, ring_outer, dist);
    
    let fill_amount = hud.targeting_progress;
    let angle = atan2(in.local_pos.y, in.local_pos.x);
    let normalized_angle = (angle + 3.14159265) / (2.0 * 3.14159265);
    let fill = step(normalized_angle, fill_amount) * step(dist, ring_inner);
    
    let base_color = vec3<f32>(1.0, 1.0, 1.0);
    let target_color = vec3<f32>(1.0, 0.3, 0.2);
    let color = mix(base_color, target_color, hud.has_target);
    
    let alpha = max(ring * 0.6, fill * 0.3 * hud.has_target);
    
    if alpha < 0.01 {
        discard;
    }
    
    return vec4<f32>(color, alpha);
}
