use std::ptr;

use glib::object::IsA;
use glib::translate::*;

use crate::{
    NetworkProxyMode, NetworkProxySettings, WebsiteData, WebsiteDataManager, WebsiteDataTypes,
};

pub trait WebsiteDataManagerExtManual: 'static {
    #[doc(alias = "webkit_website_data_manager_set_network_proxy_settings")]
    fn set_network_proxy_settings(
        &self,
        proxy_mode: NetworkProxyMode,
        proxy_settings: Option<&mut NetworkProxySettings>,
    );

    #[doc(alias = "webkit_website_data_manager_clear")]
    fn clear<P: FnOnce(Result<(), glib::Error>) + Send + 'static>(
        &self,
        types: WebsiteDataTypes,
        timespan: glib::TimeSpan,
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    );

    #[doc(alias = "webkit_website_data_manager_remove")]
    fn remove<P: FnOnce(Result<(), glib::Error>) + Send + 'static>(
        &self,
        types: WebsiteDataTypes,
        website_data: &[&WebsiteData],
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    );
}

impl<O: IsA<WebsiteDataManager>> WebsiteDataManagerExtManual for O {
    fn set_network_proxy_settings(
        &self,
        proxy_mode: NetworkProxyMode,
        mut proxy_settings: Option<&mut NetworkProxySettings>,
    ) {
        unsafe {
            ffi::webkit_website_data_manager_set_network_proxy_settings(
                self.as_ref().to_glib_none().0,
                proxy_mode.into_glib(),
                proxy_settings.to_glib_none_mut().0,
            );
        }
    }

    fn clear<P: FnOnce(Result<(), glib::Error>) + Send + 'static>(
        &self,
        types: WebsiteDataTypes,
        timespan: glib::TimeSpan,
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    ) {
        let user_data: Box<P> = Box::new(callback);
        unsafe extern "C" fn clear_trampoline<
            P: FnOnce(Result<(), glib::Error>) + Send + 'static,
        >(
            _source_object: *mut glib::gobject_ffi::GObject,
            res: *mut gio::ffi::GAsyncResult,
            user_data: glib::ffi::gpointer,
        ) {
            let mut error = ptr::null_mut();
            let _ = ffi::webkit_website_data_manager_clear_finish(
                _source_object as *mut _,
                res,
                &mut error,
            );
            let result = if error.is_null() { Ok(()) } else { Err(from_glib_full(error)) };
            let callback: Box<P> = Box::from_raw(user_data as *mut _);
            callback(result);
        }
        let callback = clear_trampoline::<P>;
        unsafe {
            ffi::webkit_website_data_manager_clear(
                self.as_ref().to_glib_none().0,
                types.into_glib(),
                timespan.0,
                cancellable.map(|p| p.as_ref()).to_glib_none().0,
                Some(callback),
                Box::into_raw(user_data) as *mut _,
            );
        }
    }

    fn remove<P: FnOnce(Result<(), glib::Error>) + Send + 'static>(
        &self,
        types: WebsiteDataTypes,
        website_data: &[&WebsiteData],
        cancellable: Option<&impl IsA<gio::Cancellable>>,
        callback: P,
    ) {
        let user_data: Box<P> = Box::new(callback);
        unsafe extern "C" fn remove_trampoline<
            P: FnOnce(Result<(), glib::Error>) + Send + 'static,
        >(
            _source_object: *mut glib::gobject_ffi::GObject,
            res: *mut gio::ffi::GAsyncResult,
            user_data: glib::ffi::gpointer,
        ) {
            let mut error = ptr::null_mut();
            let _ = ffi::webkit_website_data_manager_remove_finish(
                _source_object as *mut _,
                res,
                &mut error,
            );
            let result = if error.is_null() { Ok(()) } else { Err(from_glib_full(error)) };
            let callback: Box<P> = Box::from_raw(user_data as *mut _);
            callback(result);
        }
        let callback = remove_trampoline::<P>;
        unsafe {
            ffi::webkit_website_data_manager_remove(
                self.as_ref().to_glib_none().0,
                types.into_glib(),
                website_data.to_glib_none().0,
                cancellable.map(|p| p.as_ref()).to_glib_none().0,
                Some(callback),
                Box::into_raw(user_data) as *mut _,
            );
        }
    }
}
