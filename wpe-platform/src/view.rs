//! Trait for subclassing View.

use std::ffi::{CStr, c_char, c_uint};
use std::slice;

use ffi::{WPEBuffer, WPERectangle, WPEView};
use glib::subclass::prelude::*;
use glib::translate::{IntoGlib, from_glib_none};
use glib_sys::{GError, gboolean};

use crate::{Buffer, View};

pub trait ViewImpl: ObjectImpl {
    /// Render a new buffer.
    ///
    /// An empty array of rectangles indicates **full** damage, rather than no
    /// damage at all.
    fn render_buffer(&self, buffer: Buffer, damage_rects: &[WPERectangle]);

    /// Update the buffer's opaque region.
    fn set_opaque_rectangles(&self, _rects: &[WPERectangle]) {}

    /// Update the cursor to a named cursor.
    fn set_cursor_from_name(&self, _name: &str) {}
}

unsafe impl<T: ViewImpl> IsSubclassable<T> for View {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.render_buffer = Some(render_buffer::<T>);
        klass.set_cursor_from_name = Some(set_cursor_from_name::<T>);
        klass.set_cursor_from_bytes = None;
        klass.set_opaque_rectangles = Some(set_opaque_rectangles::<T>);
        klass.can_be_mapped = None;
    }
}

unsafe extern "C" fn render_buffer<T: ViewImpl>(
    view: *mut WPEView,
    buffer: *mut WPEBuffer,
    damage_rects: *const WPERectangle,
    n_damage_rects: c_uint,
    _error: *mut *mut GError,
) -> gboolean {
    unsafe {
        let damage_rects = if n_damage_rects > 0 {
            slice::from_raw_parts(damage_rects, n_damage_rects as usize)
        } else {
            &[]
        };
        let buffer: Buffer = from_glib_none(buffer);

        let instance = &*(view as *mut T::Instance);
        instance.imp().render_buffer(buffer, damage_rects);
        true.into_glib()
    }
}

unsafe extern "C" fn set_cursor_from_name<T: ViewImpl>(view: *mut WPEView, name: *const c_char) {
    unsafe {
        if let Ok(name) = CStr::from_ptr(name).to_str() {
            let instance = &*(view as *mut T::Instance);
            instance.imp().set_cursor_from_name(name);
        }
    }
}

unsafe extern "C" fn set_opaque_rectangles<T: ViewImpl>(
    view: *mut WPEView,
    rects: *mut WPERectangle,
    n_rects: c_uint,
) {
    unsafe {
        let rects = slice::from_raw_parts(rects, n_rects as usize);
        let instance = &*(view as *mut T::Instance);
        instance.imp().set_opaque_rectangles(rects);
    }
}
