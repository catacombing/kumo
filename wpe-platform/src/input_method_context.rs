//! Trait for subclassing InputMethodContext.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::{cmp, ptr};

use ffi::{WPEInputMethodContext, wpe_input_method_underline_new};
use glib::subclass::prelude::*;
use glib_sys::{GList, g_list_prepend};

use crate::InputMethodContext;

pub trait InputMethodContextImpl: ObjectImpl {
    /// Element with IME support was focused.
    fn focus_in(&self);

    /// Element with IME support was unfocused.
    fn focus_out(&self);

    /// Set the IME cursor rectangle.
    fn set_cursor_area(&self, x: i32, y: i32, width: i32, height: i32);

    /// Set the surrounding text.
    fn set_surrounding(&self, text: &str, cursor_index: u32, selection_index: u32);

    /// Get current preedit string.
    fn preedit_string(&self) -> Option<PreeditString>;

    /// Reset the IME state.
    fn reset(&self);
}

unsafe impl<T: InputMethodContextImpl> IsSubclassable<T> for InputMethodContext {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.get_preedit_string = Some(get_preedit_string::<T>);
        klass.filter_key_event = None;
        klass.focus_in = Some(focus_in::<T>);
        klass.focus_out = Some(focus_out::<T>);
        klass.set_cursor_area = Some(set_cursor_area::<T>);
        klass.set_surrounding = Some(set_surrounding::<T>);
        klass.reset = Some(reset::<T>);
    }
}

unsafe extern "C" fn get_preedit_string<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
    return_text: *mut *mut c_char,
    underlines: *mut *mut GList,
    cursor_offset: *mut c_uint,
) {
    unsafe {
        // Check all the pointers.
        if return_text.is_null() || underlines.is_null() || cursor_offset.is_null() {
            return;
        }

        let instance = &*(input_method_context as *mut T::Instance);
        let (text, cursor_begin, cursor_end) = match instance.imp().preedit_string() {
            Some(PreeditString { text, cursor_begin, cursor_end }) => {
                (text, cursor_begin, cursor_end)
            },
            None => return,
        };

        *return_text = CString::new(text).unwrap().into_raw();

        // Only set cursor offset when the cursor is visible.
        if cursor_begin > 0 {
            *cursor_offset = cursor_begin as c_uint;
        }

        // Add underline between cursor start and end.
        *underlines = ptr::null_mut();
        if cursor_begin > 0 && cursor_end > 0 && cursor_begin != cursor_end {
            let underline = wpe_input_method_underline_new(cursor_begin as u32, cursor_end as u32);
            *underlines = g_list_prepend(*underlines, underline.cast());
        }
    }
}

unsafe extern "C" fn focus_in<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
) {
    unsafe {
        let instance = &*(input_method_context as *mut T::Instance);
        instance.imp().focus_in();
    }
}

unsafe extern "C" fn focus_out<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
) {
    unsafe {
        let instance = &*(input_method_context as *mut T::Instance);
        instance.imp().focus_out();
    }
}

unsafe extern "C" fn set_cursor_area<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
) {
    unsafe {
        let instance = &*(input_method_context as *mut T::Instance);
        instance.imp().set_cursor_area(x, y, width, height);
    }
}

unsafe extern "C" fn set_surrounding<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
    text: *const c_char,
    length: c_uint,
    cursor_index: c_uint,
    selection_index: c_uint,
) {
    unsafe {
        if let Ok(text) = CStr::from_ptr(text).to_str() {
            // Clamp text to specified maximum length.
            let length = cmp::min(length as usize, text.len());
            let text = &text[..length];

            let instance = &*(input_method_context as *mut T::Instance);
            instance.imp().set_surrounding(text, cursor_index, selection_index);
        }
    }
}

unsafe extern "C" fn reset<T: InputMethodContextImpl>(
    input_method_context: *mut WPEInputMethodContext,
) {
    unsafe {
        let instance = &*(input_method_context as *mut T::Instance);
        instance.imp().reset();
    }
}

/// IME preedit string details.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PreeditString {
    pub text: String,
    pub cursor_begin: i32,
    pub cursor_end: i32,
}
