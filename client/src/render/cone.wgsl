struct CameraUniform {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    player_velocity: vec4<f32>,
};

struct ConeUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> cone: ConeUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec3<f32>,
    @location(1) view_pos: vec3<f32>,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @location(1) position: vec4<f32>,
    @location(2) velocity: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = cone.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.local_pos = in.position;
    out.view_pos = (camera.view * world_pos).xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    // Fade alpha based on distance from apex (z=0 is apex, z increases towards base)
    let fade = clamp(in.local_pos.z / 500.0, 0.0, 1.0);
    let alpha = cone.color.a * (1.0 - fade * 0.5);
    
    var out: FragmentOutput;
    out.color = vec4<f32>(cone.color.rgb, alpha);
    out.position = vec4<f32>(in.view_pos, length(in.view_pos));
    out.velocity = vec4<f32>(-camera.player_velocity.xyz, 0.0);
    return out;
}
