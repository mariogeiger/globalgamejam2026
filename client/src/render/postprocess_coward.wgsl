// Coward mask: Desaturated with velocity-based motion blur for speed enhancement

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
    let pixel_coord = vec2<i32>(in.uv * params.resolution);
    
    // Sample G-buffer
    let position = textureLoad(t_position, pixel_coord, 0);
    let velocity = textureLoad(t_velocity, pixel_coord, 0);
    
    var blurred = vec4<f32>(0.0);
    var total_weight = 0.0;
    
    // Sample along the motion direction with gaussian-like weighting
    const SAMPLES = 7;
    for (var i = -SAMPLES; i <= SAMPLES; i++) {
        let t = f32(i) / f32(SAMPLES); // -1 to 1
        let weight = 1.;
        let offset = velocity.xy*t;
        blurred += textureSample(t_scene, s_scene, in.uv + offset) * weight;
        total_weight += weight;
    }
    blurred /= total_weight;

    let col_lum = dot(blurred.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let col_desaturated = mix(blurred.rgb, vec3<f32>(col_lum), 0.4);
    let col_brightened = col_desaturated * 1.1 + vec3<f32>(0.05);

    return vec4(vec3(1.-exp(.5-5.*col_lum)),1.);
}
