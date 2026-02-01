// Coward mask: Desaturated with velocity-based motion blur for speed enhancement

@group(0) @binding(0) var t_scene: texture_2d<f32>;
@group(0) @binding(1) var s_scene: sampler;
@group(0) @binding(2) var t_position: texture_2d<f32>;
@group(0) @binding(3) var t_velocity: texture_2d<f32>;

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
    let pixel_coord = vec2<i32>(in.uv * params.resolution);
    
    // Sample G-buffer
    let position = textureLoad(t_position, pixel_coord, 0);
    let velocity = textureLoad(t_velocity, pixel_coord, 0);
    
    let view_pos = position.xyz;
    let depth = max(position.w, 1.0); // Avoid division by zero
    let vel = velocity.xyz;
    
    // Project 3D velocity to screen-space velocity
    // For perspective projection: screen_pos = view_pos.xy / view_pos.z
    // Derivative: screen_vel = (vel.xy * z - view_pos.xy * vel.z) / z²
    // This gives us: lateral motion / depth - radial expansion from forward motion
    let screen_vel = (vel.xy - view_pos.xy * vel.z / depth) / depth;
    
    // Convert to UV-space blur direction (flip Y for screen coords)
    let blur_dir = vec2<f32>(screen_vel.x, -screen_vel.y);
    
    // Screen-space speed determines blur amount
    let screen_speed = length(screen_vel);
    let speed_factor = clamp(screen_speed / 2.0, 0.0, 1.0);
    
    // Directional motion blur: elongated kernel along velocity direction
    // More samples for smoother blur, weighted by distance from center
    let blur_length = speed_factor * 0.08;
    let blur_axis = normalize(blur_dir + vec2<f32>(0.0001)); // Normalized direction
    
    var blurred = vec4<f32>(0.0);
    var total_weight = 0.0;
    
    // Sample along the motion direction with gaussian-like weighting
    for (var i = -7; i <= 7; i++) {
        let t = f32(i) / 7.0; // -1 to 1
        let weight = exp(-2.0 * t * t); // Gaussian falloff
        let offset = blur_axis * blur_length * t;
        blurred += textureSample(t_scene, s_scene, in.uv + offset) * weight;
        total_weight += weight;
    }
    blurred /= total_weight;
    
    // Blend: full blur when moving fast, sharp when still
    let color = mix(textureSample(t_scene, s_scene, in.uv), blurred, speed_factor);
    
    // Desaturate and brighten (coward's washed-out look)
    let luminance = dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let desaturated = mix(color.rgb, vec3<f32>(luminance), 0.4);
    let brightened = desaturated * 1.1 + vec3<f32>(0.05);
    
    return vec4<f32>(brightened, color.a);
}
