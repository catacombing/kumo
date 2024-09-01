//! Trait for subclassing Display.

use std::ffi::{c_char, CStr};

use ffi::{WPEBufferDMABufFormats, WPEDisplay, WPEInputMethodContext, WPEView};
use glib::subclass::prelude::*;
use glib::translate::ToGlibPtr;

use crate::{BufferDMABufFormats, Display, InputMethodContext, View};

pub trait DisplayImpl: ObjectImpl {
    /// Create a new [`crate::View`].
    fn create_view(&self) -> &View;

    /// Create a new [`crate::InputMethodContext`].
    fn create_input_method_context(&self) -> &InputMethodContext;

    /// Get acceptable DMA buffer formats.
    fn preferred_dmabuf_formats(&self) -> Option<BufferDMABufFormats>;

    /// Get the DRM render node path.
    fn render_node(&self) -> &CStr;
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
        klass.get_n_monitors = None;
        klass.get_monitor = None;
        klass.get_drm_device = None;
        klass.get_drm_render_node = Some(render_node::<T>);
        klass.create_input_method_context = Some(create_input_method_context::<T>);
    }
}

unsafe extern "C" fn create_view<T: DisplayImpl>(display: *mut WPEDisplay) -> *mut WPEView {
    let instance = &*(display as *mut T::Instance);
    instance.imp().create_view().to_glib_none().0
}

unsafe extern "C" fn preferred_dmabuf_formats<T: DisplayImpl>(
    display: *mut WPEDisplay,
) -> *mut WPEBufferDMABufFormats {
    let instance = &*(display as *mut T::Instance);
    instance.imp().preferred_dmabuf_formats().to_glib_full()
}

unsafe extern "C" fn create_input_method_context<T: DisplayImpl>(
    display: *mut WPEDisplay,
) -> *mut WPEInputMethodContext {
    let instance = &*(display as *mut T::Instance);
    instance.imp().create_input_method_context().to_glib_none().0
}

unsafe extern "C" fn render_node<T: DisplayImpl>(display: *mut WPEDisplay) -> *const c_char {
    let instance = &*(display as *mut T::Instance);
    instance.imp().render_node().as_ptr()
}
