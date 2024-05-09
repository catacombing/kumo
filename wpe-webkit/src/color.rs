use ffi::WebKitColor;
use glib::translate::*;

use crate::Color;

impl Color {
    /// Create a new color.
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        let color = WebKitColor { red, green, blue, alpha };
        unsafe { from_glib_none(&color as *const _) }
    }
}
