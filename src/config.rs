//! Configuration options.

use std::fmt::{self, Formatter};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use serde::de::Visitor;
use serde::{Deserialize, Deserializer};

/// Shared configuration state.
pub static CONFIG: LazyLock<Arc<RwLock<Config>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Config::default())));

#[derive(Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub colors: Colors,
    pub input: Input,

    /// Incremental config ID, to track changes.
    #[serde(skip)]
    pub generation: u32,
}

/// Font configuration.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    /// Font family.
    pub family: String,
    size: f64,
}

impl Default for Font {
    fn default() -> Self {
        Self { family: String::from("sans"), size: 16. }
    }
}

impl Font {
    /// Get font size relative to the default.
    pub fn size(&self, scale: f64) -> u8 {
        (self.size * scale).round() as u8
    }
}

/// Color configuration.
#[derive(Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    /// Primary background color.
    pub fg: Color,
    /// Primary background color.
    pub bg: Color,
    /// Primary accent color.
    pub hl: Color,

    /// Secondary foreground color.
    pub secondary_fg: Color,
    /// Secondary background color.
    pub secondary_bg: Color,

    /// Error foreground color.
    pub error: Color,
    /// Disabled foreground color.
    pub disabled: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            fg: Color::new(255, 255, 255),
            bg: Color::new(24, 24, 24),
            hl: Color::new(117, 42, 42),

            secondary_fg: Color::new(191, 191, 191),
            secondary_bg: Color::new(40, 40, 40),

            error: Color::new(172, 66, 66),
            disabled: Color::new(102, 102, 102),
        }
    }
}

/// Input configuration.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    /// Square of the maximum distance before touch input is considered a drag.
    pub max_tap_distance: f64,
    /// Maximum interval between taps to be considered a double/trible-tap.
    #[serde(deserialize_with = "duration_ms")]
    pub max_multi_tap: Duration,
    /// Minimum time before a tap is considered a long-press.
    #[serde(deserialize_with = "duration_ms")]
    pub long_press: Duration,

    /// Microseconds per velocity tick.
    pub velocity_interval: f64,
    /// Percentage of velocity retained each tick.
    pub velocity_friction: f64,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            max_multi_tap: Duration::from_millis(300),
            long_press: Duration::from_millis(300),
            velocity_interval: 30_000.,
            velocity_friction: 0.85,
            max_tap_distance: 400.,
        }
    }
}

/// RGB color.
#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn as_u8(&self) -> [u8; 4] {
        [self.r, self.g, self.b, 255]
    }

    pub const fn as_u16(&self) -> [u16; 3] {
        let factor = u16::MAX / u8::MAX as u16;
        [self.r as u16 * factor, self.g as u16 * factor, self.b as u16 * factor]
    }

    pub const fn as_u32(&self) -> [u32; 3] {
        let factor = u32::MAX / u8::MAX as u32;
        [self.r as u32 * factor, self.g as u32 * factor, self.b as u32 * factor]
    }

    pub const fn as_f32(&self) -> [f32; 3] {
        [self.r as f32 / 255., self.g as f32 / 255., self.b as f32 / 255.]
    }

    pub const fn as_f64(&self) -> [f64; 3] {
        [self.r as f64 / 255., self.g as f64 / 255., self.b as f64 / 255.]
    }
}

/// Deserialize rgb color from a hex string.
impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ColorVisitor;

        impl Visitor<'_> for ColorVisitor {
            type Value = Color;

            fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("hex color like #ff00ff")
            }

            fn visit_str<E>(self, value: &str) -> Result<Color, E>
            where
                E: serde::de::Error,
            {
                let channels = match value.strip_prefix('#') {
                    Some(channels) => channels,
                    None => {
                        return Err(E::custom(format!("color {value:?} is missing leading '#'")));
                    },
                };

                let digits = channels.len();
                if digits != 6 {
                    let msg = format!("color {value:?} has {digits} digits; expected 6");
                    return Err(E::custom(msg));
                }

                match u32::from_str_radix(channels, 16) {
                    Ok(mut color) => {
                        let b = (color & 0xFF) as u8;
                        color >>= 8;
                        let g = (color & 0xFF) as u8;
                        color >>= 8;
                        let r = color as u8;

                        Ok(Color::new(r, g, b))
                    },
                    Err(_) => Err(E::custom(format!("color {value:?} contains non-hex digits"))),
                }
            }
        }

        deserializer.deserialize_str(ColorVisitor)
    }
}

/// Deserialize rgb color from a hex string.
fn duration_ms<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let ms = u64::deserialize(deserializer)?;
    Ok(Duration::from_millis(ms))
}
