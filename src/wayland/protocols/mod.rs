//! Wayland protocol handling.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_xdg_shell,
    delegate_xdg_window, registry_handlers,
};

use crate::wayland::protocols::fractional_scale::{FractionalScaleHandler, FractionalScaleManager};
use crate::wayland::protocols::viewporter::Viewporter;
use crate::State;

pub mod fractional_scale;
pub mod viewporter;

#[derive(Debug)]
pub struct ProtocolStates {
    pub fractional_scale: FractionalScaleManager,
    pub compositor: CompositorState,
    pub registry: RegistryState,
    pub viewporter: Viewporter,
    pub xdg_shell: XdgShell,
    pub output: OutputState,
}

impl ProtocolStates {
    pub fn new(globals: &GlobalList, wayland_queue: &QueueHandle<State>) -> Self {
        let registry = RegistryState::new(globals);
        let fractional_scale = FractionalScaleManager::new(globals, wayland_queue).unwrap();
        let compositor = CompositorState::bind(globals, wayland_queue).unwrap();
        let viewporter = Viewporter::new(globals, wayland_queue).unwrap();
        let xdg_shell = XdgShell::bind(globals, wayland_queue).unwrap();
        let output = OutputState::new(globals, wayland_queue);

        Self { fractional_scale, viewporter, registry, compositor, xdg_shell, output }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
        // NOTE: We exclusively use fractional scaling.
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: Transform,
    ) {
    }

    fn frame(
        &mut self,
        _connection: &Connection,
        queue: &QueueHandle<Self>,
        surface: &WlSurface,
        _serial: u32,
    ) {
        let window = self.windows.values_mut().find(|window| window.xdg.wl_surface() == surface);
        if let Some(window) = window {
            window.draw(queue, &self.engines);
        }
    }
}
delegate_compositor!(State);

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.protocol_states.output
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}
delegate_output!(State);

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.terminated = true;
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let window = self.windows.values_mut().find(|w| &w.xdg == window);
        if let Some(window) = window {
            // Update window dimensions.
            let width = configure.new_size.0.map(|w| w.get()).unwrap_or(window.width);
            let height = configure.new_size.1.map(|h| h.get()).unwrap_or(window.height);
            window.set_size(&mut self.engines, width, height);
        }
    }
}
delegate_xdg_shell!(State);
delegate_xdg_window!(State);

impl FractionalScaleHandler for State {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        surface: &WlSurface,
        scale: f64,
    ) {
        let window = self.windows.values_mut().find(|w| w.xdg.wl_surface() == surface);
        if let Some(window) = window {
            window.set_scale(&mut self.engines, scale);
        }
    }
}

impl ProvidesRegistryState for State {
    registry_handlers![OutputState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.protocol_states.registry
    }
}
delegate_registry!(State);
