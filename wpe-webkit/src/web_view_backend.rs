use std::ptr;

use glib::translate::*;
use wpe_sys::wpe_view_backend;

use crate::WebViewBackend;

impl WebViewBackend {
    #[doc(alias = "webkit_web_view_backend_new")]
    pub unsafe fn new(backend: *mut wpe_view_backend) -> WebViewBackend {
        from_glib_full(unsafe { ffi::webkit_web_view_backend_new(backend, None, ptr::null_mut()) })
    }

    #[doc(alias = "webkit_web_view_backend_get_wpe_backend")]
    #[doc(alias = "get_wpe_backend")]
    pub unsafe fn wpe_backend(&mut self) -> *mut wpe_view_backend {
        unsafe { ffi::webkit_web_view_backend_get_wpe_backend(self.to_glib_none_mut().0) }
    }
}
