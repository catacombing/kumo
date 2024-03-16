//! Wayland protocol handling.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RepeatInfo,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::subcompositor::SubcompositorState;
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_subcompositor, delegate_touch, delegate_xdg_shell, delegate_xdg_window,
    registry_handlers,
};

use crate::wayland::protocols::fractional_scale::{FractionalScaleHandler, FractionalScaleManager};
use crate::wayland::protocols::viewporter::Viewporter;
use crate::{KeyboardState, Size, State};

pub mod fractional_scale;
pub mod viewporter;

#[derive(Debug)]
pub struct ProtocolStates {
    pub fractional_scale: FractionalScaleManager,
    pub subcompositor: SubcompositorState,
    pub compositor: CompositorState,
    pub registry: RegistryState,
    pub viewporter: Viewporter,
    pub xdg_shell: XdgShell,
    pub output: OutputState,
    pub seat: SeatState,
}

impl ProtocolStates {
    pub fn new(globals: &GlobalList, queue: &QueueHandle<State>) -> Self {
        let registry = RegistryState::new(globals);
        let compositor = CompositorState::bind(globals, queue).unwrap();
        let wl_compositor = compositor.wl_compositor().clone();
        let fractional_scale = FractionalScaleManager::new(globals, queue).unwrap();
        let subcompositor = SubcompositorState::bind(wl_compositor, globals, queue).unwrap();
        let viewporter = Viewporter::new(globals, queue).unwrap();
        let xdg_shell = XdgShell::bind(globals, queue).unwrap();
        let output = OutputState::new(globals, queue);
        let seat = SeatState::new(globals, queue);

        Self {
            fractional_scale,
            subcompositor,
            compositor,
            viewporter,
            xdg_shell,
            registry,
            output,
            seat,
        }
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
        let window = self.windows.values_mut().find(|window| window.owns_surface(surface));
        if let Some(window) = window {
            window.draw(queue, &mut self.engines);
        }
    }
}
delegate_compositor!(State);
delegate_subcompositor!(State);

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
        self.main_loop.quit();
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
            let width = configure.new_size.0.map(|w| w.get()).unwrap_or(window.size.width);
            let height = configure.new_size.1.map(|h| h.get()).unwrap_or(window.size.height);
            window.set_size(&self.egl_display, &mut self.engines, Size { width, height });
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
        let window = self.windows.values_mut().find(|w| w.owns_surface(surface));
        if let Some(window) = window {
            window.set_scale(&mut self.engines, scale);
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.protocol_states.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _connection: &Connection,
        queue: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                let keyboard = self.protocol_states.seat.get_keyboard(queue, &seat, None).ok();
                self.keyboard = keyboard.map(|kbd| KeyboardState::new(self.queue.handle(), kbd));
            },
            Capability::Pointer if self.pointer.is_none() => {
                self.pointer = self.protocol_states.seat.get_pointer(queue, &seat).ok();
            },
            Capability::Touch if self.touch.is_none() => {
                self.touch = self.protocol_states.seat.get_touch(queue, &seat).ok();
            },
            _ => (),
        }
    }

    fn remove_capability(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => self.keyboard = None,
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            },
            Capability::Touch => {
                if let Some(touch) = self.touch.take() {
                    touch.release();
                }
            },
            _ => (),
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}
delegate_seat!(State);

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        _serial: u32,
        _raws: &[u32],
        _keysyms: &[Keysym],
    ) {
        // Update window with keyboard focus.
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };
        self.keyboard_focus = Some(window.id);
    }

    fn leave(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };

        // Cancel active key repetition.
        keyboard_state.cancel_repeat();

        // Update window with keyboard focus.
        self.keyboard_focus = None;
    }

    fn press_key(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };
        keyboard_state.press_key(event.raw_code, event.keysym);

        // Update pressed keys.
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        window.press_key(&mut self.engines, event.raw_code, event.keysym, keyboard_state.modifiers);
    }

    fn release_key(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };
        keyboard_state.release_key(event.raw_code);

        // Update pressed keys.
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        let modifiers = keyboard_state.modifiers;
        window.release_key(&mut self.engines, event.raw_code, event.keysym, modifiers);
    }

    fn update_modifiers(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };

        // Update pressed modifiers.
        keyboard_state.modifiers = modifiers;
    }

    fn update_repeat_info(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        repeat_info: RepeatInfo,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };

        // Update keyboard repeat state.
        keyboard_state.repeat_info = repeat_info;
    }
}
delegate_keyboard!(State);

#[funq::callbacks(State)]
pub trait KeyRepeat {
    fn repeat_key(&mut self);
}

impl KeyRepeat for State {
    fn repeat_key(&mut self) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };
        let (raw, keysym, modifiers) = match keyboard_state.repeat_key() {
            Some(repeat_key) => repeat_key,
            None => return,
        };

        // Once the timeout completed, we need to clear the GLib repeat source ID, since
        // removing an invalid source ID causes a panic.
        keyboard_state.current_repeat.take();

        // Update pressed keys.
        if let Some(window) = self.keyboard_focus.and_then(|focus| self.windows.get(&focus)) {
            window.press_key(&mut self.engines, raw, keysym, modifiers);
        }

        // Request next repeat.
        keyboard_state.request_repeat(raw, keysym, false);
    }
}

impl TouchHandler for State {
    fn down(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _touch: &WlTouch,
        _serial: u32,
        time: u32,
        surface: WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        // Update window with touch focus.
        let window = match self.windows.values_mut().find(|win| win.owns_surface(&surface)) {
            Some(window) => window,
            None => return,
        };
        self.touch_focus = Some((window.id, surface.clone()));

        let modifiers = match &self.keyboard {
            Some(keyboard_state) => keyboard_state.modifiers,
            None => Modifiers::default(),
        };

        window.touch_down(&mut self.engines, &surface, time, id, position.into(), modifiers);
    }

    fn up(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _touch: &WlTouch,
        _serial: u32,
        time: u32,
        id: i32,
    ) {
        let (window_id, surface) = match self.touch_focus.as_ref() {
            Some(focus) => focus,
            None => return,
        };
        let window = match self.windows.get_mut(window_id) {
            Some(window) => window,
            None => return,
        };

        let modifiers = match &self.keyboard {
            Some(keyboard_state) => keyboard_state.modifiers,
            None => Modifiers::default(),
        };

        window.touch_up(&mut self.engines, surface, time, id, modifiers);
    }

    fn motion(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _touch: &WlTouch,
        time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        let (window_id, surface) = match self.touch_focus.as_ref() {
            Some(focus) => focus,
            None => return,
        };
        let window = match self.windows.get_mut(window_id) {
            Some(window) => window,
            None => return,
        };

        let modifiers = match &self.keyboard {
            Some(keyboard_state) => keyboard_state.modifiers,
            None => Modifiers::default(),
        };

        window.touch_motion(&mut self.engines, surface, time, id, position.into(), modifiers);
    }

    fn cancel(&mut self, _connection: &Connection, _queue: &QueueHandle<Self>, _touch: &WlTouch) {}

    fn shape(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _touch: &WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _touch: &WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }
}
delegate_touch!(State);

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            // Find target window.
            let mut windows = self.windows.values();
            let window = match windows.find(|window| window.owns_surface(&event.surface)) {
                Some(window) => window,
                None => continue,
            };

            // Get shared event attributes.
            let position = event.position.into();
            let modifiers = match &self.keyboard {
                Some(keyboard_state) => keyboard_state.modifiers,
                None => Modifiers::default(),
            };

            // Dispatch event to the window.
            let engines = &mut self.engines;
            let surface = &event.surface;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Leave { .. } => (),
                PointerEventKind::Motion { time } => {
                    window.pointer_motion(engines, surface, time, position, modifiers)
                },
                PointerEventKind::Press { time, button, .. } => {
                    window.pointer_button(engines, surface, time, position, button, 1, modifiers)
                },
                PointerEventKind::Release { time, button, .. } => {
                    window.pointer_button(engines, surface, time, position, button, 0, modifiers)
                },
                PointerEventKind::Axis { time, horizontal, vertical, .. } => window.pointer_axis(
                    engines, surface, time, position, horizontal, vertical, modifiers,
                ),
            }
        }
    }
}
delegate_pointer!(State);

impl ProvidesRegistryState for State {
    registry_handlers![OutputState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.protocol_states.registry
    }
}
delegate_registry!(State);
