use glib::object::IsA;
use glib::translate::*;

use crate::{AuthenticationRequest, Credential};

pub trait AuthenticationRequestExtManual: IsA<AuthenticationRequest> + 'static {
    #[doc(alias = "webkit_authentication_request_authenticate")]
    fn authenticate(&self, credential: Option<&mut Credential>);
}

impl<O: IsA<AuthenticationRequest>> AuthenticationRequestExtManual for O {
    fn authenticate(&self, mut credential: Option<&mut Credential>) {
        unsafe {
            ffi::webkit_authentication_request_authenticate(
                self.as_ref().to_glib_none().0,
                credential.to_glib_none_mut().0,
            );
        }
    }
}
