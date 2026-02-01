// View mask: rendered in view space (relative to camera, projection only).

struct ViewMaskUniform {
    projection: mat4x4<f32>,
    model: mat4x4<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> u: ViewMaskUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_pos: vec3<f32>,
    @location(1) view_normal: vec3<f32>,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @location(1) position: vec4<f32>,
    @location(2) velocity: vec4<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Model transforms directly to view space
    let view_pos = u.model * vec4<f32>(in.position, 1.0);
    out.clip_position = u.projection * view_pos;
    out.view_pos = view_pos.xyz;
    out.view_normal = (u.model * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let normal = normalize(in.view_normal);
    let light_dir = normalize(vec3<f32>(0.0, 0.0, -1.0));
    let diffuse = max(dot(normal, light_dir), 0.0);
    let ambient = 0.6;
    let brightness = ambient + diffuse * 0.4;

    var out: FragmentOutput;
    out.color = vec4<f32>(u.color.rgb * brightness, 1.0);
    out.position = vec4<f32>(in.view_pos, length(in.view_pos));
    out.velocity = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return out;
}
