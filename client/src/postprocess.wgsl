// Fullscreen post-processing: scene + previous frame → history (motion blur + smear).
// Output is written to a history texture; a separate present pass blits to swapchain.

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

struct PostProcessParams {
    blur_direction: vec2<f32>,
    blur_strength: f32,
    smear_factor: f32,
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

const MOTION_BLUR_SAMPLES: i32 = 9;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = params.blur_direction;
    let strength = params.blur_strength;

    var current: vec4<f32>;
    if strength <= 0.0 {
        current = textureSample(t_scene, s_scene, in.tex_coord);
    } else {
        var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        for (var i = 0; i < MOTION_BLUR_SAMPLES; i++) {
            let t = (f32(i) / f32(MOTION_BLUR_SAMPLES - 1)) - 0.5;
            let offset = dir * strength * t;
            color += textureSample(t_scene, s_scene, in.tex_coord + offset);
        }
        current = color / f32(MOTION_BLUR_SAMPLES);
    }

    let previous = textureSample(t_previous, s_previous, in.tex_coord);
    // Smear: blend current with previous (high smear_factor = stronger trails).
    return mix(current, previous, params.smear_factor);
}
