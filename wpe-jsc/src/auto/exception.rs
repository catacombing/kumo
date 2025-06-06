// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use glib::translate::*;

use crate::{Context, ffi};

glib::wrapper! {
    #[doc(alias = "JSCException")]
    pub struct Exception(Object<ffi::JSCException, ffi::JSCExceptionClass>);

    match fn {
        type_ => || ffi::jsc_exception_get_type(),
    }
}

impl Exception {
    #[doc(alias = "jsc_exception_new")]
    pub fn new(context: &Context, message: &str) -> Exception {
        unsafe {
            from_glib_full(ffi::jsc_exception_new(
                context.to_glib_none().0,
                message.to_glib_none().0,
            ))
        }
    }

    //#[doc(alias = "jsc_exception_new_printf")]
    // pub fn new_printf(context: &Context, format: &str, : /*Unknown
    // conversion*//*Unimplemented*/Basic: VarArgs) -> Exception {    unsafe {
    // TODO: call ffi:jsc_exception_new_printf() }
    //}

    //#[doc(alias = "jsc_exception_new_vprintf")]
    // pub fn new_vprintf(context: &Context, format: &str, args: /*Unknown
    // conversion*//*Unimplemented*/Unsupported) -> Exception {    unsafe {
    // TODO: call ffi:jsc_exception_new_vprintf() }
    //}

    #[doc(alias = "jsc_exception_new_with_name")]
    #[doc(alias = "new_with_name")]
    pub fn with_name(context: &Context, name: &str, message: &str) -> Exception {
        unsafe {
            from_glib_full(ffi::jsc_exception_new_with_name(
                context.to_glib_none().0,
                name.to_glib_none().0,
                message.to_glib_none().0,
            ))
        }
    }

    //#[doc(alias = "jsc_exception_new_with_name_printf")]
    //#[doc(alias = "new_with_name_printf")]
    // pub fn with_name_printf(context: &Context, name: &str, format: &str, :
    // /*Unknown conversion*//*Unimplemented*/Basic: VarArgs) -> Exception {
    //    unsafe { TODO: call ffi:jsc_exception_new_with_name_printf() }
    //}

    //#[doc(alias = "jsc_exception_new_with_name_vprintf")]
    //#[doc(alias = "new_with_name_vprintf")]
    // pub fn with_name_vprintf(context: &Context, name: &str, format: &str, args:
    // /*Unknown conversion*//*Unimplemented*/Unsupported) -> Exception {
    //    unsafe { TODO: call ffi:jsc_exception_new_with_name_vprintf() }
    //}

    #[doc(alias = "jsc_exception_get_backtrace_string")]
    #[doc(alias = "get_backtrace_string")]
    pub fn backtrace_string(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::jsc_exception_get_backtrace_string(self.to_glib_none().0)) }
    }

    #[doc(alias = "jsc_exception_get_column_number")]
    #[doc(alias = "get_column_number")]
    pub fn column_number(&self) -> u32 {
        unsafe { ffi::jsc_exception_get_column_number(self.to_glib_none().0) }
    }

    #[doc(alias = "jsc_exception_get_line_number")]
    #[doc(alias = "get_line_number")]
    pub fn line_number(&self) -> u32 {
        unsafe { ffi::jsc_exception_get_line_number(self.to_glib_none().0) }
    }

    #[doc(alias = "jsc_exception_get_message")]
    #[doc(alias = "get_message")]
    pub fn message(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::jsc_exception_get_message(self.to_glib_none().0)) }
    }

    #[doc(alias = "jsc_exception_get_name")]
    #[doc(alias = "get_name")]
    pub fn name(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::jsc_exception_get_name(self.to_glib_none().0)) }
    }

    #[doc(alias = "jsc_exception_get_source_uri")]
    #[doc(alias = "get_source_uri")]
    pub fn source_uri(&self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::jsc_exception_get_source_uri(self.to_glib_none().0)) }
    }

    #[doc(alias = "jsc_exception_report")]
    pub fn report(&self) -> Option<glib::GString> {
        unsafe { from_glib_full(ffi::jsc_exception_report(self.to_glib_none().0)) }
    }

    #[doc(alias = "jsc_exception_to_string")]
    #[doc(alias = "to_string")]
    pub fn to_str(&self) -> glib::GString {
        unsafe { from_glib_full(ffi::jsc_exception_to_string(self.to_glib_none().0)) }
    }
}

impl std::fmt::Display for Exception {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.to_str())
    }
}
