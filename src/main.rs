use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};
use std::os::fd::{AsFd, AsRawFd};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{env, io, mem, process};

use clap::Parser;
use cli::{ConfigOptions, Options, Subcommands};
use configory::ipc::Ipc;
use configory::{Config as ConfigManager, Event as ConfigEvent};
use funq::{MtQueueHandle, Queue, StQueueHandle};
use glib::{ControlFlow, IOCondition, MainLoop, Priority, Source, source};
use glutin::display::{Display, DisplayApiPreference};
#[cfg(feature = "profiling")]
use profiling::puffin;
#[cfg(feature = "profiling")]
use puffin_http::Server;
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smithay_client_toolkit::data_device_manager::data_source::CopyPasteSource;
use smithay_client_toolkit::reexports::client::globals::{self, BindError, GlobalError};
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::protocol::wl_touch::WlTouch;
use smithay_client_toolkit::reexports::client::{
    ConnectError, Connection, EventQueue, QueueHandle,
};
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers, RepeatInfo};
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::config::{CONFIG, Config};
use crate::engine::webkit::{WebKitError, WebKitState};
use crate::engine::{Group, GroupId, NO_GROUP_ID};
use crate::storage::Storage;
use crate::storage::history::History;
use crate::wayland::WaylandDispatch;
use crate::wayland::protocols::{KeyRepeat, ProtocolStates, TextInput};
use crate::window::{KeyboardFocus, PasteTarget, Window, WindowHandler, WindowId};

mod cli;
mod config;
mod engine;
mod storage;
mod thread;
mod ui;
mod uri;
mod wayland;
mod window;

mod gl {
    #![allow(clippy::all, unsafe_op_in_unsafe_fn)]
    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("{0}")]
    WaylandConnect(#[from] ConnectError),
    #[error("Wayland protocol error for {0}: {1}")]
    WaylandProtocol(&'static str, #[source] BindError),
    #[error("{0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("{0}")]
    Glutin(#[from] glutin::error::Error),
    #[error("{0}")]
    WaylandGlobal(#[from] GlobalError),
    #[error("{0}")]
    Config(#[from] configory::Error),
    #[error("{0}")]
    WebKit(#[from] WebKitError),
    #[error("{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("local database version ({0}) is higher than latest supported version ({1})")]
    UnknownDbVersion(u8, u8),
    #[error("No available IPC socket files found")]
    NoSocketFound,
}

fn main() {
    if let Err(err) = run() {
        error!("[CRITICAL] {err}");
        process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    // Setup logging.
    let directives = env::var("RUST_LOG").unwrap_or("warn,kumo=info,configory=info".into());
    let env_filter = EnvFilter::builder().parse_lossy(directives);
    FmtSubscriber::builder().with_env_filter(env_filter).with_line_number(true).init();

    // Start profiling server.
    #[cfg(feature = "profiling")]
    let _server = {
        puffin::set_scopes_on(true);
        Server::new(&format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT)).unwrap()
    };

    // Parse CLI options.
    let options = Options::parse();

    // Handle subcommands like config IPC.
    if let Some(subcommands) = options.subcommands {
        handle_subcommands(subcommands)?;
        return Ok(());
    }

    info!("Started Kumo");

    // Set GLib application name.
    //
    // This is necessary to match the flatpak ID when run inside flatpak due to
    // WebKit's internal flatpak sandbox handling, which is why the reverse
    // domain name notation is used.
    glib::set_prgname(Some("org.catacombing.kumo"));

    // Initialize configuration state.
    let queue = Queue::new()?;
    let config_shutdown = init_config(queue.handle())?;

    let main_loop = MainLoop::new(None, true);
    let mut state = State::new(queue.local_handle(), main_loop.clone())?;

    // Create our initial window.
    let window_id = state.create_window();

    // Create an empty tab for loading a new page.
    let mut is_first_tab = true;
    let get_empty_tab =
        |window: &mut Window, is_first_tab: &mut bool, group_id: GroupId, focus: bool| -> _ {
            if *is_first_tab && group_id == NO_GROUP_ID {
                window.set_keyboard_focus(KeyboardFocus::None);
                *is_first_tab = false;
                window.active_tab().unwrap().id()
            } else {
                window.add_tab(false, focus, group_id)
            }
        };

    // Get all sessions requiring restoration, sorted by PID and window ID.
    let mut orphan_sessions = state.storage.session.orphans();
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
                let new_window_id = state.create_window();
                window = state.windows.get_mut(&new_window_id).unwrap();
                is_first_tab = true;
            }

            session_window_id = Some(entry.window_id);
            session_pid = Some(entry.pid);
        }

        // Recreate tab groups.
        let group_id = if entry.group_id == NO_GROUP_ID.uuid() {
            NO_GROUP_ID
        } else {
            let db_group = state.storage.groups.group_by_id(entry.group_id);
            let group =
                db_group.unwrap_or_else(|| Group::with_uuid(entry.group_id, "-".into(), false));
            window.create_tab_group(Some(group), entry.focused)
        };

        // Restore the session in a new empty tab.
        let engine_id = get_empty_tab(window, &mut is_first_tab, group_id, entry.focused);
        if let Some(engine) = window.tab_mut(engine_id) {
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
        window.persist_session();
    }
    state.storage.session.delete_orphans(orphan_sessions.iter().map(|s| s.pid));

    // Spawn a new tab for every CLI argument.
    let window = state.windows.get_mut(&window_id).unwrap();
    for arg in options.links {
        get_empty_tab(window, &mut is_first_tab, NO_GROUP_ID, true);
        window.load_uri(arg, true);
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

    // Terminate config thread.
    let _ = config_shutdown.send(ConfigEvent::User(()));

    Ok(())
}

/// Handle CLI subcommands.
fn handle_subcommands(subcommands: Subcommands) -> Result<(), Error> {
    match subcommands {
        Subcommands::Config(ConfigOptions::Get(options)) => {
            // Abort if there are no sockets available.
            let ipcs = Ipc::all("kumo");
            if ipcs.is_empty() {
                return Err(Error::NoSocketFound);
            }

            // Try to get value from first available socket.
            let path = options.path.as_ref().map_or(Vec::new(), |p| p.split('.').collect());
            let result = Ipc::all("kumo").into_iter().find_map(|ipc| {
                // Get value as generic toml.
                let value = ipc.get::<_, toml::Value>(&path);

                match value {
                    // Stop once we got any socket response.
                    Ok(value) => Some(value),
                    // Log socket errors.
                    Err(err) => {
                        error!("Failed on {:?}: {err}", ipc.socket_path());
                        None
                    },
                }
            });

            match result {
                // Print value to STDOUT if it is set.
                Some(Some(value)) => println!("{value}"),
                Some(None) => (),
                // Print error if all sockets failed.
                None => return Err(Error::NoSocketFound),
            }
        },
        Subcommands::Config(ConfigOptions::Set(options)) => {
            let value = cli::parse_toml_value(&options.path, options.value)?;
            let path: Vec<_> = options.path.split('.').collect();

            // Update option for every available socket.
            let mut failed = false;
            for ipc in Ipc::all("kumo") {
                if let Err(err) = ipc.set(&path, value.clone()) {
                    error!("Failed on {:?}: {err}", ipc.socket_path());
                    failed = true;
                }
            }

            // Set failing exit code if any socket failed update.
            if failed {
                process::exit(1);
            }
        },
        Subcommands::Config(ConfigOptions::Reset(options)) => {
            let path: Vec<_> = options.path.split('.').collect();

            // Update option for every available socket.
            let mut failed = false;
            for ipc in Ipc::all("kumo") {
                if let Err(err) = ipc.reset(&path) {
                    error!("Failed on {:?}: {err}", ipc.socket_path());
                    failed = true;
                }
            }

            // Set failing exit code if any socket failed update.
            if failed {
                process::exit(1);
            }
        },
    }

    Ok(())
}

/// Main application state.
pub struct State {
    main_loop: MainLoop,

    wayland_queue: Option<EventQueue<Self>>,
    protocol_states: ProtocolStates,
    connection: Connection,
    egl_display: Display,

    text_input: Vec<TextInput>,
    keyboard: Option<KeyboardState>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,
    clipboard: ClipboardState,

    windows: HashMap<WindowId, Window>,
    keyboard_focus: Option<WindowId>,
    touch_focus: Option<(WindowId, WlSurface)>,

    engine_state: Rc<RefCell<WebKitState>>,

    storage: Storage,

    queue: StQueueHandle<State>,
}

impl State {
    fn new(queue: StQueueHandle<Self>, main_loop: MainLoop) -> Result<Self, Error> {
        // Initialize Wayland connection.
        let connection = Connection::connect_to_env()?;
        let (globals, wayland_queue) = globals::registry_queue_init(&connection)?;
        let protocol_states = ProtocolStates::new(&globals, &wayland_queue.handle())?;

        // Get EGL display.
        let display = NonNull::new(connection.backend().display_ptr().cast()).unwrap();
        let wayland_display = WaylandDisplayHandle::new(display);
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let egl_display = unsafe { Display::new(raw_display, DisplayApiPreference::Egl)? };

        let storage = Storage::new()?;

        let engine_state = Rc::new(RefCell::new(WebKitState::new(
            egl_display.clone(),
            queue.clone(),
            storage.cookie_whitelist.clone(),
            &storage.groups.all_group_ids(),
        )));

        Ok(Self {
            protocol_states,
            engine_state,
            egl_display,
            connection,
            main_loop,
            storage,
            queue,
            wayland_queue: Some(wayland_queue),
            keyboard_focus: Default::default(),
            touch_focus: Default::default(),
            text_input: Default::default(),
            clipboard: Default::default(),
            keyboard: Default::default(),
            windows: Default::default(),
            pointer: Default::default(),
            touch: Default::default(),
        })
    }

    /// Create a new browser window.
    fn create_window(&mut self) -> WindowId {
        // Setup new window.
        let connection = self.connection.clone();
        let window = Window::new(
            &self.protocol_states,
            connection,
            self.egl_display.clone(),
            self.queue.clone(),
            self.wayland_queue(),
            &self.storage,
            self.engine_state.clone(),
        );
        let window_id = window.id();
        self.windows.insert(window_id, window);

        // Ensure Wayland processing is kicked off.
        self.wayland_dispatch();

        window_id
    }

    /// Get access to the Wayland queue.
    fn wayland_queue(&self) -> QueueHandle<Self> {
        self.wayland_queue.as_ref().unwrap().handle()
    }

    /// Update the seat's clipboard content.
    fn set_clipboard(&mut self, text: String) {
        let wayland_queue = self.wayland_queue();
        let serial = self.clipboard.next_serial();

        let copy_paste_source = self
            .protocol_states
            .data_device_manager
            .create_copy_paste_source(&wayland_queue, ["text/plain"]);
        copy_paste_source.set_selection(&self.protocol_states.data_device, serial);
        self.clipboard.source = Some(copy_paste_source);

        self.clipboard.text = text;
    }

    /// Request clipboard paste.
    fn request_paste(&mut self, target: PasteTarget) {
        // Get available Wayland text selection.
        let selection_offer = match self.protocol_states.data_device.data().selection_offer() {
            Some(selection_offer) => selection_offer,
            None => return,
        };
        let mut pipe = match selection_offer.receive("text/plain".into()) {
            Ok(pipe) => pipe,
            Err(err) => {
                warn!("Clipboard paste failed: {err}");
                return;
            },
        };

        // Asynchronously write paste text to the window.
        let mut queue = self.queue.clone();
        source::unix_fd_add_local(pipe.as_raw_fd(), IOCondition::IN, move |_, _| {
            // Read available text from pipe.
            let mut text = String::new();
            pipe.read_to_string(&mut text).unwrap();

            // Forward text to the paste target.
            queue.paste(target, text);

            ControlFlow::Break
        });
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
        if self.current_repeat.as_ref().is_some_and(|repeat| repeat.raw == raw) {
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

/// Clipboard content cache.
#[derive(Default)]
struct ClipboardState {
    serial: u32,
    text: String,
    source: Option<CopyPasteSource>,
}

impl ClipboardState {
    fn next_serial(&mut self) -> u32 {
        self.serial += 1;
        self.serial
    }
}

/// Initialize configuration state.
fn init_config(mut queue: MtQueueHandle<State>) -> Result<Sender<ConfigEvent<()>>, Error> {
    // Load initial configuration.
    let config_manager = ConfigManager::<()>::new("kumo")?;
    let config = config_manager
        .get::<&str, Config>(&[])
        .inspect_err(|err| error!("Config error: {err}"))
        .ok()
        .flatten()
        .unwrap_or_default();
    *CONFIG.write().unwrap() = config;

    // Monitor channel for configuration updates.
    let update_tx = config_manager.update_tx().clone();
    thread::spawn_named("config channel watcher", move || {
        let update_rx = config_manager.update_rx();
        while let Ok(event) = update_rx.recv() {
            match event {
                // Update configuration on change.
                ConfigEvent::FileChanged | ConfigEvent::IpcChanged => {
                    info!("Reloading configuration file");

                    // Parse config or fall back to the default.
                    let parsed = config_manager
                        .get::<&str, Config>(&[])
                        .inspect_err(|err| error!("Config error: {err}"))
                        .ok()
                        .flatten()
                        .unwrap_or_default();

                    // Calculate generation based on current config.
                    let mut config = CONFIG.write().unwrap();
                    let next_generation = config.generation + 1;

                    // Update the config.
                    *config = parsed;
                    config.generation = next_generation;

                    // Request redraw.
                    queue.unstall();
                },
                ConfigEvent::FileError(err) => error!("Configuration file error: {err}"),
                // User events are only used to shut down the thread.
                ConfigEvent::User(()) => break,
                ConfigEvent::Ipc(_) => unreachable!(),
            }
        }
    });

    Ok(update_tx)
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

impl Position<f64> {
    fn i32_round(&self) -> Position {
        Position::new(self.x.round() as i32, self.y.round() as i32)
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

impl Div<f64> for Position {
    type Output = Self;

    fn div(mut self, scale: f64) -> Self {
        self.x = (self.x as f64 / scale).round() as i32;
        self.y = (self.y as f64 / scale).round() as i32;
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

impl<T: Mul<T, Output = T> + Copy> Mul<T> for Size<T> {
    type Output = Self;

    fn mul(mut self, scale: T) -> Self {
        self.width = self.width * scale;
        self.height = self.height * scale;
        self
    }
}

impl Div<f64> for Size {
    type Output = Self;

    fn div(mut self, scale: f64) -> Self {
        self.width = (self.width as f64 / scale).round() as u32;
        self.height = (self.height as f64 / scale).round() as u32;
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
