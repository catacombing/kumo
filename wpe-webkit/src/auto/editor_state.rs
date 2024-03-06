// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use std::boxed::Box as Box_;

use glib::prelude::*;
use glib::signal::{connect_raw, SignalHandlerId};
use glib::translate::*;

glib::wrapper! {
    #[doc(alias = "WebKitEditorState")]
    pub struct EditorState(Object<ffi::WebKitEditorState, ffi::WebKitEditorStateClass>);

    match fn {
        type_ => || ffi::webkit_editor_state_get_type(),
    }
}

impl EditorState {
    pub const NONE: Option<&'static EditorState> = None;
}

mod sealed {
    pub trait Sealed {}
    impl<T: super::IsA<super::EditorState>> Sealed for T {}
}

pub trait EditorStateExt: IsA<EditorState> + sealed::Sealed + 'static {
    #[doc(alias = "webkit_editor_state_get_typing_attributes")]
    #[doc(alias = "get_typing_attributes")]
    fn typing_attributes(&self) -> u32 {
        unsafe { ffi::webkit_editor_state_get_typing_attributes(self.as_ref().to_glib_none().0) }
    }

    #[doc(alias = "webkit_editor_state_is_copy_available")]
    fn is_copy_available(&self) -> bool {
        unsafe {
            from_glib(ffi::webkit_editor_state_is_copy_available(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "webkit_editor_state_is_cut_available")]
    fn is_cut_available(&self) -> bool {
        unsafe {
            from_glib(ffi::webkit_editor_state_is_cut_available(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "webkit_editor_state_is_paste_available")]
    fn is_paste_available(&self) -> bool {
        unsafe {
            from_glib(ffi::webkit_editor_state_is_paste_available(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "webkit_editor_state_is_redo_available")]
    fn is_redo_available(&self) -> bool {
        unsafe {
            from_glib(ffi::webkit_editor_state_is_redo_available(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "webkit_editor_state_is_undo_available")]
    fn is_undo_available(&self) -> bool {
        unsafe {
            from_glib(ffi::webkit_editor_state_is_undo_available(self.as_ref().to_glib_none().0))
        }
    }

    #[doc(alias = "typing-attributes")]
    fn connect_typing_attributes_notify<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn notify_typing_attributes_trampoline<
            P: IsA<EditorState>,
            F: Fn(&P) + 'static,
        >(
            this: *mut ffi::WebKitEditorState,
            _param_spec: glib::ffi::gpointer,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(EditorState::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"notify::typing-attributes\0".as_ptr() as *const _,
                Some(std::mem::transmute::<_, unsafe extern "C" fn()>(
                    notify_typing_attributes_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }
}

impl<O: IsA<EditorState>> EditorStateExt for O {}