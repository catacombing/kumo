// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use std::boxed::Box as Box_;

use glib::prelude::*;
use glib::signal::{connect_raw, SignalHandlerId};
use glib::translate::*;

use crate::{ffi, Display, Event, InputHints, InputPurpose, View};

glib::wrapper! {
    #[doc(alias = "WPEInputMethodContext")]
    pub struct InputMethodContext(Object<ffi::WPEInputMethodContext, ffi::WPEInputMethodContextClass>);

    match fn {
        type_ => || ffi::wpe_input_method_context_get_type(),
    }
}

impl InputMethodContext {
    pub const NONE: Option<&'static InputMethodContext> = None;

    #[doc(alias = "wpe_input_method_context_new")]
    pub fn new(view: &impl IsA<View>) -> InputMethodContext {
        unsafe { from_glib_full(ffi::wpe_input_method_context_new(view.as_ref().to_glib_none().0)) }
    }
}

mod sealed {
    pub trait Sealed {}
    impl<T: super::IsA<super::InputMethodContext>> Sealed for T {}
}

pub trait InputMethodContextExt: IsA<InputMethodContext> + sealed::Sealed + 'static {
    #[doc(alias = "wpe_input_method_context_filter_key_event")]
    fn filter_key_event(&self, event: &Event) -> bool {
        unsafe {
            from_glib(ffi::wpe_input_method_context_filter_key_event(
                self.as_ref().to_glib_none().0,
                event.to_glib_none().0,
            ))
        }
    }

    #[doc(alias = "wpe_input_method_context_focus_in")]
    fn focus_in(&self) {
        unsafe {
            ffi::wpe_input_method_context_focus_in(self.as_ref().to_glib_none().0);
        }
    }

    #[doc(alias = "wpe_input_method_context_focus_out")]
    fn focus_out(&self) {
        unsafe {
            ffi::wpe_input_method_context_focus_out(self.as_ref().to_glib_none().0);
        }
    }

    #[doc(alias = "wpe_input_method_context_get_display")]
    #[doc(alias = "get_display")]
    fn display(&self) -> Option<Display> {
        unsafe {
            from_glib_none(ffi::wpe_input_method_context_get_display(
                self.as_ref().to_glib_none().0,
            ))
        }
    }

    //#[doc(alias = "wpe_input_method_context_get_preedit_string")]
    //#[doc(alias = "get_preedit_string")]
    // fn preedit_string(&self, underlines:
    // /*Unimplemented*/Vec<InputMethodUnderline>) -> (Option<glib::GString>, u32) {
    //    unsafe { TODO: call ffi:wpe_input_method_context_get_preedit_string() }
    //}

    #[doc(alias = "wpe_input_method_context_get_view")]
    #[doc(alias = "get_view")]
    fn view(&self) -> Option<View> {
        unsafe {
            from_glib_none(ffi::wpe_input_method_context_get_view(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "wpe_input_method_context_reset")]
    fn reset(&self) {
        unsafe {
            ffi::wpe_input_method_context_reset(self.as_ref().to_glib_none().0);
        }
    }

    #[doc(alias = "wpe_input_method_context_set_cursor_area")]
    fn set_cursor_area(&self, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            ffi::wpe_input_method_context_set_cursor_area(
                self.as_ref().to_glib_none().0,
                x,
                y,
                width,
                height,
            );
        }
    }

    #[doc(alias = "wpe_input_method_context_set_surrounding")]
    fn set_surrounding(&self, text: &str, cursor_index: u32, selection_index: u32) {
        let length = text.len() as _;
        unsafe {
            ffi::wpe_input_method_context_set_surrounding(
                self.as_ref().to_glib_none().0,
                text.to_glib_none().0,
                length,
                cursor_index,
                selection_index,
            );
        }
    }

    #[doc(alias = "input-hints")]
    fn input_hints(&self) -> InputHints {
        ObjectExt::property(self.as_ref(), "input-hints")
    }

    #[doc(alias = "input-hints")]
    fn set_input_hints(&self, input_hints: InputHints) {
        ObjectExt::set_property(self.as_ref(), "input-hints", input_hints)
    }

    #[doc(alias = "input-purpose")]
    fn input_purpose(&self) -> InputPurpose {
        ObjectExt::property(self.as_ref(), "input-purpose")
    }

    #[doc(alias = "input-purpose")]
    fn set_input_purpose(&self, input_purpose: InputPurpose) {
        ObjectExt::set_property(self.as_ref(), "input-purpose", input_purpose)
    }

    #[doc(alias = "committed")]
    fn connect_committed<F: Fn(&Self, &str) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn committed_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P, &str) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            text: *mut libc::c_char,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(
                InputMethodContext::from_glib_borrow(this).unsafe_cast_ref(),
                &glib::GString::from_glib_borrow(text),
            )
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"committed\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    committed_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "delete-surrounding")]
    fn connect_delete_surrounding<F: Fn(&Self, i32, u32) + 'static>(
        &self,
        f: F,
    ) -> SignalHandlerId {
        unsafe extern "C" fn delete_surrounding_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P, i32, u32) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            offset: libc::c_int,
            n_chars: libc::c_uint,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref(), offset, n_chars)
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"delete-surrounding\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    delete_surrounding_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "preedit-changed")]
    fn connect_preedit_changed<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn preedit_changed_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"preedit-changed\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    preedit_changed_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "preedit-finished")]
    fn connect_preedit_finished<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn preedit_finished_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"preedit-finished\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    preedit_finished_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "preedit-started")]
    fn connect_preedit_started<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn preedit_started_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"preedit-started\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    preedit_started_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "input-hints")]
    fn connect_input_hints_notify<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn notify_input_hints_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            _param_spec: glib::ffi::gpointer,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"notify::input-hints\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    notify_input_hints_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "input-purpose")]
    fn connect_input_purpose_notify<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn notify_input_purpose_trampoline<
            P: IsA<InputMethodContext>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WPEInputMethodContext,
            _param_spec: glib::ffi::gpointer,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(InputMethodContext::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"notify::input-purpose\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    notify_input_purpose_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }
}

impl<O: IsA<InputMethodContext>> InputMethodContextExt for O {}