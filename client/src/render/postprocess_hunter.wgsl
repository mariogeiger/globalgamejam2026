// Hunter mask: Heat distortion, red tint, aggressive vignette

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
    let t = params.time;
    var uv = in.uv;
    
    // Heat distortion - multiple wave frequencies for organic flame look
    let distort_x = sin(uv.y * 25.0 + t * 4.0) * 0.004
                  + sin(uv.y * 50.0 + t * 6.0) * 0.002
                  + sin(uv.y * 12.0 + t * 2.5) * 0.003;
    let distort_y = cos(uv.x * 30.0 + t * 3.5) * 0.003
                  + cos(uv.x * 15.0 + t * 2.0) * 0.002;
    uv = uv + vec2<f32>(distort_x, distort_y);
    
    let color = textureSample(t_scene, s_scene, uv);
    
    // Red tint with contrast boost
    let tinted = color.rgb * vec3<f32>(1.2, 0.85, 0.85);
    
    // Vignette - darker edges with red tint
    let dist = length(in.uv - vec2<f32>(0.5)) * 1.4;
    let vignette = smoothstep(0.2, 1.2, dist);
    let vignetted = mix(tinted, vec3<f32>(0.8, 0.1, 0.0), vignette * 0.3);
    
    return vec4<f32>(vignetted, color.a);
}
