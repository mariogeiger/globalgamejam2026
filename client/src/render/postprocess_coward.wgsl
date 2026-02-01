// Coward mask: Desaturated, washed-out fearful look

@group(0) @binding(0) var t_scene: texture_2d<f32>;
@group(0) @binding(1) var s_scene: sampler;

@group(1) @binding(0) var<uniform> params: Params;

struct Params {
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
    let color = textureSample(t_scene, s_scene, in.uv);
    
    // Desaturate and brighten
    let luminance = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let desaturated = mix(color.rgb, vec3<f32>(luminance), 0.4);
    let brightened = desaturated * 1.1 + vec3<f32>(0.05);
    
    return vec4<f32>(brightened, color.a);
}
