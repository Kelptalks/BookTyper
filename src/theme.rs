//! Color themes: the built-in presets, your saved custom ones, and the
//! settings file that remembers them.
//!
//! %APPDATA%\BookTyper\settings.txt:
//!   current<TAB>#text<TAB>#background<TAB>#cursor
//!   custom<TAB>#text<TAB>#background<TAB>#cursor<TAB><name>
//!   hop<TAB><correct-letter hop strength, 1 = normal>
//!   shake<TAB><wrong-letter shake strength, 1 = normal>
//!   zoom<TAB><Ctrl +/- zoom, 1 = normal>
//!   background<TAB>dots | grain | streaks | marble | plain
//!   trail<TAB>on | off<TAB><length, 1 = normal><TAB><brightness, 1 = normal>
//!   marble<TAB><scroll speed><TAB><vein thickness><TAB><web size><TAB>plain | accent | fade<TAB><sparks, 0 = off><TAB>lamp | nolamp<TAB><layers><TAB><each layer's speed, compared to the one above>

use std::path::PathBuf;

use eframe::egui::Color32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Typed text, headings, numbers.
    pub text: Color32,
    pub background: Color32,
    /// The cursor bar and progress bars.
    pub cursor: Color32,
}

impl Theme {
    /// Text you haven't typed yet: partway from the background to the text color.
    pub fn upcoming(&self) -> Color32 {
        self.background.lerp_to_gamma(self.text, 0.45)
    }

    /// A slight shade of the background, for highlights and empty bars.
    pub fn shade(&self, amount: f32) -> Color32 {
        self.background.lerp_to_gamma(self.text, amount)
    }

    pub fn is_light(&self) -> bool {
        self.background.intensity() > 0.5
    }
}

pub const PRESETS: [(&str, Theme); 6] = [
    ("Dark", Theme {
        text: Color32::from_rgb(225, 225, 225),
        background: Color32::from_rgb(27, 27, 27),
        cursor: Color32::from_rgb(235, 190, 90),
    }),
    ("Light", Theme {
        text: Color32::from_rgb(34, 34, 34),
        background: Color32::from_rgb(250, 248, 243),
        cursor: Color32::from_rgb(205, 125, 25),
    }),
    ("Sepia", Theme {
        text: Color32::from_rgb(88, 66, 48),
        background: Color32::from_rgb(244, 234, 212),
        cursor: Color32::from_rgb(176, 98, 40),
    }),
    ("Midnight", Theme {
        text: Color32::from_rgb(205, 214, 235),
        background: Color32::from_rgb(19, 23, 36),
        cursor: Color32::from_rgb(110, 168, 255),
    }),
    ("Forest", Theme {
        text: Color32::from_rgb(212, 226, 206),
        background: Color32::from_rgb(23, 32, 26),
        cursor: Color32::from_rgb(150, 210, 115),
    }),
    ("High contrast", Theme {
        text: Color32::WHITE,
        background: Color32::BLACK,
        cursor: Color32::from_rgb(255, 220, 0),
    }),
];

pub struct Settings {
    pub theme: Theme,
    pub custom: Vec<(String, Theme)>,
    /// How far correct letters hop: 0 = off, 1 = normal.
    pub hop: f32,
    /// How far wrong letters shake: 0 = off, 1 = normal.
    pub shake: f32,
    /// How far you've zoomed in with Ctrl +/-: 1 = normal.
    pub zoom: f32,
    /// What's behind the text while reading.
    pub background: Background,
    /// The glow behind the cursor: shown at all, how long (1 = normal), how bright.
    pub trail: bool,
    pub trail_length: f32,
    pub trail_brightness: f32,
    pub marble: MarbleSettings,
}

/// The marble background's settings (1 = normal for the numbers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarbleSettings {
    /// How fast the web scrolls as you type.
    pub scroll: f32,
    pub thickness: f32,
    /// How big the web's cells are.
    pub size: f32,
    pub veins: Veins,
    /// How bright the sparks running along the veins are: 0 = none.
    pub sparks: f32,
    /// Veins near the line you're typing are lit brighter.
    pub lamp: bool,
    /// Sheets of veins stacked for depth (1 to `MAX_LAYERS`).
    pub layers: usize,
    /// How fast each sheet scrolls compared to the one above it.
    pub ratio: f32,
}

/// Most marble layers (the shader draws up to this many).
pub const MAX_LAYERS: usize = 10;

impl Default for MarbleSettings {
    fn default() -> Self {
        MarbleSettings { scroll: 1.0, thickness: 1.0, size: 1.0, veins: Veins::Fade, sparks: 1.0, lamp: true, layers: 3, ratio: 0.5 }
    }
}

/// What color the marble's veins are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Veins {
    /// A shade of the text color.
    Plain,
    /// A shade of the cursor color.
    Accent,
    /// Cursor color on the top sheet, a step closer to plain on each sheet below.
    Fade,
}

impl Veins {
    fn name(self) -> &'static str {
        match self {
            Veins::Plain => "plain",
            Veins::Accent => "accent",
            Veins::Fade => "fade",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Dots,
    /// Faint random specks, like paper.
    Grain,
    /// Particles flying toward you, faster the faster you type.
    Streaks,
    /// Layers of veins that scroll as you type, with sparks running along them.
    Marble,
    Plain,
}

impl Background {
    fn name(self) -> &'static str {
        match self {
            Background::Dots => "dots",
            Background::Grain => "grain",
            Background::Streaks => "streaks",
            Background::Marble => "marble",
            Background::Plain => "plain",
        }
    }
}

/// Highest the hop and shake sliders go.
pub const MAX_STRENGTH: f32 = 3.0;

fn settings_file() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("BookTyper").join("settings.txt"))
}

impl Settings {
    pub fn load() -> Settings {
        let mut settings = Settings { theme: PRESETS[0].1, custom: Vec::new(), hop: 1.0, shake: 1.0, zoom: 1.0,
            background: Background::Dots,
            trail: true,
            trail_length: 1.0,
            trail_brightness: 1.0,
            marble: MarbleSettings::default(),
        };
        let text = settings_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let theme = |at: usize| {
                Some(Theme { text: parse_hex(fields.get(at)?)?, background: parse_hex(fields.get(at + 1)?)?, cursor: parse_hex(fields.get(at + 2)?)? })
            };
            match fields.first() {
                Some(&"current") => settings.theme = theme(1).unwrap_or(settings.theme),
                Some(&"custom") => {
                    if let (Some(theme), Some(name)) = (theme(1), fields.get(4)) {
                        settings.custom.push((name.to_string(), theme));
                    }
                }
                Some(&"hop") => settings.hop = strength(fields.get(1)).unwrap_or(settings.hop),
                Some(&"shake") => settings.shake = strength(fields.get(1)).unwrap_or(settings.shake),
                Some(&"trail") => {
                    settings.trail = fields.get(1) != Some(&"off");
                    settings.trail_length = strength(fields.get(2)).unwrap_or(settings.trail_length);
                    settings.trail_brightness = strength(fields.get(3)).unwrap_or(settings.trail_brightness);
                }
                Some(&"background") => {
                    let all = [Background::Dots, Background::Grain, Background::Streaks, Background::Marble, Background::Plain];
                    if let Some(choice) = all.into_iter().find(|b| fields.get(1) == Some(&b.name())) {
                        settings.background = choice;
                    }
                }
                Some(&"marble") => {
                    let m = &mut settings.marble;
                    m.scroll = strength(fields.get(1)).unwrap_or(m.scroll).max(0.1);
                    m.thickness = strength(fields.get(2)).unwrap_or(m.thickness).max(0.2);
                    m.size = strength(fields.get(3)).unwrap_or(m.size).max(0.4);
                    let all = [Veins::Plain, Veins::Accent, Veins::Fade];
                    m.veins = all.into_iter().find(|v| fields.get(4) == Some(&v.name())).unwrap_or(m.veins);
                    m.sparks = strength(fields.get(5)).unwrap_or(m.sparks);
                    m.lamp = fields.get(6) != Some(&"nolamp");
                    if let Some(layers) = fields.get(7).and_then(|f| f.parse::<usize>().ok()) {
                        m.layers = layers.clamp(1, MAX_LAYERS);
                    }
                    m.ratio = strength(fields.get(8)).unwrap_or(m.ratio).clamp(0.1, 1.0);
                }
                Some(&"zoom") => {
                    let zoom = fields.get(1).and_then(|f| f.parse::<f32>().ok()).filter(|z| z.is_finite());
                    settings.zoom = zoom.map_or(settings.zoom, |z| z.clamp(0.25, 5.0));
                }
                _ => {}
            }
        }
        settings
    }

    /// Writes the settings file. Done on a background thread because antivirus
    /// can make even small writes take most of a second.
    pub fn save(&self) -> std::thread::JoinHandle<()> {
        let mut text = format!("current\t{}\n", theme_fields(&self.theme));
        for (name, theme) in &self.custom {
            text += &format!("custom\t{}\t{name}\n", theme_fields(theme));
        }
        text += &format!("hop\t{}\nshake\t{}\nzoom\t{}\n", self.hop, self.shake, self.zoom);
        text += &format!("background\t{}\n", self.background.name());
        let trail = if self.trail { "on" } else { "off" };
        text += &format!("trail\t{trail}\t{}\t{}\n", self.trail_length, self.trail_brightness);
        let m = &self.marble;
        let lamp = if m.lamp { "lamp" } else { "nolamp" };
        text += &format!("marble\t{}\t{}\t{}\t{}\t{}\t{lamp}\t{}\t{}\n", m.scroll, m.thickness, m.size, m.veins.name(), m.sparks, m.layers, m.ratio);
        std::thread::spawn(move || {
            let Some(file) = settings_file() else { return };
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&file, text) {
                eprintln!("couldn't save settings to {}: {e}", file.display());
            }
        })
    }
}

fn strength(field: Option<&&str>) -> Option<f32> {
    let value: f32 = field?.parse().ok()?;
    value.is_finite().then(|| value.clamp(0.0, MAX_STRENGTH))
}

fn theme_fields(theme: &Theme) -> String {
    format!("{}\t{}\t{}", to_hex(theme.text), to_hex(theme.background), to_hex(theme.cursor))
}

fn to_hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

fn parse_hex(text: &str) -> Option<Color32> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let color = Color32::from_rgb(12, 200, 255);
        assert_eq!(parse_hex(&to_hex(color)), Some(color));
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(parse_hex("123456"), None);
    }

    #[test]
    fn strength_is_clamped() {
        assert_eq!(strength(Some(&"1.5")), Some(1.5));
        assert_eq!(strength(Some(&"-1")), Some(0.0));
        assert_eq!(strength(Some(&"99")), Some(MAX_STRENGTH));
        assert_eq!(strength(Some(&"NaN")), None);
        assert_eq!(strength(None), None);
    }
}
