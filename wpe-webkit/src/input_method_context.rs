//! Trait for implementing an IME handler.

use std::cmp;
use std::ffi::{c_char, CStr};

use ffi::WebKitInputMethodContext;
use glib::subclass::prelude::*;
pub use wpe_sys::wpe_input_keyboard_event;

use crate::InputMethodContext;

pub trait InputMethodContextImpl: ObjectImpl {
    fn notify_focus_in(&self);
    fn notify_focus_out(&self);
    fn notify_cursor_area(&self, x: i32, y: i32, width: i32, height: i32);
    fn notify_surrounding(&self, text: &str, cursor_index: u32, selection_index: u32);
    fn reset(&self);
}

unsafe impl<T: InputMethodContextImpl> IsSubclassable<T> for InputMethodContext {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.preedit_started = None;
        klass.preedit_changed = None;
        klass.preedit_finished = None;
        klass.committed = None;
        klass.delete_surrounding = None;
        klass.set_enable_preedit = None;
        klass.get_preedit = None;
        klass.filter_key_event = None;
        klass.notify_focus_in = Some(notify_focus_in::<T>);
        klass.notify_focus_out = Some(notify_focus_out::<T>);
        klass.notify_cursor_area = Some(notify_cursor_area::<T>);
        klass.notify_surrounding = Some(notify_surrounding::<T>);
        klass.reset = Some(reset::<T>);
    }
}

unsafe extern "C" fn notify_focus_in<T: InputMethodContextImpl>(
    context: *mut WebKitInputMethodContext,
) {
    let instance = &*(context as *mut T::Instance);
    instance.imp().notify_focus_in();
}

unsafe extern "C" fn notify_focus_out<T: InputMethodContextImpl>(
    context: *mut WebKitInputMethodContext,
) {
    let instance = &*(context as *mut T::Instance);
    instance.imp().notify_focus_out();
}

unsafe extern "C" fn notify_cursor_area<T: InputMethodContextImpl>(
    context: *mut WebKitInputMethodContext,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let instance = &*(context as *mut T::Instance);
    instance.imp().notify_cursor_area(x, y, width, height);
}

unsafe extern "C" fn notify_surrounding<T: InputMethodContextImpl>(
    context: *mut WebKitInputMethodContext,
    text: *const c_char,
    length: u32,
    cursor_index: u32,
    selection_index: u32,
) {
    if let Ok(text) = CStr::from_ptr(text).to_str() {
        // Clamp text to
        let length = cmp::min(length as usize, text.len());
        let text = &text[..length];

        let instance = &*(context as *mut T::Instance);
        instance.imp().notify_surrounding(text, cursor_index, selection_index);
    }
}

unsafe extern "C" fn reset<T: InputMethodContextImpl>(context: *mut WebKitInputMethodContext) {
    let instance = &*(context as *mut T::Instance);
    instance.imp().reset();
}
