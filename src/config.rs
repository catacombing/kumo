//! Configuration options.

use std::fmt::{self, Display, Formatter};
use std::ops::Deref;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use configory::docgen::{DocType, Docgen, Leaf};
use configory::{EventHandler, Manager};
use funq::MtQueueHandle;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer};
use tracing::{error, info};

use crate::{Error, State};

#[funq::callbacks(State)]
trait ConfigHandler {
    /// Config reload handler.
    fn config_reloaded(&mut self);
}

impl ConfigHandler for State {
    fn config_reloaded(&mut self) {
        for window in self.windows.values_mut() {
            // Update settings UI.
            window.reload_settings();

            // Trigger redraw if rendering is stalled.
            window.unstall();
        }
    }
}

/// Shared configuration state.
pub static CONFIG: LazyLock<Arc<RwLock<Config>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Config::default())));

/// Initialize configuration state.
pub fn init_config(queue: MtQueueHandle<State>) -> Result<Manager, Error> {
    let config_manager = Manager::new("kumo", ConfigEventHandler::new(queue))?;

    // Load initial configuration.
    let config = config_manager
        .get::<&str, Config>(&[])
        .inspect_err(|err| error!("Config error: {err}"))
        .ok()
        .flatten()
        .unwrap_or_default();
    *CONFIG.write().unwrap() = config;

    Ok(config_manager)
}

/// # Kumo
///
/// ## Syntax
///
/// Kumo's configuration file uses the TOML format. The format's specification
/// can be found at _https://toml.io/en/v1.0.0_.
///
/// ## Location
///
/// Kumo doesn't create the configuration file for you, but it looks for one at
/// <br> `${XDG_CONFIG_HOME:-$HOME/.config}/kumo/kumo.toml`.
///
/// ## Fields
#[derive(Docgen, Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// This section documents the `[font]` table.
    pub font: Font,
    /// This section documents the `[color]` table.
    pub colors: Colors,
    /// This section documents the `[input]` table.
    pub search: Search,
    /// This section documents the `[input]` table.
    pub input: Input,

    /// Incremental config ID, to track changes.
    #[serde(skip)]
    #[docgen(skip)]
    pub generation: u32,
}

/// Font configuration.
#[derive(Docgen, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    /// Font family.
    pub family: String,
    /// Monospace font family.
    pub monospace_family: String,
    /// Font size.
    pub size: f64,
}

impl Default for Font {
    fn default() -> Self {
        Self { monospace_family: String::from("mono"), family: String::from("sans"), size: 16. }
    }
}

impl Font {
    /// Get font size relative to the default.
    pub fn size(&self, scale: f64) -> u8 {
        (self.size * scale).round() as u8
    }
}

/// Color configuration.
#[derive(Docgen, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    /// Primary foreground color.
    #[serde(alias = "fg")]
    pub foreground: Color,
    /// Primary background color.
    #[serde(alias = "bg")]
    pub background: Color,
    /// Primary accent color.
    #[serde(alias = "hl")]
    pub highlight: Color,

    /// Alternative foreground color.
    #[serde(alias = "alt_fg")]
    pub alt_foreground: Color,
    /// Alternative background color.
    #[serde(alias = "alt_bg")]
    pub alt_background: Color,

    /// Error foreground color.
    pub error: Color,
    /// Disabled foreground color.
    pub disabled: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            foreground: Color::new(255, 255, 255),
            background: Color::new(24, 24, 24),
            highlight: Color::new(117, 42, 42),

            alt_foreground: Color::new(191, 191, 191),
            alt_background: Color::new(40, 40, 40),

            error: Color::new(172, 66, 66),
            disabled: Color::new(102, 102, 102),
        }
    }
}

#[derive(Docgen, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Search {
    /// Search engine URI.
    ///
    /// The search query will be appended to the end of this URI.
    pub uri: String,
}

impl Default for Search {
    fn default() -> Self {
        Self { uri: "https://duckduckgo.com/?q=".into() }
    }
}

/// Input configuration.
#[derive(Docgen, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    /// Square of the maximum distance before touch input is considered a drag.
    pub max_tap_distance: f64,
    /// Maximum interval between taps to be considered a double/trible-tap.
    #[docgen(doc_type = "integer (milliseconds)", default = "300")]
    pub max_multi_tap: MillisDuration,
    /// Minimum time before a tap is considered a long-press.
    #[docgen(doc_type = "integer (milliseconds)", default = "300")]
    pub long_press: MillisDuration,

    /// Milliseconds per velocity tick.
    pub velocity_interval: u16,
    /// Percentage of velocity retained each tick.
    pub velocity_friction: f64,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            max_multi_tap: Duration::from_millis(300).into(),
            long_press: Duration::from_millis(300).into(),
            velocity_interval: 30,
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

impl Docgen for Color {
    fn doc_type() -> DocType {
        DocType::Leaf(Leaf::new("color"))
    }

    fn format(&self) -> String {
        format!("\"#{:0>2x}{:0>2x}{:0>2x}\"", self.r, self.g, self.b)
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

impl Display for Color {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "#{:0>2x}{:0>2x}{:0>2x}", self.r, self.g, self.b)
    }
}

/// Config wrapper for millisecond-precision durations.
#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct MillisDuration(Duration);

impl Deref for MillisDuration {
    type Target = Duration;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MillisDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ms = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(ms).into())
    }
}

impl From<Duration> for MillisDuration {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl Display for MillisDuration {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0.as_millis())
    }
}

/// Event handler for configuration manager updates.
struct ConfigEventHandler {
    queue: MtQueueHandle<State>,
}

impl ConfigEventHandler {
    fn new(queue: MtQueueHandle<State>) -> Self {
        Self { queue }
    }

    /// Reload config and unstall renderer.
    fn reload_config(&self, config: &configory::Config) {
        reload_config(config);
        self.queue.clone().config_reloaded();
    }
}

impl EventHandler<()> for ConfigEventHandler {
    fn file_changed(&self, config: &configory::Config) {
        self.reload_config(config);
    }

    fn ipc_changed(&self, config: &configory::Config) {
        self.reload_config(config);
    }

    fn file_error(&self, _config: &configory::Config, err: configory::Error) {
        error!("Configuration file error: {err}");
    }
}

/// Update the configuration file.
pub fn reload_config(config: &configory::Config) {
    info!("Reloading configuration file");

    // Parse config or fall back to the default.
    let parsed = config
        .get::<&str, Config>(&[])
        .inspect_err(|err| error!("Config error: {err}"))
        .ok()
        .flatten()
        .unwrap_or_default();

    // Calculate generation based on current config.
    let mut config = CONFIG.write().unwrap();
    let next_generation = config.generation + 1;

    // Update the config.
    *config = parsed;
    config.generation = next_generation;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use configory::docgen::markdown::Markdown;

    use super::*;

    #[test]
    fn config_docs() {
        let mut formatter = Markdown::new();
        formatter.set_heading_size(3);
        let expected = formatter.format::<Config>();

        // Uncomment to update config documentation.
        // fs::write("./docs/config.md", &expected).unwrap();

        // Ensure documentation is up to date.
        let docs = fs::read_to_string("./docs/config.md").unwrap();
        assert_eq!(docs, expected);
    }
}
