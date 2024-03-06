// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use std::boxed::Box as Box_;

use glib::prelude::*;
use glib::signal::{connect_raw, SignalHandlerId};
use glib::translate::*;

use crate::OptionMenuItem;

glib::wrapper! {
    #[doc(alias = "WebKitOptionMenu")]
    pub struct OptionMenu(Object<ffi::WebKitOptionMenu, ffi::WebKitOptionMenuClass>);

    match fn {
        type_ => || ffi::webkit_option_menu_get_type(),
    }
}

impl OptionMenu {
    pub const NONE: Option<&'static OptionMenu> = None;
}

mod sealed {
    pub trait Sealed {}
    impl<T: super::IsA<super::OptionMenu>> Sealed for T {}
}

pub trait OptionMenuExt: IsA<OptionMenu> + sealed::Sealed + 'static {
    #[doc(alias = "webkit_option_menu_activate_item")]
    fn activate_item(&self, index: u32) {
        unsafe {
            ffi::webkit_option_menu_activate_item(self.as_ref().to_glib_none().0, index);
        }
    }

    #[doc(alias = "webkit_option_menu_close")]
    fn close(&self) {
        unsafe {
            ffi::webkit_option_menu_close(self.as_ref().to_glib_none().0);
        }
    }

    #[doc(alias = "webkit_option_menu_get_item")]
    #[doc(alias = "get_item")]
    fn item(&self, index: u32) -> Option<OptionMenuItem> {
        unsafe {
            from_glib_none(ffi::webkit_option_menu_get_item(self.as_ref().to_glib_none().0, index))
        }
    }

    #[doc(alias = "webkit_option_menu_get_n_items")]
    #[doc(alias = "get_n_items")]
    fn n_items(&self) -> u32 {
        unsafe { ffi::webkit_option_menu_get_n_items(self.as_ref().to_glib_none().0) }
    }

    #[doc(alias = "webkit_option_menu_select_item")]
    fn select_item(&self, index: u32) {
        unsafe {
            ffi::webkit_option_menu_select_item(self.as_ref().to_glib_none().0, index);
        }
    }

    #[doc(alias = "close")]
    fn connect_close<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn close_trampoline<P: IsA<OptionMenu>, F: Fn(&P) + 'static>(
            this: *mut ffi::WebKitOptionMenu,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(OptionMenu::from_glib_borrow(this).unsafe_cast_ref())
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"close\0".as_ptr() as *const _,
                Some(std::mem::transmute::<_, unsafe extern "C" fn()>(
                    close_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }
}

impl<O: IsA<OptionMenu>> OptionMenuExt for O {}