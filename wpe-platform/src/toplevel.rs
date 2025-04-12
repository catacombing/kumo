//! Trait for subclassing Toplevel.

use std::ptr;

use ffi::{WPEBufferDMABufFormats, WPEToplevel};
use glib::object::ObjectExt;
use glib::subclass::prelude::*;
use glib::translate::{IntoGlib, ToGlibPtr};
use glib_sys::{GTRUE, gboolean};

use crate::{Display, Toplevel};

pub trait ToplevelImpl: ObjectImpl {
    /// Update window fullscreen state.
    fn set_fullscreen(&self, _fullscreen: bool) -> bool {
        false
    }
}

unsafe impl<T: ToplevelImpl> IsSubclassable<T> for Toplevel {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.set_title = None;
        klass.get_max_views = None;
        klass.get_screen = None;
        klass.resize = None;
        klass.set_fullscreen = Some(set_fullscreen::<T>);
        klass.set_maximized = None;
        klass.get_preferred_dma_buf_formats = Some(get_preferred_dmabuf_formats::<T>);
    }
}

unsafe extern "C" fn set_fullscreen<T: ToplevelImpl>(
    toplevel: *mut WPEToplevel,
    fullscreen: gboolean,
) -> gboolean {
    unsafe {
        let instance = &*(toplevel as *mut T::Instance);
        let fullscreened = instance.imp().set_fullscreen(fullscreen == GTRUE);
        fullscreened.into_glib()
    }
}

unsafe extern "C" fn get_preferred_dmabuf_formats<T: ToplevelImpl>(
    toplevel: *mut WPEToplevel,
) -> *mut WPEBufferDMABufFormats {
    unsafe {
        let instance = &*(toplevel as *mut T::Instance);
        let display: Display = instance.imp().obj().property("display");
        match display.class().as_ref().get_preferred_dma_buf_formats {
            Some(fun) => fun(display.to_glib_full()),
            None => ptr::null_mut(),
        }
    }
}
