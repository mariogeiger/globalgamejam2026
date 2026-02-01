struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct PlayerUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> player: PlayerUniform;

@group(2) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(2) @binding(1)
var s_diffuse: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = player.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.tex_coord = in.tex_coord;
    out.world_normal = (player.model * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coord);
    
    let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.3));
    let normal = normalize(in.world_normal);
    let diffuse = max(dot(normal, light_dir), 0.0);
    let ambient = 0.3;
    let brightness = ambient + diffuse * 0.7;
    
    // Multiply texture by color (allows tinting or using solid color with white texture)
    let final_color = tex_color.rgb * player.color.rgb * brightness;
    
    return vec4<f32>(final_color, tex_color.a * player.color.a);
}
