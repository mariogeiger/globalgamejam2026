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
    inv_view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    mask_tint: vec3<f32>,
    mask_tint_strength: f32,
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

    // Reconstruct world position from NDC + depth
    let ndc = vec4<f32>(
        2.0 * in.tex_coord.x - 1.0,
        1.0 - 2.0 * in.tex_coord.y,
        depth_buffer,
        1.0,
    );
    let world_h = params.inv_view_proj * ndc;
    let world_position = world_h.xyz / world_h.w;

    // Position relative to camera (view space)
    let screen_space_position = (params.view * vec4<f32>(world_position, 1.0)).xyz;

    let current = textureSample(t_scene, s_scene, in.tex_coord);
    let d_vis = .5 + .5 * cos(d * 0.01 * 3.14159265358979323846);

    let previous = textureSample(t_previous, s_previous, in.tex_coord);
    // let smear = mix(current, previous, params.smear_factor * d_vis);

    let screen_xy = (in.tex_coord.xy-vec2(.5))*vec2(params.resolution.x/params.resolution.y,1);
    // if(length(screen_xy)>.25) {
    //     return vec4(cos(world_position.xyz),1);
    // }

    // Apply mask tint
    let tinted = mix(current.rgb, params.mask_tint, params.mask_tint_strength);

    return vec4<f32>(tinted, current.a);
}
