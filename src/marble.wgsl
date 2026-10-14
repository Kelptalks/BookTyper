// The marble background, drawn every frame from the cached sheets of veins
// (marble_fill.wgsl draws those once, then just the rows that scroll into
// view). Stacks them deepest first, deeper ones dimmer, and adds the lamp
// light, the sparks walking the veins and the chapter-end wave.

struct Uniforms {
    // viewport width and height in pixels, time in seconds, number of sheets
    a: vec4<f32>,
    // lamp (0 or 1), where the line you type is (fraction from the bottom),
    // the screen height the web is scaled to, and the viewport's offset from that screen's middle
    b: vec4<f32>,
    // how far each sheet has scrolled, in pixels, top sheet first (up to 12)
    off: array<vec4<f32>, 3>,
    // background color, number of spark points
    bg: vec4<f32>,
    // plain vein color, and the vein setting: 0 plain, 1 accent, 2 accent fading to plain
    gray: vec4<f32>,
    // accent vein color, and spark strength
    accent: vec4<f32>,
    // spark light color, number of junction flashes
    spark: vec4<f32>,
    // chapter-end wave: center x, y (pixels from the viewport's top-left), radius in pixels, strength
    wave: vec4<f32>,
    // viewport's top-left in the window (pixels), rows in each cached sheet
    origin: vec4<f32>,
    // spark light points: x, y in the sheet's world units, brightness, sheet
    sparks: array<vec4<f32>, 72>,
    // junctions a spark just went through: x, y, brightness, sheet
    nodes: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// The cached sheets: r = vein, g = vein plus its soft glow.
@group(0) @binding(1) var sheets: texture_2d_array<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole viewport.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// A sheet's (vein, vein with glow) at this pixel: its world row is the
// screen row plus how far it has scrolled, blended between the two nearest
// cached rows so it slides smoothly.
fn sample_sheet(x: i32, y: f32, k: i32, rows: i32) -> vec2<f32> {
    let world = y - 0.5 + u.off[k / 4][k % 4];
    let r0 = i32(floor(world));
    let t = world - f32(r0);
    let a = textureLoad(sheets, vec2<i32>(x, ((r0 % rows) + rows) % rows), k, 0).rg;
    let b = textureLoad(sheets, vec2<i32>(x, (((r0 + 1) % rows) + rows) % rows), k, 0).rg;
    return mix(a, b, t);
}

// Where a point in a sheet's world units is on screen, in viewport pixels.
fn on_screen(w: vec2<f32>, k: i32) -> vec2<f32> {
    let height = u.b.z;
    return vec2<f32>(w.x * height, u.a.y + u.b.w - w.y * height - u.off[k / 4][k % 4]);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let local = pos.xy - u.origin.xy;
    let size = u.a.xy;
    let height = u.b.z;
    let rows = i32(u.origin.z);
    let x = i32(floor(local.x));
    let bg = u.bg.rgb;
    var col = bg;
    // Lamp: veins near the line you're typing catch more light.
    let up = (size.y - local.y + u.b.w) / height;
    let dy = up - u.b.y;
    let lamp = mix(1.0, 0.55 + 0.9 * exp(-dy * dy / 0.02), u.b.x);
    // The chapter-end wave: a ring spreading out through the veins.
    let ring_d = length(local - u.wave.xy) - u.wave.z;
    let ring = exp(-ring_d * ring_d / 4900.0) * u.wave.w;
    // Spark light on the top three sheets, sized to the screen.
    var glow = array<f32, 3>(0.0, 0.0, 0.0);
    let px_scale = height / 1000.0;
    let spark_width = 98.0 * px_scale * px_scale;
    let node_width = 162.0 * px_scale * px_scale;
    for (var i = 0; i < i32(u.bg.w); i++) {
        let sp = u.sparks[i];
        let k = i32(sp.w);
        let d = local - on_screen(sp.xy, k);
        glow[k] += sp.z * exp(-dot(d, d) / spark_width);
    }
    for (var i = 0; i < i32(u.spark.w); i++) {
        let nd = u.nodes[i];
        let k = i32(nd.w);
        let d = local - on_screen(nd.xy, k);
        glow[k] += nd.z * exp(-dot(d, d) / node_width);
    }
    let layers = u.a.w;
    // Deepest sheet first.
    for (var k = 9; k >= 0; k--) {
        let fk = f32(k);
        if (fk >= layers) { continue; }
        let v = sample_sheet(x, local.y, k, rows);
        let web = v.r;
        let alpha = v.g;
        // Vein setting 2: accent on top, a step closer to plain each sheet down.
        var g = u.gray.w;
        if (g > 1.5) { g = 1.0 - fk / max(layers - 1.0, 1.0); }
        let vein = mix(u.gray.rgb, u.accent.rgb, g);
        // Each sheet down is a little dimmer and more see-through, so a deep
        // stack fades off gradually.
        let dim = pow(0.78, fk) * lamp;
        let shade = mix(bg, vein, clamp((0.75 + 0.45 * web) * dim, 0.0, 1.3));
        col = mix(col, shade, alpha * pow(0.9, fk));
        var light = 0.0;
        if (k < 3) {
            light = glow[k] * (web + 0.6 * alpha) * u.accent.w * 0.16;
        }
        let lit = (light + ring * (web * 0.9 + alpha * 0.35) * 0.5) * pow(0.75, fk);
        let spark_color = mix(vec3<f32>(0.75, 0.82, 0.95), u.spark.rgb, g);
        col = mix(col, spark_color, clamp(lit, 0.0, 1.0));
    }
    // Dither so smooth fades don't band.
    col += (hash(pos.xy + fract(u.a.z)) - 0.5) / 255.0;
    return vec4<f32>(col, 1.0);
}
