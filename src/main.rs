use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicUsize, Ordering};

use funq::{Queue, StQueueHandle};
use glib::{source, ControlFlow, IOCondition, MainLoop};
use glutin::display::{Display, DisplayApiPreference};
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smithay_client_toolkit::reexports::client::globals::{self, GlobalError};
use smithay_client_toolkit::reexports::client::{
    ConnectError, Connection, EventQueue, QueueHandle,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::shell::xdg::window::{Window as XdgWindow, WindowDecorations};
use smithay_client_toolkit::shell::WaylandSurface;

use crate::engine::webkit::{WebKitEngine, WebKitError};
use crate::engine::{Engine, EngineId};
use crate::wayland::protocols::ProtocolStates;
use crate::wayland::WaylandDispatch;

// Default window size.
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

mod engine;
mod wayland;

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
    let mut state = State::new(queue.local_handle())?;

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
    MainLoop::new(None, true).run();

    Ok(())
}

/// Main application state.
pub struct State {
    engines: HashMap<EngineId, Box<dyn Engine>>,
    terminated: bool,

    wayland_queue: Option<EventQueue<Self>>,
    protocol_states: ProtocolStates,
    connection: Connection,
    egl_display: Display,

    windows: HashMap<WindowId, Window>,

    local_queue: StQueueHandle<Self>,
}

impl State {
    fn new(local_queue: StQueueHandle<Self>) -> Result<Self, Error> {
        // Initialize Wayland connection.
        let connection = Connection::connect_to_env()?;
        let (globals, wayland_queue) = globals::registry_queue_init(&connection)?;
        let protocol_states = ProtocolStates::new(&globals, &wayland_queue.handle());

        // Get EGL display.
        let mut wayland_display = WaylandDisplayHandle::empty();
        wayland_display.display = connection.backend().display_ptr().cast();
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let egl_display = unsafe { Display::new(raw_display, DisplayApiPreference::Egl)? };

        Ok(Self {
            protocol_states,
            local_queue,
            egl_display,
            connection,
            wayland_queue: Some(wayland_queue),
            terminated: Default::default(),
            engines: Default::default(),
            windows: Default::default(),
        })
    }

    /// Create a new browser window.
    fn create_window(&mut self) -> Result<(), WebKitError> {
        // Setup new window.
        let mut window = Window::new(&self.protocol_states, &self.wayland_queue());

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
            self.local_queue.clone(),
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
    initial_commit_done: bool,

    dirty: bool,
}

impl Window {
    fn new(protocol_states: &ProtocolStates, queue: &QueueHandle<State>) -> Self {
        let surface = protocol_states.compositor.create_surface(queue);

        // Enable fractional scaling.
        protocol_states.fractional_scale.fractional_scaling(queue, &surface);

        // Enable viewporter.
        let viewport = protocol_states.viewporter.viewport(queue, &surface);

        // Create XDG window.
        let decorations = WindowDecorations::RequestServer;
        let xdg = protocol_states.xdg_shell.create_window(surface, decorations, queue);
        xdg.set_title("Kumo");
        xdg.set_app_id("Kumo");
        xdg.commit();

        let id = WindowId::new();

        Self {
            viewport,
            xdg,
            id,
            height: DEFAULT_HEIGHT,
            width: DEFAULT_WIDTH,
            scale: 1.,
            initial_commit_done: Default::default(),
            active_tab: Default::default(),
            dirty: Default::default(),
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
        engines: &HashMap<EngineId, Box<dyn Engine>>,
    ) {
        if !self.dirty {
            let surface = self.xdg.wl_surface();
            surface.frame(wayland_queue, surface.clone());
            surface.commit();
            return;
        }

        let engine = engines.get(&self.active_tab()).unwrap();
        let buffer = match engine.wl_buffer() {
            Some(buffer) => buffer,
            None => return,
        };

        self.dirty = false;

        // Submit a new frame.
        let surface = self.xdg.wl_surface();
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, self.width as i32, self.height as i32);
        surface.frame(wayland_queue, surface.clone());
        surface.commit();

        // Request next frame from engine.
        engine.frame_done();
    }

    /// Notify window about an engine's buffer update.
    fn mark_engine_dirty(
        &mut self,
        connection: &Connection,
        wayland_queue: &QueueHandle<State>,
        engines: &HashMap<EngineId, Box<dyn Engine>>,
        engine_id: EngineId,
    ) {
        // Mark window as dirty if active tab changed.
        if engine_id == self.active_tab() {
            self.dirty = true;
        }

        // Perform initial draw if necessary.
        if !self.initial_commit_done {
            self.draw(wayland_queue, engines);
            self.initial_commit_done = true;
            let _ = connection.flush();
        }
    }

    /// Update surface size.
    fn set_size(
        &mut self,
        engines: &mut HashMap<EngineId, Box<dyn Engine>>,
        width: u32,
        height: u32,
    ) {
        // Update window dimensions.
        self.width = width;
        self.height = height;

        // Update viewporter logical render size.
        self.viewport.set_destination(self.width as i32, self.height as i32);

        // Resize window's browser engines.
        for engine_id in &mut self.tabs {
            let engine = match engines.get_mut(engine_id) {
                Some(engine) => engine,
                None => continue,
            };
            engine.set_size(self.width, self.height);
        }
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
    }

    /// Get the engine ID for the current tab.
    fn active_tab(&self) -> EngineId {
        self.tabs[self.active_tab]
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
