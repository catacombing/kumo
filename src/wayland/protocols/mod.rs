//! Wayland protocol handling.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use _dmabuf::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1;
use _dmabuf::zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1;
use _spb::wp_single_pixel_buffer_manager_v1::{self, WpSinglePixelBufferManagerV1};
use _text_input::zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3};
use _text_input::zwp_text_input_v3::{self, ZwpTextInputV3};
use glib::{ControlFlow, Priority, source};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::dmabuf::{DmabufFeedback, DmabufHandler, DmabufState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::{self, WlBuffer};
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::WlDataDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::linux_dmabuf::zv1::client as _dmabuf;
use smithay_client_toolkit::reexports::protocols::wp::single_pixel_buffer::v1::client as _spb;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers, RepeatInfo,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::subcompositor::SubcompositorState;
use smithay_client_toolkit::{
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_keyboard, delegate_output,
    delegate_pointer, delegate_registry, delegate_seat, delegate_subcompositor, delegate_touch,
    delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use crate::wayland::protocols::fractional_scale::{FractionalScaleHandler, FractionalScaleManager};
use crate::wayland::protocols::viewporter::Viewporter;
use crate::window::WindowHandler as _;
use crate::{CurrentRepeat, Error, KeyboardState, State};

pub mod fractional_scale;
pub mod viewporter;

#[derive(Debug)]
pub struct ProtocolStates {
    pub single_pixel_buffer: Option<WpSinglePixelBufferManagerV1>,
    pub fractional_scale: Option<FractionalScaleManager>,
    pub data_device_manager: DataDeviceManagerState,
    pub subcompositor: SubcompositorState,
    pub compositor: CompositorState,
    pub data_device: DataDevice,
    pub viewporter: Viewporter,
    pub xdg_shell: XdgShell,
    pub dmabuf: DmabufState,

    text_input: TextInputManager,
    registry: RegistryState,
    output: OutputState,
    seat: SeatState,
}

impl ProtocolStates {
    pub fn new(globals: &GlobalList, queue: &QueueHandle<State>) -> Result<Self, Error> {
        // SPB is optional for rendering the engine backdrop.
        let single_pixel_buffer = globals.bind(queue, 1..=1, ()).ok();
        let text_input = TextInputManager::new(globals, queue);
        let registry = RegistryState::new(globals);
        let compositor = CompositorState::bind(globals, queue)
            .map_err(|err| Error::WaylandProtocol("wl_compositor", err))?;
        let wl_compositor = compositor.wl_compositor().clone();
        let fractional_scale = FractionalScaleManager::new(globals, queue).ok();
        let subcompositor = SubcompositorState::bind(wl_compositor, globals, queue)
            .map_err(|err| Error::WaylandProtocol("wl_subcompositor", err))?;
        let viewporter = Viewporter::new(globals, queue)
            .map_err(|err| Error::WaylandProtocol("wp_viewporter", err))?;
        let xdg_shell = XdgShell::bind(globals, queue)
            .map_err(|err| Error::WaylandProtocol("xdg_shell", err))?;
        let dmabuf = DmabufState::new(globals, queue);
        let output = OutputState::new(globals, queue);
        let seat = SeatState::new(globals, queue);
        let data_device_manager = DataDeviceManagerState::bind(globals, queue)
            .map_err(|err| Error::WaylandProtocol("wl_data_device_manager", err))?;

        // Get data device for the default seat.
        let default_seat = seat.seats().next().unwrap();
        let data_device = data_device_manager.get_data_device(queue, &default_seat);

        // Immediately request default DMA buffer feedback.
        let _ = dmabuf.get_default_feedback(queue);

        Ok(Self {
            single_pixel_buffer,
            data_device_manager,
            fractional_scale,
            subcompositor,
            data_device,
            compositor,
            viewporter,
            text_input,
            xdg_shell,
            registry,
            dmabuf,
            output,
            seat,
        })
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
        surface: &WlSurface,
        scale: i32,
    ) {
        if self.protocol_states.fractional_scale.is_some() {
            return;
        }

        let window = self.windows.values_mut().find(|w| w.owns_surface(surface));
        if let Some(window) = window {
            window.set_scale(scale as f64);
        }
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
        keyboard_state.press_key(event.time, event.raw_code, event.keysym);

        // Update pressed keys.
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        window.press_key(event.time, event.raw_code, event.keysym, keyboard_state.modifiers);
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
        window.release_key(event.time, event.raw_code, event.keysym, modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn repeat_key(
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
        let window = match self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            Some(focus) => focus,
            None => return,
        };
        window.press_key(event.time, event.raw_code, event.keysym, keyboard_state.modifiers);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn update_modifiers(
        &mut self,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
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
        let modifiers = match &self.keyboard {
            Some(KeyboardState { modifiers, .. }) => *modifiers,
            None => return,
        };

        // Send the initial key press.
        KeyRepeat::press_key(self, raw, keysym, modifiers);

        // Keep repeating the key until repetition is cancelled.
        let mut queue = self.queue.handle();
        let interval_ms = 1000 / rate;
        let interval = Duration::from_millis(interval_ms);
        let source = source::timeout_source_new(interval, None, Priority::DEFAULT, move || {
            queue.press_key(raw, keysym, modifiers);
            ControlFlow::Continue
        });
        source.attach(None);

        // Update the repeat source and clear the initial GLib delay source ID in the
        // process, since calling `destroy` on a dead source causes a panic.
        if let Some(current_repeat) = &mut self.keyboard.as_mut().unwrap().current_repeat {
            *current_repeat =
                CurrentRepeat::new(source, raw, current_repeat.time, interval_ms as u32);
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Get timestamp for the key event.
        let time =
            match self.keyboard.as_mut().and_then(|keyboard| keyboard.current_repeat.as_mut()) {
                Some(current_repeat) => current_repeat.next_time(),
                None => return,
            };

        if let Some(window) = self.keyboard_focus.and_then(|focus| self.windows.get_mut(&focus)) {
            window.press_key(time, raw, keysym, modifiers);
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
                PointerEventKind::Enter { .. } => {
                    window.pointer_enter(surface, position, modifiers)
                },
                PointerEventKind::Leave { .. } => {
                    window.pointer_leave(surface, position, modifiers)
                },
                PointerEventKind::Motion { time } => {
                    window.pointer_motion(surface, time, position, modifiers)
                },
                PointerEventKind::Press { time, button, .. } => {
                    window.pointer_button(surface, time, position, button, true, modifiers)
                },
                PointerEventKind::Release { time, button, .. } => {
                    window.pointer_button(surface, time, position, button, false, modifiers)
                },
                PointerEventKind::Axis { time, horizontal, vertical, .. } => {
                    window.pointer_axis(surface, time, position, horizontal, vertical, modifiers)
                },
            }
        }
    }
}
delegate_pointer!(State);

impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}
impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        mut pipe: WritePipe,
    ) {
        let _ = pipe.write_all(self.clipboard.text.as_bytes());
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}
impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}
delegate_data_device!(State);

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.protocol_states.dmabuf
    }

    fn dmabuf_feedback(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpLinuxDmabufFeedbackV1,
        feedback: DmabufFeedback,
    ) {
        // Notify windows, to update their engines.
        for window in self.windows.values_mut() {
            window.dmabuf_feedback_changed(&feedback);
        }

        // Update globally shared feedback.
        self.engine_state.borrow_mut().dmabuf_feedback.replace(Some(feedback));
    }

    fn created(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpLinuxBufferParamsV1,
        _: WlBuffer,
    ) {
    }

    fn failed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpLinuxBufferParamsV1) {}

    fn released(&mut self, _: &Connection, _: &QueueHandle<Self>, buffer: &WlBuffer) {
        for window in self.windows.values_mut() {
            window.buffer_released(buffer);
        }
    }
}
delegate_dmabuf!(State);

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

        window.set_preedit_string(text, cursor_begin, cursor_end);
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

impl Dispatch<WpSinglePixelBufferManagerV1, ()> for State {
    fn event(
        _state: &mut State,
        _manager: &WpSinglePixelBufferManagerV1,
        _event: wp_single_pixel_buffer_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<State>,
    ) {
        // No events.
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _state: &mut State,
        _buffer: &WlBuffer,
        event: wl_buffer::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<State>,
    ) {
        match event {
            // We never release our SPB buffers.
            wl_buffer::Event::Release => (),
            _ => unreachable!(),
        }
    }
}
