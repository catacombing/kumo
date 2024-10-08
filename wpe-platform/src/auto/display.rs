// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use std::boxed::Box as Box_;

use glib::prelude::*;
use glib::signal::{connect_raw, SignalHandlerId};
use glib::translate::*;

use crate::{ffi, BufferDMABufFormats, Keymap, Monitor};

glib::wrapper! {
    #[doc(alias = "WPEDisplay")]
    pub struct Display(Object<ffi::WPEDisplay, ffi::WPEDisplayClass>);

    match fn {
        type_ => || ffi::wpe_display_get_type(),
    }
}

impl Display {
    pub const NONE: Option<&'static Display> = None;

    #[doc(alias = "wpe_display_get_default")]
    #[doc(alias = "get_default")]
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Option<Display> {
        unsafe { from_glib_none(ffi::wpe_display_get_default()) }
    }

    #[doc(alias = "wpe_display_get_primary")]
    #[doc(alias = "get_primary")]
    pub fn primary() -> Option<Display> {
        unsafe { from_glib_none(ffi::wpe_display_get_primary()) }
    }
}

mod sealed {
    pub trait Sealed {}
    impl<T: super::IsA<super::Display>> Sealed for T {}
}

pub trait DisplayExt: IsA<Display> + sealed::Sealed + 'static {
    #[doc(alias = "wpe_display_connect")]
    fn connect(&self) -> Result<(), glib::Error> {
        unsafe {
            let mut error = std::ptr::null_mut();
            let is_ok = ffi::wpe_display_connect(self.as_ref().to_glib_none().0, &mut error);
            debug_assert_eq!(is_ok == glib::ffi::GFALSE, !error.is_null());
            if error.is_null() {
                Ok(())
            } else {
                Err(from_glib_full(error))
            }
        }
    }

    #[doc(alias = "wpe_display_get_drm_device")]
    #[doc(alias = "get_drm_device")]
    fn drm_device(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::wpe_display_get_drm_device(self.as_ref().to_glib_none().0)) }
    }

    #[doc(alias = "wpe_display_get_drm_render_node")]
    #[doc(alias = "get_drm_render_node")]
    fn drm_render_node(&self) -> Option<glib::GString> {
        unsafe {
            from_glib_none(ffi::wpe_display_get_drm_render_node(self.as_ref().to_glib_none().0))
        }
    }

    //#[doc(alias = "wpe_display_get_egl_display")]
    //#[doc(alias = "get_egl_display")]
    // fn egl_display(&self) -> Result</*Unimplemented*/Option<Basic: Pointer>,
    // glib::Error> {    unsafe { TODO: call ffi:wpe_display_get_egl_display() }
    //}

    #[doc(alias = "wpe_display_get_keymap")]
    #[doc(alias = "get_keymap")]
    fn keymap(&self) -> Result<Keymap, glib::Error> {
        unsafe {
            let mut error = std::ptr::null_mut();
            let ret = ffi::wpe_display_get_keymap(self.as_ref().to_glib_none().0, &mut error);
            if error.is_null() {
                Ok(from_glib_none(ret))
            } else {
                Err(from_glib_full(error))
            }
        }
    }

    #[doc(alias = "wpe_display_get_monitor")]
    #[doc(alias = "get_monitor")]
    fn monitor(&self, index: u32) -> Option<Monitor> {
        unsafe {
            from_glib_none(ffi::wpe_display_get_monitor(self.as_ref().to_glib_none().0, index))
        }
    }

    #[doc(alias = "wpe_display_get_n_monitors")]
    #[doc(alias = "get_n_monitors")]
    fn n_monitors(&self) -> u32 {
        unsafe { ffi::wpe_display_get_n_monitors(self.as_ref().to_glib_none().0) }
    }

    #[doc(alias = "wpe_display_get_preferred_dma_buf_formats")]
    #[doc(alias = "get_preferred_dma_buf_formats")]
    fn preferred_dma_buf_formats(&self) -> Option<BufferDMABufFormats> {
        unsafe {
            from_glib_none(ffi::wpe_display_get_preferred_dma_buf_formats(
                self.as_ref().to_glib_none().0,
            ))
        }
    }

    #[doc(alias = "wpe_display_monitor_added")]
    fn monitor_added(&self, monitor: &impl IsA<Monitor>) {
        unsafe {
            ffi::wpe_display_monitor_added(
                self.as_ref().to_glib_none().0,
                monitor.as_ref().to_glib_none().0,
            );
        }
    }

    #[doc(alias = "wpe_display_monitor_removed")]
    fn monitor_removed(&self, monitor: &impl IsA<Monitor>) {
        unsafe {
            ffi::wpe_display_monitor_removed(
                self.as_ref().to_glib_none().0,
                monitor.as_ref().to_glib_none().0,
            );
        }
    }

    #[doc(alias = "wpe_display_set_primary")]
    fn set_primary(&self) {
        unsafe {
            ffi::wpe_display_set_primary(self.as_ref().to_glib_none().0);
        }
    }

    #[doc(alias = "wpe_display_use_explicit_sync")]
    fn use_explicit_sync(&self) -> bool {
        unsafe { from_glib(ffi::wpe_display_use_explicit_sync(self.as_ref().to_glib_none().0)) }
    }

    #[doc(alias = "monitor-added")]
    fn connect_monitor_added<F: Fn(&Self, &Monitor) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn monitor_added_trampoline<
            P: IsA<Display>,
            F: Fn(&P, &Monitor) + 'static,
        >(
            this: *mut ffi::WPEDisplay,
            monitor: *mut ffi::WPEMonitor,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(Display::from_glib_borrow(this).unsafe_cast_ref(), &from_glib_borrow(monitor))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"monitor-added\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    monitor_added_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "monitor-removed")]
    fn connect_monitor_removed<F: Fn(&Self, &Monitor) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn monitor_removed_trampoline<
            P: IsA<Display>,
            F: Fn(&P, &Monitor) + 'static,
        >(
            this: *mut ffi::WPEDisplay,
            monitor: *mut ffi::WPEMonitor,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(Display::from_glib_borrow(this).unsafe_cast_ref(), &from_glib_borrow(monitor))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                b"monitor-removed\0".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    monitor_removed_trampoline::<Self, F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }
}

impl<O: IsA<Display>> DisplayExt for O {}
