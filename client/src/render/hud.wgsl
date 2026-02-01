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
    let view_pos = vec4<f32>(
        in.position.x * hud.reticle_radius,
        in.position.y * hud.reticle_radius,
        -hud.reticle_distance,
        1.0
    );
    out.clip_position = hud.projection * view_pos;
    out.local_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(in.local_pos);
    if dist > 1.0 { discard; }

    let edge = smoothstep(1.0, 0.95, dist);
    let fill = smoothstep(1.0 - hud.targeting_progress, 1.05 - hud.targeting_progress, dist);

    let color = mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(1.0, 0.3, 0.2), fill);
    let alpha = edge * mix(0.25, 0.5, fill * hud.has_target);

    if alpha < 0.01 { discard; }
    return vec4<f32>(color, alpha);
}
