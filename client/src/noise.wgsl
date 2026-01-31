// Procedural noise for WGSL (no textures).
// Include this before shaders that need noise (e.g. postprocess).
//
// Summary:
//   xy → f32:    noise2d(p), noise2d_fbm(p, octaves), hash21(p)
//   xy → vec2:   hash22(p)
//   xyz → f32:   noise3d(p), hash31(p)
//   xyz → xyz:   noise3d_vec3(p)   — ℝ³→ℝ³, result in [0,1]³
//   xyzt → xyz:  noise4d_vec3(p)   — ℝ⁴→ℝ³ (p.w = time), result in [0,1]³
//   xyzt → f32:  noise4d(p), hash41(p)

// Hash from 2D cell → pseudo-random vec2 in [0, 1]
fn hash22(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

// Hash from 2D cell → single value in [0, 1]
fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// 1D hash
fn hash11(p: f32) -> f32 {
    return fract(sin(p) * 43758.5453);
}

// Smoothstep-style quintic for C2 interpolation
fn quintic(t: vec2<f32>) -> vec2<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// 2D value noise, result in [0, 1]
fn noise2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = quintic(f);

    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 2D value noise with derivative (noise, ddx, ddy)
fn noise2d_deriv(p: vec2<f32>) -> vec3<f32> {
    let eps = vec2<f32>(0.001, 0.0);
    let n = noise2d(p);
    let dx = noise2d(p + eps.xy) - noise2d(p - eps.xy);
    let dy = noise2d(p + eps.yx) - noise2d(p - eps.yx);
    return vec3<f32>(n, dx, dy);
}

// Fractal Brownian motion: sum of octaves of noise (good for terrain/film grain)
fn noise2d_fbm(p: vec2<f32>, octaves: i32) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var frequency = 1.0;
    var max_value = 0.0;
    for (var i = 0; i < octaves; i++) {
        value += amplitude * noise2d(p * frequency);
        max_value += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    return value / max_value;
}

// --- 3D noise (ℝ³ → ℝ³) ---

// Hash from 3D cell → pseudo-random vec3 in [0, 1]
fn hash33(p: vec3<f32>) -> vec3<f32> {
    let q = vec3<f32>(
        dot(p, vec3<f32>(127.1, 311.7, 74.7)),
        dot(p, vec3<f32>(269.5, 183.3, 246.1)),
        dot(p, vec3<f32>(113.5, 271.9, 124.6))
    );
    return fract(sin(q) * 43758.5453);
}

// Hash from 3D cell → single value in [0, 1]
fn hash31(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

// Quintic interpolation for 3D
fn quintic3(t: vec3<f32>) -> vec3<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// 3D value noise, scalar result in [0, 1]
fn noise3d(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = quintic3(f);

    let n000 = hash31(i);
    let n100 = hash31(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash31(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash31(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash31(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash31(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash31(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash31(i + vec3<f32>(1.0, 1.0, 1.0));

    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);
    let nxy0 = mix(nx00, nx10, u.y);
    let nxy1 = mix(nx01, nx11, u.y);
    return mix(nxy0, nxy1, u.z);
}

// 3D vector noise: ℝ³ → ℝ³ (trilinear interpolation of hash33 at cell corners)
// Result components in [0, 1]; use (v - 0.5) * 2.0 for range [-1, 1]
fn noise3d_vec3(p: vec3<f32>) -> vec3<f32> {
    let i = floor(p);
    let f = fract(p);
    let u = quintic3(f);

    let n000 = hash33(i);
    let n100 = hash33(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash33(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash33(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash33(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash33(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash33(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash33(i + vec3<f32>(1.0, 1.0, 1.0));

    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);
    let nxy0 = mix(nx00, nx10, u.y);
    let nxy1 = mix(nx01, nx11, u.y);
    return mix(nxy0, nxy1, u.z);
}

// --- 4D noise (xyzt → xyz and xyzt → f32) ---

// Hash from 4D cell → pseudo-random vec4 in [0, 1]
fn hash44(p: vec4<f32>) -> vec4<f32> {
    let q = vec4<f32>(
        dot(p, vec4<f32>(127.1, 311.7, 74.7, 173.3)),
        dot(p, vec4<f32>(269.5, 183.3, 246.1, 97.1)),
        dot(p, vec4<f32>(113.5, 271.9, 124.6, 221.7)),
        dot(p, vec4<f32>(37.9, 157.2, 283.4, 199.1))
    );
    return fract(sin(q) * 43758.5453);
}

// Hash from 4D cell → single value in [0, 1]
fn hash41(p: vec4<f32>) -> f32 {
    return fract(sin(dot(p, vec4<f32>(127.1, 311.7, 74.7, 173.3))) * 43758.5453);
}

// Quintic interpolation for 4D
fn quintic4(t: vec4<f32>) -> vec4<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// 4D value noise, scalar result in [0, 1]
fn noise4d(p: vec4<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = quintic4(f);

    let n0000 = hash41(i);
    let n1000 = hash41(i + vec4<f32>(1.0, 0.0, 0.0, 0.0));
    let n0100 = hash41(i + vec4<f32>(0.0, 1.0, 0.0, 0.0));
    let n1100 = hash41(i + vec4<f32>(1.0, 1.0, 0.0, 0.0));
    let n0010 = hash41(i + vec4<f32>(0.0, 0.0, 1.0, 0.0));
    let n1010 = hash41(i + vec4<f32>(1.0, 0.0, 1.0, 0.0));
    let n0110 = hash41(i + vec4<f32>(0.0, 1.0, 1.0, 0.0));
    let n1110 = hash41(i + vec4<f32>(1.0, 1.0, 1.0, 0.0));
    let n0001 = hash41(i + vec4<f32>(0.0, 0.0, 0.0, 1.0));
    let n1001 = hash41(i + vec4<f32>(1.0, 0.0, 0.0, 1.0));
    let n0101 = hash41(i + vec4<f32>(0.0, 1.0, 0.0, 1.0));
    let n1101 = hash41(i + vec4<f32>(1.0, 1.0, 0.0, 1.0));
    let n0011 = hash41(i + vec4<f32>(0.0, 0.0, 1.0, 1.0));
    let n1011 = hash41(i + vec4<f32>(1.0, 0.0, 1.0, 1.0));
    let n0111 = hash41(i + vec4<f32>(0.0, 1.0, 1.0, 1.0));
    let n1111 = hash41(i + vec4<f32>(1.0, 1.0, 1.0, 1.0));

    let nx000 = mix(n0000, n1000, u.x);
    let nx100 = mix(n0100, n1100, u.x);
    let nx010 = mix(n0010, n1010, u.x);
    let nx110 = mix(n0110, n1110, u.x);
    let nx001 = mix(n0001, n1001, u.x);
    let nx101 = mix(n0101, n1101, u.x);
    let nx011 = mix(n0011, n1011, u.x);
    let nx111 = mix(n0111, n1111, u.x);
    let nxy00 = mix(nx000, nx100, u.y);
    let nxy10 = mix(nx010, nx110, u.y);
    let nxy01 = mix(nx001, nx101, u.y);
    let nxy11 = mix(nx011, nx111, u.y);
    let nxyz0 = mix(nxy00, nxy10, u.z);
    let nxyz1 = mix(nxy01, nxy11, u.z);
    return mix(nxyz0, nxyz1, u.w);
}

// 4D → 3D vector noise: xyzt → xyz (e.g. xy = space, z = layer, w = time)
// Three decorrelated 4D scalar noises; result in [0, 1]³
fn noise4d_vec3(p: vec4<f32>) -> vec3<f32> {
    let o1 = vec4<f32>(17.0, 31.0, 47.0, 71.0);
    let o2 = vec4<f32>(113.0, 137.0, 163.0, 191.0);
    return vec3<f32>(noise4d(p), noise4d(p + o1), noise4d(p + o2));
}
