//! Trait for subclassing Display.

use std::ffi::CStr;
use std::ptr;

use ffi::{WPEBufferDMABufFormats, WPEDRMDevice, WPEDisplay, WPEInputMethodContext, WPEView};
use glib::object::IsA;
use glib::subclass::prelude::*;
use glib::translate::{ToGlibPtr, *};

use crate::{BufferDMABufFormats, Display, InputMethodContext, Settings, View};

pub trait DisplayExtManual: IsA<Display> + 'static {
    /// Get WPE platform settings.
    fn settings(&self) -> Settings;
}

impl<O: IsA<Display>> DisplayExtManual for O {
    fn settings(&self) -> Settings {
        unsafe { from_glib_none(ffi::wpe_display_get_settings(self.as_ref().to_glib_none().0)) }
    }
}

pub trait DisplayImpl: ObjectImpl {
    /// Create a new [`crate::View`].
    fn create_view(&self) -> &View;

    /// Create a new [`crate::InputMethodContext`].
    fn create_input_method_context(&self) -> &InputMethodContext;

    /// Get acceptable DMA buffer formats.
    fn preferred_dmabuf_formats(&self) -> Option<BufferDMABufFormats>;

    /// Get the DRM device node path.
    fn device_node(&self) -> &CStr;
}

unsafe impl<T: DisplayImpl> IsSubclassable<T> for Display {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.connect = None;
        klass.create_view = Some(create_view::<T>);
        klass.get_egl_display = None;
        klass.get_keymap = None;
        klass.get_preferred_dma_buf_formats = Some(preferred_dmabuf_formats::<T>);
        klass.get_n_screens = None;
        klass.get_screen = None;
        klass.get_drm_device = Some(drm_device::<T>);
        klass.create_input_method_context = Some(create_input_method_context::<T>);
    }
}

unsafe extern "C" fn create_view<T: DisplayImpl>(display: *mut WPEDisplay) -> *mut WPEView {
    unsafe {
        let instance = &*(display as *mut T::Instance);
        instance.imp().create_view().to_glib_none().0
    }
}

unsafe extern "C" fn preferred_dmabuf_formats<T: DisplayImpl>(
    display: *mut WPEDisplay,
) -> *mut WPEBufferDMABufFormats {
    unsafe {
        let instance = &*(display as *mut T::Instance);
        instance.imp().preferred_dmabuf_formats().to_glib_full()
    }
}

unsafe extern "C" fn create_input_method_context<T: DisplayImpl>(
    display: *mut WPEDisplay,
    _view: *mut WPEView,
) -> *mut WPEInputMethodContext {
    unsafe {
        let instance = &*(display as *mut T::Instance);
        instance.imp().create_input_method_context().to_glib_none().0
    }
}

unsafe extern "C" fn drm_device<T: DisplayImpl>(display: *mut WPEDisplay) -> *mut WPEDRMDevice {
    unsafe {
        let instance = &*(display as *mut T::Instance);
        let device_node = instance.imp().device_node().as_ptr();
        ffi::wpe_drm_device_new(device_node, ptr::null())
    }
}
