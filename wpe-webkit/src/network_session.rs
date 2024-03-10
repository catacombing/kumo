use glib::object::IsA;
use glib::translate::*;

use crate::{NetworkProxyMode, NetworkProxySettings, NetworkSession};

pub trait NetworkSessionExtManual: IsA<NetworkSession> + 'static {
    #[doc(alias = "webkit_network_session_set_proxy_settings")]
    fn set_proxy_settings(
        &self,
        proxy_mode: NetworkProxyMode,
        proxy_settings: Option<&mut NetworkProxySettings>,
    );
}

impl<O: IsA<NetworkSession>> NetworkSessionExtManual for O {
    fn set_proxy_settings(
        &self,
        proxy_mode: NetworkProxyMode,
        mut proxy_settings: Option<&mut NetworkProxySettings>,
    ) {
        unsafe {
            ffi::webkit_network_session_set_proxy_settings(
                self.as_ref().to_glib_none().0,
                proxy_mode.into_glib(),
                proxy_settings.to_glib_none_mut().0,
            );
        }
    }
}
