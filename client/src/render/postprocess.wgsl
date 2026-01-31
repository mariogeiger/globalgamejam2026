@group(0) @binding(0)
var t_scene: texture_2d<f32>;
@group(0) @binding(1)
var s_scene: sampler;

@group(1) @binding(0)
var<uniform> params: PostProcessParams;

@group(2) @binding(0)
var t_previous: texture_2d<f32>;
@group(2) @binding(1)
var s_previous: sampler;

@group(3) @binding(0)
var t_depth: texture_depth_2d;
@group(3) @binding(1)
var s_depth: sampler;

struct PostProcessParams {
    blur_direction: vec2<f32>,
    blur_strength: f32,
    smear_factor: f32,
    resolution: vec2<f32>,
    depth_near: f32,
    depth_far: f32,
}

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coord: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.tex_coord = in.tex_coord;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel_x = in.tex_coord.x * params.resolution.x;
    let pixel_y = in.tex_coord.y * params.resolution.y;
    let depth_buffer = textureSample(t_depth, s_depth, in.tex_coord);
    let near = params.depth_near;
    let far = params.depth_far;
    let d = (near * far) / (far - depth_buffer * (far - near));

    let current = textureSample(t_scene, s_scene, in.tex_coord);

    let d_vis = .5 + .5 * cos(d * 0.01 * 3.14159265358979323846);

    let previous = textureSample(t_previous, s_previous, in.tex_coord);
    let smear = mix(current, previous, params.smear_factor * d_vis);
    return smear;
}
