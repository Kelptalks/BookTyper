//! The marble background: sheets of gold and gray veins behind the text that
//! scroll as you type, deeper sheets slower, with sparks of light walking
//! along the veins. The veins are drawn by a wgpu shader (marble.wgsl). The
//! sparks are moved here, on a copy of the same web: both sides build it from
//! one table of random bytes, so the sparks stay on the veins you see.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use eframe::egui::{self, Rect};
use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;

use crate::theme::{MarbleSettings, Theme, Veins};

/// Side of the random byte table, in cells. The web repeats after this many.
const T: i32 = 64;
const SPARK_SLOTS: usize = 72;
const NODE_SLOTS: usize = 16;
/// The composite shader's uniforms, in vec4s: 11 settings, then sparks,
/// then junction flashes; and the fill shader's.
const COMPOSITE_VEC4S: usize = 11 + SPARK_SLOTS + NODE_SLOTS;
const FILL_VEC4S: usize = 3;
/// Most sheets of veins (the shader's limit).
const SHEETS_MAX: usize = crate::theme::MAX_LAYERS;
/// Sheets that carry sparks (the top ones).
const SPARK_SHEETS: usize = 3;
/// How much of the web is drawn (the rest is left out so it isn't a closed net).
const FILL: f32 = 0.7;
/// How far the top sheet scrolls per letter typed, in screen heights, at
/// 100%: about a fifth of the text's own speed.
pub const WEB_PER_LETTER: f32 = 0.2 * 44.0 / 65.0 / 900.0;
/// Seconds the scrolling takes to catch up, so single letters don't tick.
const SCROLL_EASE: f32 = 0.35;
/// Sparks on each sheet, top first, and their speed in web units a second.
const SPARKS_PER_SHEET: [usize; SPARK_SHEETS] = [4, 3, 3];
const SPARK_SPEED: f32 = 0.45;
/// A spark lives this long (seconds, plus up to `SPARK_LIFE_VARIES` more) and
/// fades in and out over `SPARK_FADE`, so you rarely see one start or stop.
const SPARK_LIFE: f32 = 14.0;
const SPARK_LIFE_VARIES: f32 = 22.0;
const SPARK_FADE: f32 = 3.0;
/// Points trailing behind each spark: (how far back along its path, brightness).
const SPARK_TRAIL: [(f32, f32); 6] = [(0.0, 1.0), (0.012, 0.8), (0.026, 0.58), (0.044, 0.38), (0.066, 0.2), (0.092, 0.08)];
/// Seconds a junction glows after a spark goes through it.
const NODE_GLOW: f32 = 0.7;
/// The chapter-end wave: seconds it lasts, and how fast it spreads in points a second.
const WAVE_TIME: f32 = 2.4;
const WAVE_SPEED: f32 = 900.0;

/// The random byte table: r, g place each cell's point, b is noise for which
/// veins are drawn, a gives each vein its width.
fn web_bytes() -> Vec<u8> {
    let mut state: u32 = 0x2545_F491;
    (0..T * T * 4)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 255) as u8
        })
        .collect()
}

type Cell = (i32, i32);

/// A corner of a cell's outline; `tag` is the neighbor the side starting here borders.
#[derive(Clone)]
struct Corner {
    p: [f32; 2],
    tag: Option<Cell>,
}

struct Spark {
    sheet: usize,
    /// It runs along the border between cells `a` and `b`, from `from` to `to`.
    a: Cell,
    b: Cell,
    from: [f32; 2],
    to: [f32; 2],
    len: f32,
    pos: f32,
    /// Distance travelled in all, and recent (x, y, distance) for the trail.
    dist: f32,
    hist: VecDeque<(f32, f32, f32)>,
    age: f32,
    life: f32,
}

struct Node {
    sheet: usize,
    p: [f32; 2],
    born: f32,
    bright: f32,
}

/// One marble background's moving parts: how far it has scrolled, and its
/// sparks. The reading screen and the settings preview each have one.
pub struct Web {
    /// Which uniform buffer it draws with (each Web drawn in a frame needs its own).
    slot: usize,
    bytes: Vec<u8>,
    polys: HashMap<(usize, i32, i32), Vec<Corner>>,
    sparks: Vec<Spark>,
    nodes: Vec<Node>,
    rng: u32,
    time: f32,
    last: Instant,
    /// How far the top sheet has scrolled at 100% speed, in screen heights (eased).
    scrolled: f32,
    wave_start: Option<f32>,
    /// Times a spark found no vein to carry on into (should stay 0).
    dead_ends: usize,
}

impl Web {
    pub fn new(slot: usize) -> Web {
        Web {
            slot,
            bytes: web_bytes(),
            polys: HashMap::new(),
            sparks: Vec::new(),
            nodes: Vec::new(),
            rng: 0x9E37_79B9 ^ slot as u32,
            time: 0.0,
            last: Instant::now(),
            scrolled: 0.0,
            wave_start: None,
            dead_ends: 0,
        }
    }

    /// Starts the chapter-end wave: light spreading out through the veins.
    pub fn start_wave(&mut self) {
        self.wave_start = Some(self.time);
    }

    /// Moves everything on by the time since the last call. `goal` is where
    /// the scrolling is headed (letters typed times `WEB_PER_LETTER`); a
    /// big jump (another chapter) is taken at once rather than swooshed.
    /// `aspect` is the drawn area's width over the screen height the web is
    /// scaled to, and `rows` the part of that height it covers (0-1, from the bottom).
    pub fn step(&mut self, goal: f32, aspect: f32, rows: (f32, f32), settings: &MarbleSettings) {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        self.time += dt;
        if (goal - self.scrolled).abs() > 0.5 {
            self.scrolled = goal;
        } else {
            self.scrolled += (goal - self.scrolled) * (1.0 - (-dt / SCROLL_EASE).exp());
        }
        if self.wave_start.is_some_and(|start| self.time - start > WAVE_TIME) {
            self.wave_start = None;
        }
        self.nodes.retain(|n| self.time - n.born < NODE_GLOW);
        if settings.sparks > 0.0 {
            self.move_sparks(dt, aspect, rows, settings);
        } else {
            self.sparks.clear();
        }
    }

    /// Still moving: keep redrawing.
    pub fn busy(&self, settings: &MarbleSettings) -> bool {
        settings.sparks > 0.0 || self.wave_start.is_some() || self.nodes.len() > 0
    }

    /// How far each sheet has scrolled: each one down slower by the "each
    /// layer slower" setting.
    fn offsets(&self, settings: &MarbleSettings) -> [f32; 12] {
        let top = self.scrolled * settings.scroll;
        std::array::from_fn(|k| top * settings.ratio.powi(k as i32))
    }

    fn scale(sheet: usize, settings: &MarbleSettings) -> f32 {
        2.6 / settings.size * (1.0 + 0.35 * sheet as f32)
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn at(x: i32, y: i32) -> usize {
        ((y.rem_euclid(T) * T + x.rem_euclid(T)) * 4) as usize
    }

    fn seed_offset(sheet: usize) -> (i32, i32) {
        ((sheet as i32 + 1) * 17, (sheet as i32 + 1) * 29)
    }

    fn cell_pt(&self, c: Cell, sheet: usize) -> [f32; 2] {
        let (sx, sy) = Self::seed_offset(sheet);
        let i = Self::at(c.0 + sx, c.1 + sy);
        [c.0 as f32 + 0.1 + 0.8 * self.bytes[i] as f32 / 255.0, c.1 as f32 + 0.1 + 0.8 * self.bytes[i + 1] as f32 / 255.0]
    }

    fn tnoise(&self, x: f32, y: f32, sheet: usize) -> f32 {
        self.tnoise_at(x, y, Self::seed_offset(sheet))
    }

    /// Smooth noise from the byte table, the same as the shader's `tnoise`.
    fn tnoise_at(&self, x: f32, y: f32, (sx, sy): (i32, i32)) -> f32 {
        let (ix, iy) = (x.floor(), y.floor());
        let (fx, fy) = (x - ix, y - iy);
        let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let v = |dx: i32, dy: i32| self.bytes[Self::at(ix as i32 + dx + sx, iy as i32 + dy + sy) + 2] as f32 / 255.0;
        let a = v(0, 0) + (v(1, 0) - v(0, 0)) * ux;
        let b = v(0, 1) + (v(1, 1) - v(0, 1)) * ux;
        a + (b - a) * uy
    }

    /// The shader's `fbm`: four octaves of smooth noise.
    fn fbm(&self, mut x: f32, mut y: f32) -> f32 {
        let (mut v, mut a) = (0.0, 0.5);
        for _ in 0..4 {
            v += a * self.tnoise_at(x, y, (5, 41));
            x = x * 2.03 + 1.7;
            y = y * 2.03 + 9.2;
            a *= 0.5;
        }
        v
    }

    /// The shader's `warp`: a sheet's world spot (plus its seed) into web units.
    fn warp(&self, s: [f32; 2], scale: f32) -> [f32; 2] {
        let (x, y) = (s[0] * 1.2, s[1] * 1.2);
        [s[0] * scale + 1.1 * self.fbm(x, y), s[1] * scale + 1.1 * self.fbm(x + 5.2, y + 5.2)]
    }

    /// Where on a sheet (in its world units) a spot in web units is: the warp
    /// run backwards, by Newton's method. None if it doesn't settle.
    fn unwarp(&self, q: [f32; 2], sheet: usize, settings: &MarbleSettings) -> Option<[f32; 2]> {
        let scale = Self::scale(sheet, settings);
        let seed = (sheet + 1) as f32 * 13.7;
        // The warp adds about 0.5 on average: start from there.
        let mut s = [(q[0] - 0.55) / scale, (q[1] - 0.55) / scale];
        const H: f32 = 1e-3;
        for _ in 0..12 {
            let f = self.warp(s, scale);
            let err = [f[0] - q[0], f[1] - q[1]];
            if err[0].abs() + err[1].abs() < 1e-5 {
                return Some([s[0] - seed, s[1] - seed]);
            }
            let fx = self.warp([s[0] + H, s[1]], scale);
            let fy = self.warp([s[0], s[1] + H], scale);
            let (a, c) = ((fx[0] - f[0]) / H, (fx[1] - f[1]) / H);
            let (b, d) = ((fy[0] - f[0]) / H, (fy[1] - f[1]) / H);
            let det = a * d - b * c;
            if det.abs() < 1e-6 {
                return None;
            }
            s[0] -= (d * err[0] - b * err[1]) / det;
            s[1] -= (a * err[1] - c * err[0]) / det;
        }
        let f = self.warp(s, scale);
        ((f[0] - q[0]).abs() + (f[1] - q[1]).abs() < 1e-3).then(|| [s[0] - seed, s[1] - seed])
    }

    /// How much this spot of the web is drawn (the shader's `keep`).
    fn keep(&self, p: [f32; 2], sheet: usize) -> f32 {
        let seed = (sheet + 1) as f32 * 3.0;
        smoothstep(1.0 - FILL - 0.12, 1.0 - FILL + 0.12, self.tnoise(p[0] * 0.45 + seed, p[1] * 0.45 + seed, sheet))
    }

    /// A cell's outline: a big square cut down by the halfway line to each
    /// neighbor. Each corner remembers which neighbor the next side borders.
    fn poly(&mut self, c: Cell, sheet: usize) -> Vec<Corner> {
        if let Some(poly) = self.polys.get(&(sheet, c.0, c.1)) {
            return poly.clone();
        }
        if self.polys.len() > 4000 {
            self.polys.clear();
        }
        let p = self.cell_pt(c, sheet);
        let mut poly: Vec<Corner> =
            [[-3.0, -3.0], [3.0, -3.0], [3.0, 3.0], [-3.0, 3.0]].iter().map(|d| Corner { p: [p[0] + d[0], p[1] + d[1]], tag: None }).collect();
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let q = self.cell_pt((c.0 + dx, c.1 + dy), sheet);
                let (nx, ny) = (q[0] - p[0], q[1] - p[1]);
                let k = (p[0] + q[0]) / 2.0 * nx + (p[1] + q[1]) / 2.0 * ny;
                let side = |v: [f32; 2]| v[0] * nx + v[1] * ny - k;
                let tag = Some((c.0 + dx, c.1 + dy));
                let mut out = Vec::with_capacity(poly.len() + 1);
                for i in 0..poly.len() {
                    let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
                    let (sa, sb) = (side(a.p), side(b.p));
                    let cut = || {
                        let t = sa / (sa - sb);
                        [a.p[0] + (b.p[0] - a.p[0]) * t, a.p[1] + (b.p[1] - a.p[1]) * t]
                    };
                    if sa <= 0.0 && sb <= 0.0 {
                        out.push(a.clone());
                    } else if sa <= 0.0 {
                        out.push(a.clone());
                        out.push(Corner { p: cut(), tag });
                    } else if sb <= 0.0 {
                        out.push(Corner { p: cut(), tag: a.tag });
                    }
                }
                poly = out;
            }
        }
        self.polys.insert((sheet, c.0, c.1), poly.clone());
        poly
    }

    /// The side of cell `a` that borders cell `b`, as its two corners.
    fn side(&mut self, a: Cell, b: Cell, sheet: usize) -> Option<([f32; 2], [f32; 2])> {
        let poly = self.poly(a, sheet);
        (0..poly.len()).find(|&i| poly[i].tag == Some(b)).map(|i| (poly[i].p, poly[(i + 1) % poly.len()].p))
    }

    /// Roughly the part of a sheet's web that's on screen, in web units.
    fn region(&self, sheet: usize, aspect: f32, rows: (f32, f32), settings: &MarbleSettings) -> [f32; 4] {
        let sc = Self::scale(sheet, settings);
        let seed = (sheet + 1) as f32 * 13.7;
        let off = self.offsets(settings)[sheet];
        [seed * sc + 0.2, (aspect + seed) * sc + 0.9, (rows.0 - off + seed) * sc + 0.2, (rows.1 - off + seed) * sc + 0.9]
    }

    fn spawn(&mut self, sheet: usize, region: [f32; 4]) -> Option<Spark> {
        for _ in 0..12 {
            let x = region[0] + self.random() * (region[1] - region[0]);
            let y = region[2] + self.random() * (region[3] - region[2]);
            // The cell whose point is nearest.
            let mut best = (f32::MAX, (0, 0));
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let c = (x.floor() as i32 + dx, y.floor() as i32 + dy);
                    let p = self.cell_pt(c, sheet);
                    let d = (p[0] - x).powi(2) + (p[1] - y).powi(2);
                    if d < best.0 {
                        best = (d, c);
                    }
                }
            }
            let a = best.1;
            let poly = self.poly(a, sheet);
            let i = (self.random() * poly.len() as f32) as usize % poly.len();
            let (c0, c1) = (poly[i].clone(), poly[(i + 1) % poly.len()].clone());
            let Some(b) = c0.tag else { continue };
            if self.keep(mid(c0.p, c1.p), sheet) < 0.55 {
                continue;
            }
            let (from, to) = if self.random() < 0.5 { (c0.p, c1.p) } else { (c1.p, c0.p) };
            let len = dist(from, to).max(1e-4);
            let life = SPARK_LIFE + self.random() * SPARK_LIFE_VARIES;
            let pos = self.random() * len;
            return Some(Spark { sheet, a, b, from, to, len, pos, dist: 0.0, hist: VecDeque::new(), age: 0.0, life });
        }
        None
    }

    /// At a junction: carry on into one of the two veins ahead, preferring
    /// drawn ones that keep roughly the same direction. Lights up the junction.
    fn turn(&mut self, i: usize) -> bool {
        let (sheet, a, b, v, from, len) = {
            let s = &self.sparks[i];
            (s.sheet, s.a, s.b, s.to, s.from, s.len)
        };
        let poly = self.poly(a, sheet);
        let Some(j) = poly.iter().position(|c| near(c.p, v)) else { return false };
        let (before, after) = (poly[(j + poly.len() - 1) % poly.len()].tag, poly[j].tag);
        let Some(c) = (if before == Some(b) { after } else { before }) else { return false };
        let dir = [(v[0] - from[0]) / len, (v[1] - from[1]) / len];
        let mut options = Vec::new();
        for (x, y) in [(a, c), (b, c)] {
            let Some((e0, e1)) = self.side(x, y, sheet) else { continue };
            let end = if near(e0, v) { e1 } else if near(e1, v) { e0 } else { continue };
            let l = dist(v, end).max(1e-4);
            let straight = ((end[0] - v[0]) * dir[0] + (end[1] - v[1]) * dir[1]) / l;
            let drawn = self.keep(mid(v, end), sheet) > 0.5;
            options.push((x, y, end, if drawn { 1.0 } else { 0.04 } * (1.2 + straight)));
        }
        if options.is_empty() {
            return false;
        }
        let mut pick = self.random() * options.iter().map(|o| o.3).sum::<f32>();
        let mut chosen = options[0];
        for o in &options {
            pick -= o.3;
            if pick <= 0.0 {
                chosen = *o;
                break;
            }
        }
        let spark = &mut self.sparks[i];
        let bright = fade(spark.age, spark.life);
        spark.a = chosen.0;
        spark.b = chosen.1;
        spark.from = v;
        spark.to = chosen.2;
        spark.len = dist(v, chosen.2).max(1e-4);
        self.nodes.push(Node { sheet, p: v, born: self.time, bright });
        if self.nodes.len() > NODE_SLOTS {
            self.nodes.remove(0);
        }
        true
    }

    fn move_sparks(&mut self, dt: f32, aspect: f32, rows: (f32, f32), settings: &MarbleSettings) {
        let sheets = SPARK_SHEETS.min(settings.layers);
        self.sparks.retain(|s| s.sheet < sheets);
        for sheet in 0..sheets {
            let have = self.sparks.iter().filter(|s| s.sheet == sheet).count();
            let region = self.region(sheet, aspect, rows, settings);
            for _ in have..SPARKS_PER_SHEET[sheet] {
                if let Some(spark) = self.spawn(sheet, region) {
                    self.sparks.push(spark);
                }
            }
        }
        let mut i = 0;
        while i < self.sparks.len() {
            self.sparks[i].age += dt;
            let mut step = SPARK_SPEED * dt;
            let mut ok = true;
            while ok && self.sparks[i].pos + step >= self.sparks[i].len {
                let s = &mut self.sparks[i];
                step -= s.len - s.pos;
                s.dist += s.len - s.pos;
                s.pos = 0.0;
                ok = self.turn(i);
                if !ok {
                    self.dead_ends += 1;
                }
            }
            let s = &mut self.sparks[i];
            s.pos += step;
            s.dist += step;
            let (x, y) = s.head();
            s.hist.push_back((x, y, s.dist));
            while s.hist.len() > 2 && s.hist[1].2 < s.dist - 0.15 {
                s.hist.pop_front();
            }
            let sheet = s.sheet;
            let r = self.region(sheet, aspect, rows, settings);
            let s = &self.sparks[i];
            let gone = !ok || s.age > s.life || x < r[0] - 1.5 || x > r[1] + 1.5 || y < r[2] - 1.5 || y > r[3] + 1.5;
            if gone {
                self.sparks.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Everything the GPU needs for one frame. Pixel sizes and positions are
    /// filled in by the callback, which knows the screen's pixels per point.
    fn frame(&self, rect: Rect, middle: f32, height: f32, theme: &Theme, settings: &MarbleSettings) -> Frame {
        let rgb = |c: egui::Color32| [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0];
        let bg = rgb(theme.background);
        let gray = rgb(theme.background.lerp_to_gamma(theme.text, 0.2));
        let accent = rgb(theme.background.lerp_to_gamma(theme.cursor, 0.36));
        let spark = rgb(theme.cursor.lerp_to_gamma(egui::Color32::WHITE, 0.25));
        let veins = match settings.veins {
            Veins::Plain => 0.0,
            Veins::Accent => 1.0,
            Veins::Fade => 2.0,
        };
        // Where the typing line is, as a fraction of the web's height from the bottom.
        let middle_up = (rect.bottom() - middle + (height - rect.height()) * 0.5) / height;
        let (wave_radius, wave_strength) = match self.wave_start {
            Some(start) => {
                let age = self.time - start;
                (age * WAVE_SPEED, (1.0 - age / WAVE_TIME).max(0.0).powf(1.5) * 1.2)
            }
            None => (0.0, 0.0),
        };

        // Sparks and junction flashes, as spots on their sheet's world.
        let mut sparks = Vec::with_capacity(SPARK_SLOTS * 4);
        for s in &self.sparks {
            let bright = fade(s.age, s.life);
            for (back, b) in SPARK_TRAIL {
                let (x, y) = if back == 0.0 { s.head() } else { s.back(back) };
                if sparks.len() < SPARK_SLOTS * 4
                    && let Some(w) = self.unwarp([x, y], s.sheet, settings)
                {
                    sparks.extend([w[0], w[1], b * bright, s.sheet as f32]);
                }
            }
        }
        let mut nodes = Vec::with_capacity(NODE_SLOTS * 4);
        for n in self.nodes.iter().take(NODE_SLOTS) {
            let age = self.time - n.born;
            // Quick to light, slower to fade.
            let glow = (age / 0.08).min(1.0) * (1.0 - age / NODE_GLOW).max(0.0).powi(2);
            if let Some(w) = self.unwarp(n.p, n.sheet, settings) {
                nodes.extend([w[0], w[1], glow * n.bright * 0.9, n.sheet as f32]);
            }
        }
        let (spark_count, node_count) = ((sparks.len() / 4) as f32, (nodes.len() / 4) as f32);

        let mut u = vec![
            // a: size (later), time, sheets
            0.0, 0.0, self.time, settings.layers as f32,
            // b: lamp, typing line, height and offset (later)
            if settings.lamp { 1.0 } else { 0.0 }, middle_up, 0.0, 0.0,
            // off (later)
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            bg[0], bg[1], bg[2], spark_count,
            gray[0], gray[1], gray[2], veins,
            accent[0], accent[1], accent[2], settings.sparks,
            spark[0], spark[1], spark[2], node_count,
            // wave, in points from the top-left (scaled later)
            rect.width() / 2.0, middle - rect.top(), wave_radius, wave_strength,
            // origin and cached rows (later)
            0.0, 0.0, 0.0, 0.0,
        ];
        u.extend(sparks);
        u.resize(u.len() + SPARK_SLOTS * 4 - spark_count as usize * 4, 0.0);
        u.extend(nodes);
        u.resize(COMPOSITE_VEC4S * 4, 0.0);
        Frame {
            uniforms: u,
            offsets: self.offsets(settings),
            layers: settings.layers.min(SHEETS_MAX),
            scale: Self::scale(0, settings),
            thickness: settings.thickness,
            height,
        }
    }

    /// The shape that draws this background into `rect`. `middle` is the y
    /// of the line you type; `height` is the screen height the web is scaled
    /// to (the rect's own height, except in a small preview).
    pub fn shape(&self, rect: Rect, middle: f32, height: f32, theme: &Theme, settings: &MarbleSettings) -> egui::Shape {
        let frame = self.frame(rect, middle, height, theme, settings);
        egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(rect, MarbleCallback { slot: self.slot, rect, frame }))
    }
}

/// One frame's worth of what the GPU needs.
struct Frame {
    /// The composite shader's uniforms (pixel values filled in later).
    uniforms: Vec<f32>,
    /// How far each sheet has scrolled, in screen heights.
    offsets: [f32; 12],
    layers: usize,
    /// The top sheet's scale (deeper sheets are smaller-celled).
    scale: f32,
    thickness: f32,
    /// The screen height the web is scaled to, in points.
    height: f32,
}

impl Spark {
    fn head(&self) -> (f32, f32) {
        let k = self.pos / self.len;
        (self.from[0] + (self.to[0] - self.from[0]) * k, self.from[1] + (self.to[1] - self.from[1]) * k)
    }

    /// Where it was `back` web units ago along its path.
    fn back(&self, back: f32) -> (f32, f32) {
        let target = self.dist - back;
        self.hist.iter().rev().find(|h| h.2 <= target).or(self.hist.front()).map_or(self.head(), |h| (h.0, h.1))
    }
}

fn fade(age: f32, life: f32) -> f32 {
    smoothstep(0.0, SPARK_FADE, age) * smoothstep(life, life - SPARK_FADE, age)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mid(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
}

/// The GPU side, made once at startup and kept in egui's callback resources.
struct Resources {
    fill_pipeline: wgpu::RenderPipeline,
    fill_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    /// The random byte table, shared with the sparks.
    points: wgpu::TextureView,
    /// One per `Web` (reading screen, preview), made when first drawn.
    slots: Vec<Option<Slot>>,
}

/// A `Web`'s buffers and its cached sheets.
struct Slot {
    composite_buffer: wgpu::Buffer,
    /// Made with the cache (it reads it).
    composite_bind: Option<wgpu::BindGroup>,
    /// A uniform buffer per sheet, for drawing its rows.
    fill_buffers: Vec<wgpu::Buffer>,
    fill_binds: Vec<wgpu::BindGroup>,
    cache: Option<Cache>,
    /// The settings the cache was drawn with; any change redraws it all.
    key: [f32; 5],
    /// The first world row each sheet's cache holds (None: needs drawing).
    held: [Option<i64>; SHEETS_MAX],
}

/// Each sheet drawn into one layer of a texture array, a row of pixels at a
/// time as it scrolls: world row R lives at image row R mod `rows`.
struct Cache {
    _texture: wgpu::Texture,
    layers: Vec<wgpu::TextureView>,
    width: u32,
    rows: u32,
}

const CACHE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn texture_entry(binding: u32, view_dimension: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension, multisampled: false },
        count: None,
    }
}

fn pipeline(device: &wgpu::Device, name: &str, source: &str, layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(name), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Sets up the shaders. Without wgpu (no render state) the marble just isn't drawn.
pub fn install(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let fill_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("marble fill"),
        entries: &[uniform_entry(0), texture_entry(1, wgpu::TextureViewDimension::D2)],
    });
    let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("marble"),
        entries: &[uniform_entry(0), texture_entry(1, wgpu::TextureViewDimension::D2Array)],
    });
    let fill_pipeline = pipeline(device, "marble fill", include_str!("marble_fill.wgsl"), &fill_layout, CACHE_FORMAT);
    let composite_pipeline = pipeline(device, "marble", include_str!("marble.wgsl"), &composite_layout, render_state.target_format);
    let texture = device.create_texture_with_data(
        &render_state.queue,
        &wgpu::TextureDescriptor {
            label: Some("marble web"),
            size: wgpu::Extent3d { width: T as u32, height: T as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &web_bytes(),
    );
    let points = texture.create_view(&wgpu::TextureViewDescriptor::default());
    render_state.renderer.write().callback_resources.insert(Resources {
        fill_pipeline,
        fill_layout,
        composite_pipeline,
        composite_layout,
        points,
        slots: Vec::new(),
    });
}

impl Resources {
    fn slot(&mut self, i: usize, device: &wgpu::Device) -> &mut Slot {
        if self.slots.len() <= i {
            self.slots.resize_with(i + 1, || None);
        }
        let (fill_layout, points) = (&self.fill_layout, &self.points);
        self.slots[i].get_or_insert_with(|| {
            let buffer = |label, vec4s: usize| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: (vec4s * 16) as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let fill_buffers: Vec<_> = (0..SHEETS_MAX).map(|_| buffer("marble fill", FILL_VEC4S)).collect();
            let fill_binds = fill_buffers
                .iter()
                .map(|b| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("marble fill"),
                        layout: fill_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(points) },
                        ],
                    })
                })
                .collect();
            Slot {
                composite_buffer: buffer("marble", COMPOSITE_VEC4S),
                composite_bind: None,
                fill_buffers,
                fill_binds,
                cache: None,
                key: [0.0; 5],
                held: [None; SHEETS_MAX],
            }
        })
    }
}

struct MarbleCallback {
    slot: usize,
    rect: Rect,
    frame: Frame,
}

/// World rows `from..to` as image rows (start, count), split in two where
/// they wrap past the bottom of the image.
fn image_rows(from: i64, to: i64, rows: u32) -> Vec<(u32, u32)> {
    let count = (to - from) as u32;
    let start = from.rem_euclid(rows as i64) as u32;
    if start + count <= rows {
        vec![(start, count)]
    } else {
        vec![(start, rows - start), (0, count - (rows - start))]
    }
}

impl egui_wgpu::CallbackTrait for MarbleCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get_mut::<Resources>() else { return Vec::new() };
        let ppp = screen.pixels_per_point;
        let width = (self.rect.width() * ppp).round().max(1.0);
        let view_height = (self.rect.height() * ppp).round().max(1.0);
        let height = self.frame.height * ppp;
        // How far the viewport's middle is from the middle of the screen the
        // web is scaled to (non-zero only in the preview).
        let adjust = (height - view_height) / 2.0;
        let offsets_px: [f32; 12] = std::array::from_fn(|k| self.frame.offsets[k] * height);
        let rows = view_height as u32 + 1;

        resources.slot(self.slot, device);
        let Resources { fill_pipeline, composite_layout, slots, .. } = resources;
        let Some(slot) = slots[self.slot].as_mut() else { return Vec::new() };

        // A new size: a new cache, drawn from scratch.
        if slot.cache.as_ref().is_none_or(|c| c.width != width as u32 || c.rows != rows) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("marble sheets"),
                size: wgpu::Extent3d { width: width as u32, height: rows, depth_or_array_layers: SHEETS_MAX as u32 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CACHE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let layers = (0..SHEETS_MAX as u32)
                .map(|k| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: k,
                        array_layer_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect();
            let all = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
            slot.composite_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("marble"),
                layout: composite_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: slot.composite_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&all) },
                ],
            }));
            slot.cache = Some(Cache { _texture: texture, layers, width: width as u32, rows });
            slot.held = [None; SHEETS_MAX];
        }
        // Different veins: redraw everything.
        let key = [height, adjust, self.frame.scale, self.frame.thickness, FILL];
        if key != slot.key {
            slot.key = key;
            slot.held = [None; SHEETS_MAX];
        }

        // Draw the rows of each sheet that have scrolled into view.
        let cache = slot.cache.as_ref().unwrap();
        for k in 0..self.frame.layers {
            let first = offsets_px[k].floor() as i64;
            let new_rows = match slot.held[k] {
                Some(old) if old == first => continue,
                Some(old) if (first - old).abs() < rows as i64 => {
                    if first > old { (old + rows as i64, first + rows as i64) } else { (first, old) }
                }
                _ => (first, first + rows as i64),
            };
            slot.held[k] = Some(first);
            let scale = self.frame.scale * (1.0 + 0.35 * k as f32);
            let fill: [f32; FILL_VEC4S * 4] = [
                width, rows as f32, height, adjust,
                k as f32, scale, self.frame.thickness, FILL,
                first as f32, view_height, 0.0, 0.0,
            ];
            let bytes: Vec<u8> = fill.iter().flat_map(|f| f.to_ne_bytes()).collect();
            queue.write_buffer(&slot.fill_buffers[k], 0, &bytes);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("marble fill"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &cache.layers[k],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(fill_pipeline);
            pass.set_bind_group(0, &slot.fill_binds[k], &[]);
            for (start, count) in image_rows(new_rows.0, new_rows.1, rows) {
                pass.set_scissor_rect(0, start, width as u32, count);
                pass.draw(0..3, 0..1);
            }
        }

        let mut u = self.frame.uniforms.clone();
        u[0] = width;
        u[1] = view_height;
        u[6] = height;
        u[7] = adjust;
        u[8..20].copy_from_slice(&offsets_px);
        // The wave's center and radius.
        for i in 36..39 {
            u[i] *= ppp;
        }
        u[40] = (self.rect.left() * ppp).round();
        u[41] = (self.rect.top() * ppp).round();
        u[42] = rows as f32;
        let bytes: Vec<u8> = u.iter().flat_map(|f| f.to_ne_bytes()).collect();
        queue.write_buffer(&slot.composite_buffer, 0, &bytes);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(resources) = resources.get::<Resources>() else { return };
        let Some(Some(slot)) = resources.slots.get(self.slot) else { return };
        let Some(bind) = &slot.composite_bind else { return };
        pass.set_pipeline(&resources.composite_pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sparks walk from vein to vein for two simulated minutes without
    /// getting stuck at a junction, and stay on the web's borders.
    #[test]
    fn sparks_walk_the_web() {
        let settings = MarbleSettings::default();
        let mut web = Web::new(0);
        for _ in 0..60 * 120 {
            web.time += 1.0 / 60.0;
            web.move_sparks(1.0 / 60.0, 1.8, (0.0, 1.0), &settings);
        }
        assert_eq!(web.dead_ends, 0);
        assert!(!web.sparks.is_empty());
        assert!(web.nodes.len() > 0, "sparks went through junctions");
        for s in &web.sparks {
            // On the border between its two cells: equally far from both points.
            let (x, y) = s.head();
            let (pa, pb) = (web.cell_pt(s.a, s.sheet), web.cell_pt(s.b, s.sheet));
            let gap = dist([x, y], pa) - dist([x, y], pb);
            assert!(gap.abs() < 1e-3, "spark off its vein by {gap}");
        }
    }

    /// Running the warp backwards finds the spot that warps to a given point,
    /// so sparks are drawn where their vein is.
    #[test]
    fn unwarp_inverts_warp() {
        let settings = MarbleSettings::default();
        let web = Web::new(0);
        let mut missed = 0;
        for i in 0..200 {
            let sheet = i % 3;
            let w = [(i as f32 * 0.37).fract() * 1.8, (i as f32 * 0.61).fract() * 3.0 - 1.0];
            let seed = (sheet + 1) as f32 * 13.7;
            let q = web.warp([w[0] + seed, w[1] + seed], Web::scale(sheet, &settings));
            match web.unwarp(q, sheet, &settings) {
                Some(back) => assert!((back[0] - w[0]).abs() + (back[1] - w[1]).abs() < 1e-3, "{w:?} came back as {back:?}"),
                None => missed += 1,
            }
        }
        assert!(missed < 5, "{missed} of 200 spots didn't settle");
    }

    #[test]
    fn rows_wrap_around_the_image() {
        assert_eq!(image_rows(10, 14, 100), vec![(10, 4)]);
        assert_eq!(image_rows(98, 103, 100), vec![(98, 2), (0, 3)]);
        assert_eq!(image_rows(-3, 2, 100), vec![(97, 3), (0, 2)]);
        assert_eq!(image_rows(200, 300, 100), vec![(0, 100)]);
    }
}
