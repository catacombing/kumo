// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use glib::translate::*;

use crate::{Feature, ffi};

glib::wrapper! {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FeatureList(Shared<ffi::WebKitFeatureList>);

    match fn {
        ref => |ptr| ffi::webkit_feature_list_ref(ptr),
        unref => |ptr| ffi::webkit_feature_list_unref(ptr),
        type_ => || ffi::webkit_feature_list_get_type(),
    }
}

impl FeatureList {
    #[doc(alias = "webkit_feature_list_get")]
    pub fn get(&self, index: usize) -> Option<Feature> {
        unsafe { from_glib_none(ffi::webkit_feature_list_get(self.to_glib_none().0, index)) }
    }

    #[doc(alias = "webkit_feature_list_get_length")]
    #[doc(alias = "get_length")]
    pub fn length(&self) -> usize {
        unsafe { ffi::webkit_feature_list_get_length(self.to_glib_none().0) }
    }
}
