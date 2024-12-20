// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use glib::bitflags::bitflags;
use glib::translate::*;

use crate::ffi;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[doc(alias = "JSCValuePropertyFlags")]
    pub struct ValuePropertyFlags: u32 {
        #[doc(alias = "JSC_VALUE_PROPERTY_CONFIGURABLE")]
        const CONFIGURABLE = ffi::JSC_VALUE_PROPERTY_CONFIGURABLE as _;
        #[doc(alias = "JSC_VALUE_PROPERTY_ENUMERABLE")]
        const ENUMERABLE = ffi::JSC_VALUE_PROPERTY_ENUMERABLE as _;
        #[doc(alias = "JSC_VALUE_PROPERTY_WRITABLE")]
        const WRITABLE = ffi::JSC_VALUE_PROPERTY_WRITABLE as _;
    }
}

#[doc(hidden)]
impl IntoGlib for ValuePropertyFlags {
    type GlibType = ffi::JSCValuePropertyFlags;

    #[inline]
    fn into_glib(self) -> ffi::JSCValuePropertyFlags {
        self.bits()
    }
}

#[doc(hidden)]
impl FromGlib<ffi::JSCValuePropertyFlags> for ValuePropertyFlags {
    #[inline]
    unsafe fn from_glib(value: ffi::JSCValuePropertyFlags) -> Self {
        Self::from_bits_truncate(value)
    }
}
