// Hunter mask: Clean view

@group(0) @binding(0) var t_scene: texture_2d<f32>;
@group(0) @binding(1) var s_scene: sampler;
@group(0) @binding(2) var t_position: texture_2d<f32>;
@group(0) @binding(3) var t_velocity: texture_2d<f32>;

@group(1) @binding(0) var<uniform> params: Params;

struct Params {
    inv_view: mat4x4<f32>,
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
}

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coord: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.tex_coord;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Use textureLoad for unfilterable float textures (Rgba32Float)
    let pixel_coord = vec2<i32>(in.uv * params.resolution);
    let position = textureLoad(t_position, pixel_coord, 0);
    let view_pos = vec4<f32>(position.xyz, 1.0);
    let world_position = params.inv_view * view_pos;
    let col = textureSample(t_scene, s_scene, in.uv);
    let c = cos((1./8.)*2*3.1415*world_position.xyz);
    let d = 1. - c.x*c.y*c.z;
    if(position.z < 0) {
        return vec4(mix(col.xyz,vec3(d),d*exp(.002*position.z)), 1.0);
    }
    return col;
}
