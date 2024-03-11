use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use funq::{MtQueueHandle, Queue, StQueueHandle};
use glib::source::SourceId;
use glib::{source, ControlFlow, IOCondition, MainLoop};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smithay_client_toolkit::reexports::client::globals::{self, GlobalError};
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{
    ConnectError, Connection, EventQueue, QueueHandle,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers, RepeatInfo};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use smithay_client_toolkit::shell::xdg::window::{Window as XdgWindow, WindowDecorations};
use smithay_client_toolkit::shell::WaylandSurface;

use crate::engine::webkit::{WebKitEngine, WebKitError};
use crate::engine::{Engine, EngineId};
use crate::ui::{Ui, UI_HEIGHT};
use crate::wayland::protocols::{KeyRepeat, ProtocolStates};
use crate::wayland::WaylandDispatch;

// Default window size.
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

mod engine;
mod ui;
mod wayland;

mod gl {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("{0}")]
    WaylandConnect(#[from] ConnectError),
    #[error("{0}")]
    Glutin(#[from] glutin::error::Error),
    #[error("{0}")]
    WaylandGlobal(#[from] GlobalError),
    #[error("{0}")]
    WebKit(#[from] WebKitError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

fn main() -> Result<(), Error> {
    let queue = Queue::new()?;
    let main_loop = MainLoop::new(None, true);
    let mut state = State::new(queue.local_handle(), main_loop.clone())?;

    // Create our initial window.
    state.create_window()?;

    // TODO: Temporary testing url input.
    let uri = std::env::args().nth(1).expect("USAGE: kumo <URI>");
    for engine in state.engines.values() {
        engine.load_uri(&uri);
    }

    // Register Wayland socket with GLib event loop.
    let mut queue_handle = queue.handle();
    let wayland_fd = state.connection.as_fd().as_raw_fd();
    source::unix_fd_add_local(wayland_fd, IOCondition::IN, move |_, _c| {
        queue_handle.wayland_dispatch();
        ControlFlow::Continue
    });

    // Register funq with GLib event loop.
    source::unix_fd_add_local(queue.fd().as_raw_fd(), IOCondition::IN, move |_, _| {
        let _ = queue.dispatch(&mut state);
        ControlFlow::Continue
    });

    // Run main event loop.
    main_loop.run();

    Ok(())
}

/// Main application state.
pub struct State {
    engines: HashMap<EngineId, Box<dyn Engine>>,
    main_loop: MainLoop,

    wayland_queue: Option<EventQueue<Self>>,
    protocol_states: ProtocolStates,
    connection: Connection,
    egl_display: Display,

    keyboard: Option<KeyboardState>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,

    windows: HashMap<WindowId, Window>,
    keyboard_focus: Option<WindowId>,
    touch_focus: Option<(WindowId, WlSurface)>,

    queue: StQueueHandle<Self>,
}

impl State {
    fn new(queue: StQueueHandle<Self>, main_loop: MainLoop) -> Result<Self, Error> {
        // Initialize Wayland connection.
        let connection = Connection::connect_to_env()?;
        let (globals, wayland_queue) = globals::registry_queue_init(&connection)?;
        let protocol_states = ProtocolStates::new(&globals, &wayland_queue.handle());

        // Get EGL display.
        let mut wayland_display = WaylandDisplayHandle::empty();
        wayland_display.display = connection.backend().display_ptr().cast();
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let egl_display = unsafe { Display::new(raw_display, DisplayApiPreference::Egl)? };

        // Setup OpenGL symbol loader.
        gl::load_with(|symbol| {
            let symbol = CString::new(symbol).unwrap();
            egl_display.get_proc_address(symbol.as_c_str()).cast()
        });

        Ok(Self {
            protocol_states,
            egl_display,
            connection,
            main_loop,
            queue,
            wayland_queue: Some(wayland_queue),
            keyboard_focus: Default::default(),
            touch_focus: Default::default(),
            keyboard: Default::default(),
            engines: Default::default(),
            windows: Default::default(),
            pointer: Default::default(),
            touch: Default::default(),
        })
    }

    /// Create a new browser window.
    fn create_window(&mut self) -> Result<(), WebKitError> {
        // Setup new window.
        let mut window =
            Window::new(&self.protocol_states, &self.egl_display, &self.wayland_queue());

        // Add initial tab.
        let engine_id = self.create_engine(window.id)?;
        window.add_tab(engine_id);

        self.windows.insert(window.id, window);

        // Ensure Wayland processing is kicked off.
        self.wayland_dispatch();

        Ok(())
    }

    /// Create a new WebKit browser engine.
    fn create_engine(&mut self, window_id: WindowId) -> Result<EngineId, WebKitError> {
        let engine_id = EngineId::new(window_id);
        let engine = WebKitEngine::new(
            &self.egl_display,
            self.queue.clone(),
            engine_id,
            DEFAULT_WIDTH,
            DEFAULT_HEIGHT,
        )?;
        self.engines.insert(engine_id, Box::new(engine));
        Ok(engine_id)
    }

    /// Get access to the Wayland queue.
    fn wayland_queue(&self) -> QueueHandle<Self> {
        self.wayland_queue.as_ref().unwrap().handle()
    }
}

/// Wayland window.
struct Window {
    id: WindowId,

    tabs: Vec<EngineId>,
    active_tab: usize,

    xdg: XdgWindow,
    viewport: WpViewport,
    scale: f64,
    width: u32,
    height: u32,
    stalled: bool,

    ui: Ui,

    // Touch point position tracking.
    touch_points: HashMap<i32, (f64, f64)>,
}

impl Window {
    fn new(
        protocol_states: &ProtocolStates,
        display: &Display,
        queue: &QueueHandle<State>,
    ) -> Self {
        let surface = protocol_states.compositor.create_surface(queue);

        // Enable fractional scaling.
        protocol_states.fractional_scale.fractional_scaling(queue, &surface);

        // Enable viewporter for the browser surface.
        let viewport = protocol_states.viewporter.viewport(queue, &surface);

        // Create UI renderer.
        let ui_surface = protocol_states.subcompositor.create_subsurface(surface.clone(), queue);
        let ui_viewport = protocol_states.viewporter.viewport(queue, &ui_surface.1);
        let ui = Ui::new(display, ui_surface, ui_viewport);

        // Create XDG window.
        let decorations = WindowDecorations::RequestServer;
        let xdg = protocol_states.xdg_shell.create_window(surface, decorations, queue);
        xdg.set_title("Kumo");
        xdg.set_app_id("Kumo");
        xdg.commit();

        Self {
            viewport,
            xdg,
            ui,
            height: DEFAULT_HEIGHT,
            width: DEFAULT_WIDTH,
            id: WindowId::new(),
            stalled: true,
            scale: 1.,
            touch_points: Default::default(),
            active_tab: Default::default(),
            tabs: Default::default(),
        }
    }

    /// Add a new tab to this window.
    fn add_tab(&mut self, engine_id: EngineId) {
        self.tabs.push(engine_id);
    }

    /// Redraw the window.
    fn draw(
        &mut self,
        wayland_queue: &QueueHandle<State>,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
    ) {
        let surface = self.xdg.wl_surface();

        // Mark window as stalled if no rendering is performed.
        self.stalled = true;

        // Redraw the active browser engine.
        let engine = engines.get_mut(&self.active_tab()).unwrap();
        if let Some(buffer) = engine.wl_buffer().filter(|_| engine.dirty()) {
            // Attach engine buffer to primary surface.
            surface.attach(Some(buffer), 0, 0);
            surface.damage(0, 0, self.width as i32, (self.height - UI_HEIGHT) as i32);

            // Request new engine frame.
            engine.frame_done();

            self.stalled = false;
        }

        // Attach new UI buffers.
        let ui_rendered = self.ui.draw();
        self.stalled &= !ui_rendered;

        // Request a new frame if this frame was dirty.
        if !self.stalled {
            surface.frame(wayland_queue, surface.clone());
        }

        // Submit the new frame.
        surface.commit();
    }

    /// Unstall the renderer.
    ///
    /// This will render a new frame if there currently is no frame request
    /// pending.
    fn unstall(
        &mut self,
        connection: &Connection,
        wayland_queue: &QueueHandle<State>,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
    ) {
        // Ignore while rendering is performed every frame.
        if !self.stalled {
            return;
        }

        // Redraw immediately to unstall rendering.
        self.draw(wayland_queue, engines);
        let _ = connection.flush();
    }

    /// Update surface size.
    fn set_size(
        &mut self,
        display: &Display,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        width: u32,
        height: u32,
    ) {
        // Update window dimensions.
        self.width = width;
        self.height = height;

        // Resize window's browser engines.
        for engine_id in &mut self.tabs {
            let engine = match engines.get_mut(engine_id) {
                Some(engine) => engine,
                None => continue,
            };

            let engine_height = self.height - UI_HEIGHT;
            engine.set_size(self.width, engine_height);

            // Update browser's viewporter logical render size.
            self.viewport.set_destination(self.width as i32, engine_height as i32);
        }

        // Resize UI element surface.
        self.ui.set_geometry(display, 0, (self.height - UI_HEIGHT) as i32, self.width, UI_HEIGHT);
    }

    /// Update surface scale.
    fn set_scale(&mut self, engines: &mut HashMap<EngineId, Box<dyn Engine>>, scale: f64) {
        // Update window scale.
        self.scale = scale;

        // Resize window's browser engines.
        for engine_id in &mut self.tabs {
            let engine = match engines.get_mut(engine_id) {
                Some(engine) => engine,
                None => continue,
            };
            engine.set_scale(scale);
        }

        // Resize UI.
        self.ui.set_scale(scale);
    }

    /// Handle new key press.
    fn press_key(
        &self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        raw: u32,
        keysym: Keysym,
        modifiers: Modifiers,
    ) {
        let engine = match engines.get_mut(&self.active_tab()) {
            Some(engine) => engine,
            None => return,
        };
        engine.press_key(raw, keysym, modifiers);
    }

    /// Handle new key release.
    fn release_key(
        &self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        raw: u32,
        keysym: Keysym,
        modifiers: Modifiers,
    ) {
        // Forward keyboard event to browser engine.
        if let Some(engine) = engines.get_mut(&self.active_tab()) {
            engine.release_key(raw, keysym, modifiers);
        }
    }

    /// Handle scroll axis events.
    #[allow(clippy::too_many_arguments)]
    fn pointer_axis(
        &self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        x: f64,
        y: f64,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        modifiers: Modifiers,
    ) {
        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.pointer_axis(time, x, y, horizontal, vertical, modifiers);
            }
        } else {
            self.ui.pointer_axis(time, x, y, horizontal, vertical, modifiers);
        }
    }

    /// Handle pointer button events.
    #[allow(clippy::too_many_arguments)]
    fn pointer_button(
        &self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        x: f64,
        y: f64,
        button: u32,
        state: u32,
        modifiers: Modifiers,
    ) {
        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.pointer_button(time, x, y, button, state, modifiers);
            }
        } else {
            println!("CLICK ON UI");
            self.ui.pointer_button(time, x, y, button, state, modifiers);
        }
    }

    /// Handle pointer motion events.
    fn pointer_motion(
        &self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    ) {
        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.pointer_motion(time, x, y, modifiers);
            }
        } else {
            self.ui.pointer_motion(time, x, y, modifiers);
        }
    }

    /// Handle touch press events.
    #[allow(clippy::too_many_arguments)]
    fn touch_down(
        &mut self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        id: i32,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    ) {
        self.touch_points.insert(id, (x, y));

        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.touch_down(&self.touch_points, time, id, modifiers);
            }
        } else {
            self.ui.touch_down(time, id, x, y, modifiers);
        }
    }

    /// Handle touch release events.
    fn touch_up(
        &mut self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        id: i32,
        modifiers: Modifiers,
    ) {
        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.touch_up(&self.touch_points, time, id, modifiers);
            }
        } else {
            self.ui.touch_up(time, id, modifiers);
        }

        // Remove touch point from all future events.
        self.touch_points.remove(&id);
    }

    /// Handle touch motion events.
    #[allow(clippy::too_many_arguments)]
    fn touch_motion(
        &mut self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        surface: &WlSurface,
        time: u32,
        id: i32,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    ) {
        self.touch_points.insert(id, (x, y));

        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = engines.get_mut(&self.active_tab()) {
                engine.touch_motion(&self.touch_points, time, id, modifiers);
            }
        } else {
            self.ui.touch_motion(time, id, x, y, modifiers);
        }
    }

    /// Check whether a surface is owned by this window.
    fn owns_surface(&self, surface: &WlSurface) -> bool {
        self.xdg.wl_surface() == surface || self.ui.owns_surface(surface)
    }

    /// Get the engine ID for the current tab.
    fn active_tab(&self) -> EngineId {
        self.tabs[self.active_tab]
    }
}

/// Key status tracking for WlKeyboard.
pub struct KeyboardState {
    wl_keyboard: WlKeyboard,
    repeat_info: RepeatInfo,
    modifiers: Modifiers,

    queue: MtQueueHandle<State>,
    current_repeat: Option<(SourceId, u32, Keysym)>,
}

impl Drop for KeyboardState {
    fn drop(&mut self) {
        self.wl_keyboard.release();
    }
}

impl KeyboardState {
    pub fn new(queue: MtQueueHandle<State>, wl_keyboard: WlKeyboard) -> Self {
        Self {
            wl_keyboard,
            queue,
            repeat_info: RepeatInfo::Disable,
            current_repeat: Default::default(),
            modifiers: Default::default(),
        }
    }

    /// Handle new key press.
    fn press_key(&mut self, raw: u32, keysym: Keysym) {
        // Update key repeat timers.
        if !keysym.is_modifier_key() {
            self.request_repeat(raw, keysym, true);
        }
    }

    /// Handle new key release.
    fn release_key(&mut self, raw: u32) {
        // Cancel repetition if released key is being repeated.
        if self.current_repeat.as_ref().map_or(false, |repeat| repeat.1 == raw) {
            self.cancel_repeat();
        }
    }

    /// Stage new key repetition.
    fn request_repeat(&mut self, raw: u32, keysym: Keysym, initial: bool) {
        // Ensure all previous events are cleared.
        self.cancel_repeat();

        let (delay, rate) = match self.repeat_info {
            RepeatInfo::Repeat { delay, rate } => (delay, rate),
            _ => return,
        };

        // Stage new timer.
        let mut queue = self.queue.clone();
        let delay = if initial {
            Duration::from_millis(delay as u64)
        } else {
            Duration::from_millis(1000 / rate.get() as u64)
        };
        let source_id = source::timeout_add_once(delay, move || queue.repeat_key());

        self.current_repeat = Some((source_id, raw, keysym));
    }

    /// Cancel currently staged key repetition.
    fn cancel_repeat(&mut self) {
        if let Some((source_id, ..)) = self.current_repeat.take() {
            source_id.remove();
        }
    }

    /// Get last pressed key for repetition.
    fn repeat_key(&self) -> Option<(u32, Keysym, Modifiers)> {
        let (_, raw, keysym) = self.current_repeat.as_ref()?;
        Some((*raw, *keysym, self.modifiers))
    }
}

/// Unique identifier for one window.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(usize);

impl WindowId {
    pub fn new() -> Self {
        static NEXT_WINDOW_ID: AtomicUsize = AtomicUsize::new(0);
        Self(NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for WindowId {
    fn default() -> Self {
        Self::new()
    }
}
