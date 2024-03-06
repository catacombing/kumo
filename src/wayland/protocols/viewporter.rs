//! Handling of the viewporter protocol.

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::reexports::client::globals::{BindError, GlobalList};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{
    delegate_dispatch, Connection, Dispatch, Proxy, QueueHandle,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::State;

/// Viewporter.
#[derive(Debug)]
pub struct Viewporter {
    viewporter: WpViewporter,
}

impl Viewporter {
    /// Create new viewporter.
    pub fn new(globals: &GlobalList, queue_handle: &QueueHandle<State>) -> Result<Self, BindError> {
        let viewporter = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { viewporter })
    }

    /// Get the viewport for the given object.
    pub fn viewport(&self, queue_handle: &QueueHandle<State>, surface: &WlSurface) -> WpViewport {
        self.viewporter.get_viewport(surface, queue_handle, GlobalData)
    }
}

impl Dispatch<WpViewporter, GlobalData, State> for Viewporter {
    fn event(
        _: &mut State,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        // No events.
    }
}
impl Dispatch<WpViewport, GlobalData, State> for Viewporter {
    fn event(
        _: &mut State,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        // No events.
    }
}

delegate_dispatch!(State: [WpViewporter: GlobalData] => Viewporter);
delegate_dispatch!(State: [WpViewport: GlobalData] => Viewporter);
