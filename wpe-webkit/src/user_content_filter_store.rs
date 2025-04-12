use glib::object::IsA;
use glib::translate::*;

use crate::UserContentFilterStore;

pub trait UserContentFilterStoreExtManual: 'static {
    #[doc(alias = "webkit_user_content_filter_store_fetch_identifiers")]
    fn fetch_identifiers<P: FnOnce(Vec<glib::GString>) + 'static>(
        &self,
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    );
}

impl<O: IsA<UserContentFilterStore>> UserContentFilterStoreExtManual for O {
    fn fetch_identifiers<P: FnOnce(Vec<glib::GString>) + 'static>(
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

        let user_data: Box<glib::thread_guard::ThreadGuard<P>> =
            Box::new(glib::thread_guard::ThreadGuard::new(callback));
        unsafe extern "C" fn fetch_identifiers_trampoline<
            P: FnOnce(Vec<glib::GString>) + 'static,
        >(
            _source_object: *mut glib::gobject_ffi::GObject,
            res: *mut gio::ffi::GAsyncResult,
            user_data: glib::ffi::gpointer,
        ) {
            unsafe {
                let result = FromGlibPtrContainer::from_glib_none(
                    ffi::webkit_user_content_filter_store_fetch_identifiers_finish(
                        _source_object as *mut _,
                        res,
                    ),
                );
                let callback: Box<glib::thread_guard::ThreadGuard<P>> =
                    Box::from_raw(user_data as *mut _);
                let callback: P = callback.into_inner();
                callback(result);
            }
        }
        let callback = fetch_identifiers_trampoline::<P>;
        unsafe {
            ffi::webkit_user_content_filter_store_fetch_identifiers(
                self.as_ref().to_glib_none().0,
                cancellable.map(|p| p.as_ref()).to_glib_none().0,
                Some(callback),
                Box::into_raw(user_data) as *mut _,
            );
        }
    }
}
