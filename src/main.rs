use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};
use std::os::fd::{AsFd, AsRawFd};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;
use std::{env, io, mem};

use funq::{MtQueueHandle, Queue, StQueueHandle};
use glib::{source, ControlFlow, IOCondition, MainLoop, Priority, Source};
use glutin::display::{Display, DisplayApiPreference};
#[cfg(feature = "profiling")]
use profiling::puffin;
#[cfg(feature = "profiling")]
use puffin_http::Server;
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::globals::{self, GlobalError};
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{
    ConnectError, Connection, EventQueue, QueueHandle,
};
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers, RepeatInfo};
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::engine::webkit::WebKitError;
use crate::history::History;
use crate::wayland::protocols::{KeyRepeat, ProtocolStates, TextInput};
use crate::wayland::WaylandDispatch;
use crate::window::{KeyboardFocus, Window, WindowId};

mod engine;
mod history;
mod ui;
mod uri;
mod wayland;
mod window;

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
    // Setup logging.
    let directives = env::var("RUST_LOG").unwrap_or("warn,kumo=info".into());
    let env_filter = EnvFilter::builder().parse_lossy(directives);
    FmtSubscriber::builder().with_env_filter(env_filter).with_line_number(true).init();

    info!("Started Kumo");

    // Start profiling server.
    #[cfg(feature = "profiling")]
    let _server = {
        puffin::set_scopes_on(true);
        Server::new(&format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT)).unwrap()
    };

    let queue = Queue::new()?;
    let main_loop = MainLoop::new(None, true);
    let mut state = State::new(queue.local_handle(), main_loop.clone())?;

    // Create our initial window.
    let window_id = state.create_window()?;

    // Create an empty tab for loading a new page.
    let mut is_first_tab = true;
    let get_empty_tab = |window: &mut Window, is_first_tab: &mut bool| -> Result<_, Error> {
        if *is_first_tab {
            window.set_keyboard_focus(KeyboardFocus::None);
            *is_first_tab = false;
            Ok(window.active_tab())
        } else {
            Ok(window.add_tab(false)?)
        }
    };

    // Get all sessions requiring restoration, sorted by PID and window ID.
    let mut orphan_sessions = state.history.orphan_sessions();
    orphan_sessions.sort_by(|a, b| match a.pid.cmp(&b.pid) {
        Ordering::Equal => a.window_id.cmp(&b.window_id),
        ordering => ordering,
    });

    // Restore all orphan sessions.
    let mut window = state.windows.get_mut(&window_id).unwrap();
    let mut session_window_id = None;
    let mut session_pid = None;
    for entry in &mut orphan_sessions {
        // Create new window if session's process or window changed.
        if session_pid != Some(entry.pid) || session_window_id != Some(entry.window_id) {
            // Create a new window.
            if session_pid.is_some() || session_window_id.is_some() {
                let new_window_id = state.create_window()?;
                window = state.windows.get_mut(&new_window_id).unwrap();
                is_first_tab = true;
            }

            session_window_id = Some(entry.window_id);
            session_pid = Some(entry.pid);
        }

        // Restore the session in a new empty tab.
        let engine_id = get_empty_tab(window, &mut is_first_tab)?;
        if let Some(engine) = window.tabs().get(&engine_id) {
            engine.restore_session(mem::take(&mut entry.session_data));
            engine.load_uri(&entry.uri);
        }
    }

    // Update database with the adopted sessions.
    //
    // NOTE: Calling `load_uri` automatically causes the session to be persisted
    // once the page is loaded, however we explicitly sync here to avoid session
    // loss due to a racing condition.
    for window in state.windows.values() {
        window.persist_session(&state.history);
    }
    state.history.delete_orphan_sessions(orphan_sessions.iter().map(|s| s.pid));

    // Spawn a new tab for every CLI argument.
    let window = state.windows.get_mut(&window_id).unwrap();
    for arg in env::args().skip(1) {
        get_empty_tab(window, &mut is_first_tab)?;
        window.load_uri(arg);
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
    main_loop: MainLoop,

    wayland_queue: Option<EventQueue<Self>>,
    dmabuf_feedback: Rc<RefCell<Option<DmabufFeedback>>>,
    protocol_states: ProtocolStates,
    connection: Connection,
    egl_display: Display,

    text_input: Vec<TextInput>,
    keyboard: Option<KeyboardState>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,

    windows: HashMap<WindowId, Window>,
    keyboard_focus: Option<WindowId>,
    touch_focus: Option<(WindowId, WlSurface)>,

    history: History,

    queue: StQueueHandle<State>,
}

impl State {
    fn new(queue: StQueueHandle<Self>, main_loop: MainLoop) -> Result<Self, Error> {
        // Initialize Wayland connection.
        let connection = Connection::connect_to_env()?;
        let (globals, wayland_queue) = globals::registry_queue_init(&connection)?;
        let protocol_states = ProtocolStates::new(&globals, &wayland_queue.handle());

        // Get EGL display.
        let display = NonNull::new(connection.backend().display_ptr().cast()).unwrap();
        let wayland_display = WaylandDisplayHandle::new(display);
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let egl_display = unsafe { Display::new(raw_display, DisplayApiPreference::Egl)? };

        Ok(Self {
            protocol_states,
            egl_display,
            connection,
            main_loop,
            queue,
            wayland_queue: Some(wayland_queue),
            history: History::new(),
            dmabuf_feedback: Default::default(),
            keyboard_focus: Default::default(),
            touch_focus: Default::default(),
            text_input: Default::default(),
            keyboard: Default::default(),
            windows: Default::default(),
            pointer: Default::default(),
            touch: Default::default(),
        })
    }

    /// Create a new browser window.
    fn create_window(&mut self) -> Result<WindowId, WebKitError> {
        // Setup new window.
        let connection = self.connection.clone();
        let window = Window::new(
            &self.protocol_states,
            connection,
            self.egl_display.clone(),
            self.queue.clone(),
            self.wayland_queue(),
            self.history.clone(),
            self.dmabuf_feedback.clone(),
        )?;
        let window_id = window.id();
        self.windows.insert(window_id, window);

        // Ensure Wayland processing is kicked off.
        self.wayland_dispatch();

        Ok(window_id)
    }

    /// Get access to the Wayland queue.
    fn wayland_queue(&self) -> QueueHandle<Self> {
        self.wayland_queue.as_ref().unwrap().handle()
    }
}

/// Key status tracking for WlKeyboard.
pub struct KeyboardState {
    wl_keyboard: WlKeyboard,
    repeat_info: RepeatInfo,
    modifiers: Modifiers,

    queue: MtQueueHandle<State>,
    current_repeat: Option<CurrentRepeat>,
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
    fn press_key(&mut self, time: u32, raw: u32, keysym: Keysym) {
        // Update key repeat timers.
        if !keysym.is_modifier_key() {
            self.request_repeat(time, raw, keysym);
        }
    }

    /// Handle new key release.
    fn release_key(&mut self, raw: u32) {
        // Cancel repetition if released key is being repeated.
        if self.current_repeat.as_ref().map_or(false, |repeat| repeat.raw == raw) {
            self.cancel_repeat();
        }
    }

    /// Stage new key repetition.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn request_repeat(&mut self, time: u32, raw: u32, keysym: Keysym) {
        // Ensure all previous events are cleared.
        self.cancel_repeat();

        let (delay_ms, rate) = match self.repeat_info {
            RepeatInfo::Repeat { delay, rate } => (delay, rate),
            _ => return,
        };

        // Stage timer for initial delay.
        let mut queue = self.queue.clone();
        let delay = Duration::from_millis(delay_ms as u64);
        let delay_source = source::timeout_source_new(delay, None, Priority::DEFAULT, move || {
            queue.repeat_key(raw, keysym, rate.get() as u64);
            ControlFlow::Break
        });
        delay_source.attach(None);

        self.current_repeat = Some(CurrentRepeat::new(delay_source, raw, time, delay_ms));
    }

    /// Cancel currently staged key repetition.
    fn cancel_repeat(&mut self) {
        if let Some(CurrentRepeat { source, .. }) = self.current_repeat.take() {
            source.destroy();
        }
    }
}

/// Active keyboard repeat state.
pub struct CurrentRepeat {
    source: Source,
    interval: u32,
    time: u32,
    raw: u32,
}

impl CurrentRepeat {
    pub fn new(source: Source, raw: u32, time: u32, interval: u32) -> Self {
        Self { source, time, interval, raw }
    }

    /// Get the next key event timestamp.
    pub fn next_time(&mut self) -> u32 {
        self.time += self.interval;
        self.time
    }
}

/// 2D object position.
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Position<T = i32> {
    pub x: T,
    pub y: T,
}

impl<T> Position<T> {
    fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl<T> From<(T, T)> for Position<T> {
    fn from((x, y): (T, T)) -> Self {
        Self { x, y }
    }
}

impl From<Position> for Position<f64> {
    fn from(position: Position) -> Self {
        Self { x: position.x as f64, y: position.y as f64 }
    }
}

impl From<Position> for Position<f32> {
    fn from(position: Position) -> Self {
        Self { x: position.x as f32, y: position.y as f32 }
    }
}

impl From<Position<f64>> for Position<f32> {
    fn from(position: Position<f64>) -> Self {
        Self { x: position.x as f32, y: position.y as f32 }
    }
}

impl Mul<f64> for Position {
    type Output = Self;

    fn mul(mut self, scale: f64) -> Self {
        self.x = (self.x as f64 * scale).round() as i32;
        self.y = (self.y as f64 * scale).round() as i32;
        self
    }
}

impl Mul<f64> for Position<f64> {
    type Output = Self;

    fn mul(mut self, scale: f64) -> Self {
        self.x *= scale;
        self.y *= scale;
        self
    }
}

impl<T: Add<T, Output = T>> Add<Position<T>> for Position<T> {
    type Output = Self;

    fn add(mut self, rhs: Position<T>) -> Self {
        self.x = self.x + rhs.x;
        self.y = self.y + rhs.y;
        self
    }
}

impl<T: Add<T, Output = T> + Copy> AddAssign<Position<T>> for Position<T> {
    fn add_assign(&mut self, rhs: Position<T>) {
        *self = *self + rhs;
    }
}

impl<T: Sub<T, Output = T>> Sub<Position<T>> for Position<T> {
    type Output = Self;

    fn sub(mut self, rhs: Position<T>) -> Self {
        self.x = self.x - rhs.x;
        self.y = self.y - rhs.y;
        self
    }
}

impl<T: Sub<T, Output = T> + Copy> SubAssign<Position<T>> for Position<T> {
    fn sub_assign(&mut self, rhs: Position<T>) {
        *self = *self - rhs;
    }
}

/// 2D object size.
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Size<T = u32> {
    pub width: T,
    pub height: T,
}

impl<T> Size<T> {
    fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

impl<T> From<(T, T)> for Size<T> {
    fn from((width, height): (T, T)) -> Self {
        Self { width, height }
    }
}

impl From<Size<i32>> for Size<f32> {
    fn from(size: Size<i32>) -> Self {
        Self { width: size.width as f32, height: size.height as f32 }
    }
}

impl From<Size> for Size<i32> {
    fn from(size: Size) -> Self {
        Self { width: size.width as i32, height: size.height as i32 }
    }
}

impl From<Size> for Size<f64> {
    fn from(size: Size) -> Self {
        Self { width: size.width as f64, height: size.height as f64 }
    }
}

impl From<Size> for Size<f32> {
    fn from(size: Size) -> Self {
        Self { width: size.width as f32, height: size.height as f32 }
    }
}

impl Mul<f64> for Size {
    type Output = Self;

    fn mul(mut self, scale: f64) -> Self {
        self.width = (self.width as f64 * scale).round() as u32;
        self.height = (self.height as f64 * scale).round() as u32;
        self
    }
}

impl Mul<f64> for Size<f64> {
    type Output = Self;

    fn mul(mut self, scale: f64) -> Self {
        self.width *= scale;
        self.height *= scale;
        self
    }
}

impl<T: Sub<T, Output = T>> Sub<Size<T>> for Size<T> {
    type Output = Self;

    fn sub(mut self, rhs: Size<T>) -> Self {
        self.width = self.width - rhs.width;
        self.height = self.height - rhs.height;
        self
    }
}

impl<T: Sub<T, Output = T> + Copy> SubAssign<Size<T>> for Size<T> {
    fn sub_assign(&mut self, rhs: Size<T>) {
        *self = *self - rhs;
    }
}

/// Check if a rectangle contains a point.
pub fn rect_contains<T>(position: Position<T>, size: Size<T>, point: Position<T>) -> bool
where
    T: PartialOrd + Add<Output = T>,
{
    point.x >= position.x
        && point.y >= position.y
        && point.x < position.x + size.width
        && point.y < position.y + size.height
}
