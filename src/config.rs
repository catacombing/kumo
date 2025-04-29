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
    pub const FG: [f64; 3] = [1., 1., 1.];
    /// Primary background color.
    pub const BG: [f64; 3] = [0.1, 0.1, 0.1];
    /// Primary accent color.
    pub const HL: [f64; 3] = [0.46, 0.16, 0.16];

    /// Secondary foreground color.
    pub const SECONDARY_FG: [f64; 3] = [0.75, 0.75, 0.75];
    /// Secondary background color.
    pub const SECONDARY_BG: [f64; 3] = [0.15, 0.15, 0.15];

    /// Error foreground color.
    pub const ERROR: [f64; 3] = [0.67, 0.26, 0.26];
    /// Disabled foreground color.
    pub const DISABLED: [f64; 3] = [0.4, 0.4, 0.4];

    /// Convert [`f64`] color to [`u8`] color.
    ///
    /// If the output array is longer than 3 elements, the remaining elements
    /// will be filled with `255`.
    pub fn to_u8<const N: usize>(color: [f64; 3]) -> [u8; N] {
        let mut u8_color = [255; N];
        u8_color[0] = (color[0] * 255.).round() as u8;
        u8_color[1] = (color[1] * 255.).round() as u8;
        u8_color[2] = (color[2] * 255.).round() as u8;
        u8_color
    }
}

/// Input configuration.
pub mod input {
    use std::time::Duration;

    /// Square of the maximum distance before touch input is considered a drag.
    pub const MAX_TAP_DISTANCE: f64 = 400.;

    /// Minimum time before a tap is considered a long-press.
    pub const LONG_PRESS: Duration = Duration::from_millis(300);
}
