
fn fsr_lanczos2(x: f32) -> f32 {
    let ax = abs(x);
    if (ax >= 2.0) { return 0.0; }
    if (ax < 1e-6) { return 1.0; }
    let p = 3.14159265 * ax;
    return (sin(p) / p) * (sin(p * 0.5) / (p * 0.5));
}
fn fsr_rcas_clamp(c: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> vec3<f32> {
    return clamp(c, lo, hi);
}
