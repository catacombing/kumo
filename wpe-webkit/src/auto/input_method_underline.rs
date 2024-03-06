// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use glib::translate::*;

use crate::Color;

glib::wrapper! {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct InputMethodUnderline(Boxed<ffi::WebKitInputMethodUnderline>);

    match fn {
        copy => |ptr| ffi::webkit_input_method_underline_copy(mut_override(ptr)),
        free => |ptr| ffi::webkit_input_method_underline_free(ptr),
        type_ => || ffi::webkit_input_method_underline_get_type(),
    }
}

impl InputMethodUnderline {
    #[doc(alias = "webkit_input_method_underline_new")]
    pub fn new(start_offset: u32, end_offset: u32) -> InputMethodUnderline {
        unsafe { from_glib_full(ffi::webkit_input_method_underline_new(start_offset, end_offset)) }
    }

    #[doc(alias = "webkit_input_method_underline_set_color")]
    pub fn set_color(&mut self, color: &mut Color) {
        unsafe {
            ffi::webkit_input_method_underline_set_color(
                self.to_glib_none_mut().0,
                color.to_glib_none_mut().0,
            );
        }
    }
}