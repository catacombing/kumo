use libc::c_int;

use crate::Rectangle;

impl Rectangle {
    /// Get rectangle X position.
    pub fn x(&self) -> c_int {
        unsafe { (*self.as_ptr()).x }
    }

    /// Get rectangle Y position.
    pub fn y(&self) -> c_int {
        unsafe { (*self.as_ptr()).y }
    }

    /// Get rectangle width.
    pub fn width(&self) -> c_int {
        unsafe { (*self.as_ptr()).width }
    }

    /// Get rectangle height.
    pub fn height(&self) -> c_int {
        unsafe { (*self.as_ptr()).height }
    }

    /// Get rectangle dimensions.
    pub fn geometry(&self) -> (c_int, c_int, c_int, c_int) {
        (self.x(), self.y(), self.width(), self.height())
    }
}
