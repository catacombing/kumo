//! Handling of the fractional scaling protocol.

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::reexports::client::globals::{BindError, GlobalList};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{delegate_dispatch, Connection, Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    Event as FractionalScalingEvent, WpFractionalScaleV1,
};

use crate::State;

/// The scaling factor denominator.
const SCALE_DENOMINATOR: f64 = 120.;

/// Handle fractional scaling protocol events.
pub trait FractionalScaleHandler: Sized {
    /// Update surface's fractional scale.
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _surface: &WlSurface,
        _factor: f64,
    );
}

/// Fractional scaling manager.
#[derive(Debug)]
pub struct FractionalScaleManager {
    manager: WpFractionalScaleManagerV1,
}

impl FractionalScaleManager {
    /// Create new viewporter.
    pub fn new(globals: &GlobalList, queue_handle: &QueueHandle<State>) -> Result<Self, BindError> {
        let manager = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { manager })
    }

    pub fn fractional_scaling(
        &self,
        queue_handle: &QueueHandle<State>,
        surface: &WlSurface,
    ) -> WpFractionalScaleV1 {
        let data = FractionalScale { surface: surface.clone() };
        self.manager.get_fractional_scale(surface, queue_handle, data)
    }
}

impl Dispatch<WpFractionalScaleManagerV1, GlobalData, State> for FractionalScaleManager {
    fn event(
        _: &mut State,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        // No events.
    }
}

pub struct FractionalScale {
    surface: WlSurface,
}

impl Dispatch<WpFractionalScaleV1, FractionalScale, State> for FractionalScale {
    fn event(
        state: &mut State,
        _: &WpFractionalScaleV1,
        event: <WpFractionalScaleV1 as Proxy>::Event,
        data: &FractionalScale,
        connection: &Connection,
        queue: &QueueHandle<State>,
    ) {
        if let FractionalScalingEvent::PreferredScale { scale } = event {
            let fractional_scale = scale as f64 / SCALE_DENOMINATOR;
            state.scale_factor_changed(connection, queue, &data.surface, fractional_scale);
        }
    }
}

delegate_dispatch!(State: [WpFractionalScaleManagerV1: GlobalData] => FractionalScaleManager);
delegate_dispatch!(State: [WpFractionalScaleV1: FractionalScale] => FractionalScale);
