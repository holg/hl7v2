// Windowing and the view transform live here. The CPU uploads the frame once;
// every slider move, zoom, pan, rotate or flip only rewrites the 48-byte
// uniform below.

struct Uniforms {
    center: f32,
    width:  f32,
    invert: u32,
    interp: u32,
    // View: canvas (framebuffer) pixels per display pixel, and the canvas
    // position of the displayed image's top-left corner.
    scale:  f32,
    tx:     f32,
    ty:     f32,
    // 1 when the texture is RGBA colour: shown as stored, no windowing.
    color:  u32,
    // Quarter turns clockwise (0..3) and flip bits (1 = horizontal, 2 = vertical),
    // applied as in view.rs: flip in source space, then rotate.
    rot:    u32,
    flip:   u32,
    _pad0:  u32,
    _pad1:  u32,
};

@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: Uniforms;

// Full-screen triangle, no vertex buffer: (-1,-1), (3,-1), (-1,3).
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn load(p: vec2<i32>, maxi: vec2<i32>) -> vec4<f32> {
    return textureLoad(img, clamp(p, vec2<i32>(0), maxi), 0);
}

// Display coordinates (after flip and rotation) back to source pixels.
// Mirrors `Viewport::display_to_source` in view.rs.
fn display_to_source(d: vec2<f32>, src: vec2<f32>) -> vec2<f32> {
    var p: vec2<f32>;
    switch u.rot {
        case 1u: { p = vec2<f32>(d.y, src.y - d.x); }
        case 2u: { p = vec2<f32>(src.x - d.x, src.y - d.y); }
        case 3u: { p = vec2<f32>(src.x - d.y, d.x); }
        default: { p = d; }
    }
    if ((u.flip & 1u) != 0u) { p.x = src.x - p.x; }
    if ((u.flip & 2u) != 0u) { p.y = src.y - p.y; }
    return p;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let src = vec2<f32>(textureDimensions(img));
    var disp = src;
    if (u.rot == 1u || u.rot == 3u) { disp = vec2<f32>(src.y, src.x); }

    // Framebuffer coordinates have y down and row 0 at the top, like DICOM.
    let d = (pos.xy - vec2<f32>(u.tx, u.ty)) / u.scale;
    if (d.x < 0.0 || d.y < 0.0 || d.x >= disp.x || d.y >= disp.y) {
        return vec4<f32>(0.06, 0.06, 0.06, 1.0);
    }
    let q = clamp(display_to_source(d, src), vec2<f32>(0.0), src - vec2<f32>(1e-3));

    var s: vec4<f32>;
    if (u.interp == 1u && u.scale > 1.0) {
        // Magnified: bilinear over four texels, done by hand because the
        // R32Float format is not filterable without an optional feature.
        let maxi = vec2<i32>(src) - vec2<i32>(1);
        let c = q - vec2<f32>(0.5);
        let f = floor(c);
        let t = c - f;
        let i0 = vec2<i32>(f);
        let a = load(i0, maxi);
        let b = load(i0 + vec2<i32>(1, 0), maxi);
        let cc = load(i0 + vec2<i32>(0, 1), maxi);
        let dd = load(i0 + vec2<i32>(1, 1), maxi);
        s = mix(mix(a, b, t.x), mix(cc, dd, t.x), t.y);
    } else {
        s = textureLoad(img, vec2<i32>(q), 0);
    }
    if (u.color == 1u) {
        return vec4<f32>(s.rgb, 1.0);
    }
    let v = s.r;

    // DICOM PS3.3 C.11.2.1.2.1, linear VOI LUT function:
    //   y = ((x - (c - 0.5)) / (w - 1) + 0.5), clamped to [0, 1].
    let w = max(u.width, 1.0);
    var g = clamp((v - (u.center - 0.5)) / max(w - 1.0, 1e-3) + 0.5, 0.0, 1.0);
    if (u.invert == 1u) {
        g = 1.0 - g;
    }
    return vec4<f32>(g, g, g, 1.0);
}
