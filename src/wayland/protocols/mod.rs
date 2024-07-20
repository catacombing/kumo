//! Wayland protocol handling.

use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use _text_input::zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3};
use _text_input::zwp_text_input_v3::{self, ZwpTextInputV3};
use glib::{source, ControlFlow, Priority};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
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
use wayland_backend::client::{Backend, ObjectData, ObjectId};
use wayland_backend::protocol::Message;

use crate::wayland::protocols::fractional_scale::{FractionalScaleHandler, FractionalScaleManager};
use crate::wayland::protocols::viewporter::Viewporter;
use crate::window::WindowHandler as _;
use crate::{KeyboardState, State};

pub mod fractional_scale;
pub mod viewporter;

#[derive(Debug)]
pub struct ProtocolStates {
    pub fractional_scale: FractionalScaleManager,
    pub subcompositor: SubcompositorState,
    pub compositor: CompositorState,
    pub viewporter: Viewporter,
    pub xdg_shell: XdgShell,

    text_input: TextInputManager,
    registry: RegistryState,
    output: OutputState,
    seat: SeatState,
}

impl ProtocolStates {
    pub fn new(globals: &GlobalList, queue: &QueueHandle<State>) -> Self {
        let text_input = TextInputManager::new(globals, queue);
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
            text_input,
            xdg_shell,
            registry,
            output,
            seat,
        }
    }
}

impl CompositorHandler for State {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn frame(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        surface: &WlSurface,
        _serial: u32,
    ) {
        let window = self.windows.values_mut().find(|window| window.owns_surface(surface));
        if let Some(window) = window {
            window.draw();
        }
    }

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

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }

    /// The surface has left an output.
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
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
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        let window = self.windows.values_mut().find(|w| w.xdg() == window);
        if let Some(window) = window {
            let window_id = window.id();
            self.close_window(window_id);
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn configure(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let Some(window) = self.windows.values_mut().find(|w| w.xdg() == window) {
            window.configure(configure);
        }
    }
}
delegate_xdg_shell!(State);
delegate_xdg_window!(State);

impl FractionalScaleHandler for State {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        surface: &WlSurface,
        scale: f64,
    ) {
        let window = self.windows.values_mut().find(|w| w.owns_surface(surface));
        if let Some(window) = window {
            window.set_scale(scale);
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.protocol_states.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    #[cfg_attr(feature = "profiling", profiling::function)]
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

                // Add new IME handler for this seat.
                self.text_input.push(self.protocol_states.text_input.text_input(queue, seat));
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

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn remove_capability(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                self.keyboard = None;

                // Remove IME handler for this seat.
                self.text_input.retain(|text_input| text_input.seat != seat);
            },
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
    #[cfg_attr(feature = "profiling", profiling::function)]
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
        self.keyboard_focus = Some(window.id());
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
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

    #[cfg_attr(feature = "profiling", profiling::function)]
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
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        window.press_key(event.raw_code, event.keysym, keyboard_state.modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
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
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        let modifiers = keyboard_state.modifiers;
        window.release_key(event.raw_code, event.keysym, modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn update_modifiers(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        let keyboard_state = match &mut self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };

        // Update pressed modifiers.
        keyboard_state.modifiers = modifiers;
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
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
    /// Start key repetition.
    fn repeat_key(&mut self, raw: u32, keysym: Keysym, rate: u64);

    /// Send a key press to the focused window.
    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers);
}

impl KeyRepeat for State {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn repeat_key(&mut self, raw: u32, keysym: Keysym, rate: u64) {
        let keyboard_state = match &self.keyboard {
            Some(keyboard_state) => keyboard_state,
            None => return,
        };

        // Send the initial key press.
        let modifiers = keyboard_state.modifiers;
        KeyRepeat::press_key(self, raw, keysym, modifiers);

        // Keep repeating the key until repetition is cancelled.
        let mut queue = self.queue.handle();
        let interval = Duration::from_millis(1000 / rate);
        let source = source::timeout_source_new(interval, None, Priority::DEFAULT, move || {
            queue.press_key(raw, keysym, modifiers);
            ControlFlow::Continue
        });
        source.attach(None);

        // Update the repeat source and clear the initial GLib delay source ID in the
        // process, since calling `destroy` on a dead source causes a panic.
        let keyboard_state = self.keyboard.as_mut().unwrap();
        keyboard_state.current_repeat = Some((source, raw));
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(window) = self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            window.press_key(raw, keysym, modifiers);
        }
    }
}

impl TouchHandler for State {
    #[cfg_attr(feature = "profiling", profiling::function)]
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
        self.touch_focus = Some((window.id(), surface.clone()));

        let modifiers = match &self.keyboard {
            Some(keyboard_state) => keyboard_state.modifiers,
            None => Modifiers::default(),
        };

        window.touch_down(&surface, time, id, position.into(), modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
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

        window.touch_up(surface, time, id, modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
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

        window.touch_motion(surface, time, id, position.into(), modifiers);
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
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn pointer_frame(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            // Find target window.
            let mut windows = self.windows.values_mut();
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
            let surface = &event.surface;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Leave { .. } => (),
                PointerEventKind::Motion { time } => {
                    window.pointer_motion(surface, time, position, modifiers)
                },
                PointerEventKind::Press { time, button, .. } => {
                    window.pointer_button(surface, time, position, button, 1, modifiers)
                },
                PointerEventKind::Release { time, button, .. } => {
                    window.pointer_button(surface, time, position, button, 0, modifiers)
                },
                PointerEventKind::Axis { time, horizontal, vertical, .. } => {
                    window.pointer_axis(surface, time, position, horizontal, vertical, modifiers)
                },
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

/// zwp_text_input_v3 protocol implementation.
impl State {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn text_input_enter(&mut self, text_input: ZwpTextInputV3, surface: &WlSurface) {
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };

        window.text_input_enter(text_input);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn text_input_leave(&mut self, surface: &WlSurface) {
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };

        window.text_input_leave();
    }

    /// Delete text around the current cursor position.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn delete_surrounding_text(
        &mut self,
        surface: &WlSurface,
        before_length: u32,
        after_length: u32,
    ) {
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };

        window.delete_surrounding_text(before_length, after_length);
    }

    /// Insert text at the current cursor position.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn commit_string(&mut self, surface: &WlSurface, text: String) {
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };

        window.commit_string(text);
    }

    /// Set preedit text at the current cursor position.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn preedit_string(
        &mut self,
        surface: &WlSurface,
        text: String,
        cursor_begin: i32,
        cursor_end: i32,
    ) {
        let window = match self.windows.values_mut().find(|window| window.owns_surface(surface)) {
            Some(window) => window,
            None => return,
        };

        window.preedit_string(text, cursor_begin, cursor_end);
    }
}

/// Factory for the zwp_text_input_v3 protocol.
#[derive(Debug)]
struct TextInputManager {
    manager: ZwpTextInputManagerV3,
}

impl TextInputManager {
    fn new(globals: &GlobalList, queue: &QueueHandle<State>) -> Self {
        let manager = globals.bind(queue, 1..=1, ()).unwrap();
        Self { manager }
    }

    /// Get a new text input handle.
    fn text_input(&self, queue: &QueueHandle<State>, seat: WlSeat) -> TextInput {
        let _text_input = self.manager.get_text_input(&seat, queue, Default::default());
        TextInput { _text_input, seat }
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for State {
    fn event(
        _state: &mut State,
        _input_manager: &ZwpTextInputManagerV3,
        _event: zwp_text_input_manager_v3::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<State>,
    ) {
        // No events.
    }
}

/// State for the zwp_text_input_v3 protocol.
#[derive(Default)]
struct TextInputState {
    surface: Option<WlSurface>,
    preedit_string: Option<(String, i32, i32)>,
    commit_string: Option<String>,
    delete_surrounding_text: Option<(u32, u32)>,
}

/// Interface for the zwp_text_input_v3 protocol.
pub struct TextInput {
    _text_input: ZwpTextInputV3,
    seat: WlSeat,
}

impl Dispatch<ZwpTextInputV3, Arc<Mutex<TextInputState>>> for State {
    fn event(
        state: &mut State,
        text_input: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        data: &Arc<Mutex<TextInputState>>,
        _connection: &Connection,
        _queue: &QueueHandle<State>,
    ) {
        let mut data = data.lock().unwrap();
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                state.text_input_enter(text_input.clone(), &surface);
                data.surface = Some(surface);
            },
            zwp_text_input_v3::Event::Leave { surface } => {
                if data.surface.as_ref() == Some(&surface) {
                    state.text_input_leave(&surface);
                    data.surface = None;
                }
            },
            zwp_text_input_v3::Event::PreeditString { text, cursor_begin, cursor_end } => {
                data.preedit_string = Some((text.unwrap_or_default(), cursor_begin, cursor_end));
            },
            zwp_text_input_v3::Event::CommitString { text } => {
                data.commit_string = Some(text.unwrap_or_default());
            },
            zwp_text_input_v3::Event::DeleteSurroundingText { before_length, after_length } => {
                data.delete_surrounding_text = Some((before_length, after_length));
            },
            zwp_text_input_v3::Event::Done { .. } => {
                let preedit_string = data.preedit_string.take().unwrap_or_default();
                let delete_surrounding_text = data.delete_surrounding_text.take();
                let commit_string = data.commit_string.take();

                let surface = match &data.surface {
                    Some(surface) => surface,
                    None => return,
                };

                if let Some((before_length, after_length)) = delete_surrounding_text {
                    state.delete_surrounding_text(surface, before_length, after_length);
                }
                if let Some(text) = commit_string {
                    state.commit_string(surface, text);
                }
                let (text, cursor_begin, cursor_end) = preedit_string;
                state.preedit_string(surface, text, cursor_begin, cursor_end);
            },
            _ => unreachable!(),
        }
    }
}

/// Foreign WlBuffer object data.
pub struct BufferData {
    connection: Connection,
}

impl BufferData {
    pub fn new(connection: Connection) -> Arc<Self> {
        Arc::new(Self { connection })
    }
}

impl ObjectData for BufferData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        msg: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        // Destroy buffer on release.
        if msg.opcode == 0 {
            if let Ok(buffer) = WlBuffer::from_id(&self.connection, msg.sender_id) {
                buffer.destroy();
            }
        }

        None
    }

    fn destroyed(&self, _object_id: ObjectId) {}
}
