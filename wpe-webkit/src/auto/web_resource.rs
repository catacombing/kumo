// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use std::boxed::Box as Box_;
use std::pin::Pin;

use glib::object::ObjectType as _;
use glib::prelude::*;
use glib::signal::{SignalHandlerId, connect_raw};
use glib::translate::*;

use crate::{URIRequest, URIResponse, ffi};

glib::wrapper! {
    #[doc(alias = "WebKitWebResource")]
    pub struct WebResource(Object<ffi::WebKitWebResource, ffi::WebKitWebResourceClass>);

    match fn {
        type_ => || ffi::webkit_web_resource_get_type(),
    }
}

impl WebResource {
    #[doc(alias = "webkit_web_resource_get_data")]
    #[doc(alias = "get_data")]
    pub fn data<P: FnOnce(Result<Vec<u8>, glib::Error>) + 'static>(
        &self,
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    ) {
        let main_context = glib::MainContext::ref_thread_default();
        let is_main_context_owner = main_context.is_owner();
        let has_acquired_main_context =
            (!is_main_context_owner).then(|| main_context.acquire().ok()).flatten();
        assert!(
            is_main_context_owner || has_acquired_main_context.is_some(),
            "Async operations only allowed if the thread is owning the MainContext"
        );

        let user_data: Box_<glib::thread_guard::ThreadGuard<P>> =
            Box_::new(glib::thread_guard::ThreadGuard::new(callback));
        unsafe extern "C" fn data_trampoline<P: FnOnce(Result<Vec<u8>, glib::Error>) + 'static>(
            _source_object: *mut glib::gobject_ffi::GObject,
            res: *mut gio::ffi::GAsyncResult,
            user_data: glib::ffi::gpointer,
        ) {
            let mut error = std::ptr::null_mut();
            let mut length = std::mem::MaybeUninit::uninit();
            let ret = ffi::webkit_web_resource_get_data_finish(
                _source_object as *mut _,
                res,
                length.as_mut_ptr(),
                &mut error,
            );
            let result = if error.is_null() {
                Ok(FromGlibContainer::from_glib_full_num(ret, length.assume_init() as _))
            } else {
                Err(from_glib_full(error))
            };
            let callback: Box_<glib::thread_guard::ThreadGuard<P>> =
                Box_::from_raw(user_data as *mut _);
            let callback: P = callback.into_inner();
            callback(result);
        }
        let callback = data_trampoline::<P>;
        unsafe {
            ffi::webkit_web_resource_get_data(
                self.to_glib_none().0,
                cancellable.map(|p| p.as_ref()).to_glib_none().0,
                Some(callback),
                Box_::into_raw(user_data) as *mut _,
            );
        }
    }

    pub fn data_future(
        &self,
    ) -> Pin<Box_<dyn std::future::Future<Output = Result<Vec<u8>, glib::Error>> + 'static>> {
        Box_::pin(gio::GioFuture::new(self, move |obj, cancellable, send| {
            obj.data(Some(cancellable), move |res| {
                send.resolve(res);
            });
        }))
    }

    #[doc(alias = "webkit_web_resource_get_response")]
    #[doc(alias = "get_response")]
    pub fn response(&self) -> Option<URIResponse> {
        unsafe { from_glib_none(ffi::webkit_web_resource_get_response(self.to_glib_none().0)) }
    }

    #[doc(alias = "webkit_web_resource_get_uri")]
    #[doc(alias = "get_uri")]
    pub fn uri(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::webkit_web_resource_get_uri(self.to_glib_none().0)) }
    }

    #[doc(alias = "failed")]
    pub fn connect_failed<F: Fn(&Self, &glib::Error) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn failed_trampoline<F: Fn(&WebResource, &glib::Error) + 'static>(
            this: *mut ffi::WebKitWebResource,
            error: *mut glib::ffi::GError,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(&from_glib_borrow(this), &from_glib_borrow(error))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"failed".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    failed_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "failed-with-tls-errors")]
    pub fn connect_failed_with_tls_errors<
        F: Fn(&Self, &gio::TlsCertificate, gio::TlsCertificateFlags) + 'static,
    >(
        &self,
        f: F,
    ) -> SignalHandlerId {
        unsafe extern "C" fn failed_with_tls_errors_trampoline<
            F: Fn(&WebResource, &gio::TlsCertificate, gio::TlsCertificateFlags) + 'static,
        >(
            this: *mut ffi::WebKitWebResource,
            certificate: *mut gio::ffi::GTlsCertificate,
            errors: gio::ffi::GTlsCertificateFlags,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(&from_glib_borrow(this), &from_glib_borrow(certificate), from_glib(errors))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"failed-with-tls-errors".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    failed_with_tls_errors_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "finished")]
    pub fn connect_finished<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn finished_trampoline<F: Fn(&WebResource) + 'static>(
            this: *mut ffi::WebKitWebResource,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(&from_glib_borrow(this))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"finished".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    finished_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "sent-request")]
    pub fn connect_sent_request<F: Fn(&Self, &URIRequest, &URIResponse) + 'static>(
        &self,
        f: F,
    ) -> SignalHandlerId {
        unsafe extern "C" fn sent_request_trampoline<
            F: Fn(&WebResource, &URIRequest, &URIResponse) + 'static,
        >(
            this: *mut ffi::WebKitWebResource,
            request: *mut ffi::WebKitURIRequest,
            redirected_response: *mut ffi::WebKitURIResponse,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(
                &from_glib_borrow(this),
                &from_glib_borrow(request),
                &from_glib_borrow(redirected_response),
            )
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"sent-request".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    sent_request_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "response")]
    pub fn connect_response_notify<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn notify_response_trampoline<F: Fn(&WebResource) + 'static>(
            this: *mut ffi::WebKitWebResource,
            _param_spec: glib::ffi::gpointer,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(&from_glib_borrow(this))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"notify::response".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    notify_response_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }

    #[doc(alias = "uri")]
    pub fn connect_uri_notify<F: Fn(&Self) + 'static>(&self, f: F) -> SignalHandlerId {
        unsafe extern "C" fn notify_uri_trampoline<F: Fn(&WebResource) + 'static>(
            this: *mut ffi::WebKitWebResource,
            _param_spec: glib::ffi::gpointer,
            f: glib::ffi::gpointer,
        ) {
            let f: &F = &*(f as *const F);
            f(&from_glib_borrow(this))
        }
        unsafe {
            let f: Box_<F> = Box_::new(f);
            connect_raw(
                self.as_ptr() as *mut _,
                c"notify::uri".as_ptr() as *const _,
                Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>(
                    notify_uri_trampoline::<F> as *const (),
                )),
                Box_::into_raw(f),
            )
        }
    }
}
