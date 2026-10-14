// Draws one sheet of the marble's veins into its cached image, for only the
// rows that need it (marble.rs works out which). Each sheet is a warped web
// of thin lines; parts are left out so it isn't a closed net.
//
// The cache holds the sheet's world rows O .. O + rows - 1, row R stored at
// image row R mod rows, so scrolling only needs the rows that come into view.

struct Fill {
    // image width and rows, the screen height the web is scaled to, and how
    // far the viewport's middle is from that screen's middle (pixels)
    a: vec4<f32>,
    // sheet number (0 = top), its scale, vein thickness, how much of the web is drawn
    b: vec4<f32>,
    // first world row the image holds, the viewport's height in pixels
    c: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Fill;
// 64x64 random bytes shared with marble.rs: r, g = cell point, b = noise, a = per-vein value.
@group(0) @binding(1) var pts: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole image.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn texel(cell: vec2<f32>) -> vec4<f32> {
    let c = vec2<i32>(floor(cell));
    let w = ((c % vec2<i32>(64)) + vec2<i32>(64)) % vec2<i32>(64);
    return textureLoad(pts, w, 0);
}

fn cell_pt(cell: vec2<f32>, so: vec2<f32>) -> vec2<f32> {
    return cell + 0.1 + 0.8 * texel(cell + so).rg;
}

// Smooth noise from the shared bytes. (A sin()-based hash gives slightly
// different values for the same spot depending on how the GPU computed it,
// which tears the warp into blocks; this way marble.rs can also compute it.)
fn tnoise(x: vec2<f32>, so: vec2<f32>) -> f32 {
    let i = floor(x);
    let f = x - i;
    let t = f * f * (3.0 - 2.0 * f);
    return mix(mix(texel(i + so).b, texel(i + vec2<f32>(1.0, 0.0) + so).b, t.x),
               mix(texel(i + vec2<f32>(0.0, 1.0) + so).b, texel(i + vec2<f32>(1.0, 1.0) + so).b, t.x), t.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < 4; i++) {
        v += a * tnoise(p, vec2<f32>(5.0, 41.0));
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

// The web's own coordinates: scaled, then warped so the cells come out organic.
fn warp(s: vec2<f32>, scale: f32) -> vec2<f32> {
    return s * scale + 1.1 * vec2<f32>(fbm(s * 1.2), fbm(s * 1.2 + 5.2));
}

struct Border {
    // distance to the nearest vein, and to the next nearest (small near a junction)
    d1: f32,
    d2: f32,
    // a random 0-1 value for the nearest vein, the same all along it
    pick: f32,
};

// Distance to the nearest cell border, after Inigo Quilez.
fn border(q: vec2<f32>, so: vec2<f32>) -> Border {
    let n = floor(q);
    let f = q - n;
    var md = 8.0;
    var mr = vec2<f32>(0.0);
    var mg = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let g = vec2<f32>(f32(x), f32(y));
            let r = cell_pt(n + g, so) - n - f;
            let d = dot(r, r);
            if (d < md) { md = d; mr = r; mg = g; }
        }
    }
    var d1 = 8.0;
    var d2 = 8.0;
    var other = vec2<f32>(0.0);
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let g = mg + vec2<f32>(f32(x), f32(y));
            let r = cell_pt(n + g, so) - n - f;
            if (dot(mr - r, mr - r) > 0.00001) {
                let d = dot(0.5 * (mr + r), normalize(r - mr));
                if (d < d1) { d2 = d1; d1 = d; other = n + g; } else if (d < d2) { d2 = d; }
            }
        }
    }
    return Border(d1, d2, 0.5 * (texel(n + mg + so).a + texel(other + so).a));
}

// r = how much of a vein this pixel is, g = that plus the vein's soft glow.
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let rows = i32(u.a.y);
    let height = u.a.z;
    let first = i32(u.c.x);
    // The world row this image row holds.
    let image_row = i32(floor(pos.y));
    let row = first + (((image_row - first) % rows) + rows) % rows;
    let w = vec2<f32>(pos.x / height, (u.c.y - (f32(row) + 0.5) + u.a.w) / height);

    let fk = u.b.x;
    let scale = u.b.y;
    let seed = fk + 1.0;
    let so = vec2<f32>(seed * 17.0, seed * 29.0);
    let s = w + seed * 13.7;
    let q = warp(s, scale);
    let b = border(q, so);
    // The distance in screen pixels, measured by stepping one pixel through
    // the warp, so every vein keeps an even width however the warp stretches.
    let one_px = 1.0 / height;
    let q_per_px = 0.5 * (length(warp(s + vec2<f32>(one_px, 0.0), scale) - q) + length(warp(s + vec2<f32>(0.0, one_px), scale) - q));
    let px = b.d1 / q_per_px;
    let fill = u.b.w;
    let keep = smoothstep(1.0 - fill - 0.12, 1.0 - fill + 0.12, tnoise(q * 0.45 + seed * 3.0, so));
    // Half-width in pixels: one even width per vein, some a little finer,
    // thinner on deeper sheets.
    var thick = u.b.z * 3.0 * (height / 1000.0) * (0.75 + 0.5 * b.pick) * pow(0.84, fk);
    // At a junction the main vein carries through and finer branches thin
    // as they join it.
    if (b.pick < 0.5) {
        thick *= mix(0.4, 1.0, smoothstep(0.0, 0.25, b.d2));
    }
    // Veins narrow to a point where they end, like cracks.
    thick = max(thick * keep, 0.6);
    let edge_soft = min(0.8 + fk * 0.6, 3.0);
    let keep_mask = smoothstep(0.0, 0.08, keep);
    let web = (1.0 - smoothstep(thick - edge_soft, thick + edge_soft, px)) * keep_mask;
    let halo_width = thick * 2.2 + 2.0;
    let halo = exp(-px * px / (2.0 * halo_width * halo_width)) * 0.35 * keep_mask;
    return vec4<f32>(web, max(web, halo), 0.0, 1.0);
}
