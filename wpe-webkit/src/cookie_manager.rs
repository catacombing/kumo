use glib::object::IsA;
use glib::translate::*;

use crate::CookieManager;

pub trait CookieManagerExtManual: IsA<CookieManager> + 'static {
    #[doc(alias = "webkit_cookie_manager_replace_cookies")]
    fn replace_cookies<P: FnOnce(Result<(), glib::Error>) + 'static>(
        &self,
        cookies: &[soup::Cookie],
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    );
}

impl<O: IsA<CookieManager>> CookieManagerExtManual for O {
    fn replace_cookies<P: FnOnce(Result<(), glib::Error>) + 'static>(
        &self,
        cookies: &[soup::Cookie],
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

        let user_data: Box<glib::thread_guard::ThreadGuard<P>> =
            Box::new(glib::thread_guard::ThreadGuard::new(callback));
        unsafe extern "C" fn replace_cookies_trampoline<
            P: FnOnce(Result<(), glib::Error>) + 'static,
        >(
            _source_object: *mut glib::gobject_ffi::GObject,
            res: *mut gio::ffi::GAsyncResult,
            user_data: glib::ffi::gpointer,
        ) {
            let mut error = std::ptr::null_mut();
            let _ = ffi::webkit_cookie_manager_replace_cookies_finish(
                _source_object as *mut _,
                res,
                &mut error,
            );
            let result = if error.is_null() { Ok(()) } else { Err(from_glib_full(error)) };
            let callback: Box<glib::thread_guard::ThreadGuard<P>> =
                Box::from_raw(user_data as *mut _);
            let callback: P = callback.into_inner();
            callback(result);
        }
        let callback = replace_cookies_trampoline::<P>;
        unsafe {
            ffi::webkit_cookie_manager_replace_cookies(
                self.as_ref().to_glib_none().0,
                cookies.to_glib_none().0,
                cancellable.map(|p| p.as_ref()).to_glib_none().0,
                Some(callback),
                Box::into_raw(user_data) as *mut _,
            );
        }
    }
}
