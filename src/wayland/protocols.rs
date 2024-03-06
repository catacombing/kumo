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

use crate::State;

#[derive(Debug)]
pub struct ProtocolStates {
    pub compositor: CompositorState,
    pub registry: RegistryState,
    pub xdg_shell: XdgShell,
    pub output: OutputState,
}

impl ProtocolStates {
    pub fn new(globals: &GlobalList, wayland_queue: &QueueHandle<State>) -> Self {
        let registry = RegistryState::new(globals);
        let compositor = CompositorState::bind(globals, wayland_queue).unwrap();
        let xdg_shell = XdgShell::bind(globals, wayland_queue).unwrap();
        let output = OutputState::new(globals, wayland_queue);

        Self { registry, compositor, xdg_shell, output }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _scale: i32,
    ) {
        println!("SCALE CHANGED: {}", _scale);
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
        let window = match self.windows.values_mut().find(|w| &w.xdg == window) {
            Some(window) => window,
            None => return,
        };

        // Update window dimensions.
        window.width = configure.new_size.0.map(|w| w.get()).unwrap_or(window.width);
        window.height = configure.new_size.1.map(|h| h.get()).unwrap_or(window.height);

        // Resize window's browser engines.
        for engine_id in &mut window.tabs {
            let engine = match self.engines.get_mut(engine_id) {
                Some(engine) => engine,
                None => continue,
            };
            engine.set_size(window.width, window.height);
        }
    }
}
delegate_xdg_shell!(State);

delegate_xdg_window!(State);

impl ProvidesRegistryState for State {
    registry_handlers![OutputState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.protocol_states.registry
    }
}
delegate_registry!(State);
