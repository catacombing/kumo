//! Configuration options.

/// Font configuration.
pub mod font {
    /// Font description.
    pub const FONT: &str = "sans";

    /// Get font size relative to the default.
    pub fn font_size(scale: f64) -> u8 {
        (16. * scale).round() as u8
    }
}

/// Color configuration.
pub mod colors {
    /// Primary background color.
    pub const FG: Color = Color { r: 255, g: 255, b: 255 };
    /// Primary background color.
    pub const BG: Color = Color { r: 24, g: 24, b: 24 };
    /// Primary accent color.
    pub const HL: Color = Color { r: 117, g: 42, b: 42 };

    /// Secondary foreground color.
    pub const SECONDARY_FG: Color = Color { r: 191, g: 191, b: 191 };
    /// Secondary background color.
    pub const SECONDARY_BG: Color = Color { r: 40, g: 40, b: 40 };

    /// Error foreground color.
    pub const ERROR: Color = Color { r: 172, g: 66, b: 66 };
    /// Disabled foreground color.
    pub const DISABLED: Color = Color { r: 102, g: 102, b: 102 };

    /// RGB color.
    #[derive(Copy, Clone)]
    pub struct Color {
        pub r: u8,
        pub g: u8,
        pub b: u8,
    }

    impl Color {
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
}

/// Input configuration.
pub mod input {
    use std::time::Duration;

    /// Square of the maximum distance before touch input is considered a drag.
    pub const MAX_TAP_DISTANCE: f64 = 400.;

    /// Minimum time before a tap is considered a long-press.
    pub const LONG_PRESS: Duration = Duration::from_millis(300);

    /// Microseconds per velocity tick.
    pub const VELOCITY_INTERVAL: f64 = 30_000.;

    /// Percentage of velocity retained each tick.
    pub const VELOCITY_FRICTION: f64 = 0.85;
}
