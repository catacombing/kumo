// This file was generated by gir (https://github.com/gtk-rs/gir)
// from /usr/share/gir-1.0
// from ../gir-files
// DO NOT EDIT

use glib::prelude::*;
use glib::translate::*;

use crate::{CredentialPersistence, ffi};

glib::wrapper! {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Credential(Boxed<ffi::WebKitCredential>);

    match fn {
        copy => |ptr| ffi::webkit_credential_copy(mut_override(ptr)),
        free => |ptr| ffi::webkit_credential_free(ptr),
        type_ => || ffi::webkit_credential_get_type(),
    }
}

impl Credential {
    #[doc(alias = "webkit_credential_new")]
    pub fn new(username: &str, password: &str, persistence: CredentialPersistence) -> Credential {
        unsafe {
            from_glib_full(ffi::webkit_credential_new(
                username.to_glib_none().0,
                password.to_glib_none().0,
                persistence.into_glib(),
            ))
        }
    }

    #[doc(alias = "webkit_credential_new_for_certificate")]
    #[doc(alias = "new_for_certificate")]
    pub fn for_certificate(
        certificate: Option<&impl IsA<gio::TlsCertificate>>,
        persistence: CredentialPersistence,
    ) -> Credential {
        unsafe {
            from_glib_full(ffi::webkit_credential_new_for_certificate(
                certificate.map(|p| p.as_ref()).to_glib_none().0,
                persistence.into_glib(),
            ))
        }
    }

    #[doc(alias = "webkit_credential_new_for_certificate_pin")]
    #[doc(alias = "new_for_certificate_pin")]
    pub fn for_certificate_pin(pin: &str, persistence: CredentialPersistence) -> Credential {
        unsafe {
            from_glib_full(ffi::webkit_credential_new_for_certificate_pin(
                pin.to_glib_none().0,
                persistence.into_glib(),
            ))
        }
    }

    #[doc(alias = "webkit_credential_get_certificate")]
    #[doc(alias = "get_certificate")]
    pub fn certificate(&mut self) -> Option<gio::TlsCertificate> {
        unsafe { from_glib_none(ffi::webkit_credential_get_certificate(self.to_glib_none_mut().0)) }
    }

    #[doc(alias = "webkit_credential_get_password")]
    #[doc(alias = "get_password")]
    pub fn password(&mut self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::webkit_credential_get_password(self.to_glib_none_mut().0)) }
    }

    #[doc(alias = "webkit_credential_get_persistence")]
    #[doc(alias = "get_persistence")]
    pub fn persistence(&mut self) -> CredentialPersistence {
        unsafe { from_glib(ffi::webkit_credential_get_persistence(self.to_glib_none_mut().0)) }
    }

    #[doc(alias = "webkit_credential_get_username")]
    #[doc(alias = "get_username")]
    pub fn username(&mut self) -> Option<glib::GString> {
        unsafe { from_glib_none(ffi::webkit_credential_get_username(self.to_glib_none_mut().0)) }
    }

    #[doc(alias = "webkit_credential_has_password")]
    pub fn has_password(&mut self) -> bool {
        unsafe { from_glib(ffi::webkit_credential_has_password(self.to_glib_none_mut().0)) }
    }
}
