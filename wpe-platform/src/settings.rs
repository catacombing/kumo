use glib::translate::*;

use crate::ffi;

glib::wrapper! {
    pub struct Settings(Object<ffi::WPESettings, ffi::WPESettingsClass>);

    match fn {
        type_ => || ffi::wpe_settings_get_type(),
    }
}

impl Settings {
    pub const NONE: Option<&'static Settings> = None;

    pub fn get_boolean(&self, key: &str) -> Result<bool, glib::Error> {
        unsafe {
            let mut error = std::ptr::null_mut();
            let value = ffi::wpe_settings_get_boolean(
                self.to_glib_none().0,
                key.to_glib_none().0,
                &mut error,
            );

            if error.is_null() { Ok(from_glib(value)) } else { Err(from_glib_full(error)) }
        }
    }

    pub fn set_boolean(&self, key: &str, value: bool) -> Result<(), glib::Error> {
        unsafe {
            let mut error = std::ptr::null_mut();
            ffi::wpe_settings_set_boolean(
                self.to_glib_none().0,
                key.to_glib_none().0,
                value.into_glib(),
                ffi::WPE_SETTINGS_SOURCE_PLATFORM,
                &mut error,
            );

            if error.is_null() { Ok(()) } else { Err(from_glib_full(error)) }
        }
    }
}
