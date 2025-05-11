use std::any::Any;
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use uuid::Uuid;

use crate::config::colors::BG;
use crate::ui::overlay::downloads::{Download, DownloadId};
use crate::ui::overlay::option_menu::OptionMenuId;
use crate::window::TextInputChange;
use crate::{KeyboardFocus, Position, Size, State, WindowId};

pub mod webkit;

// Constants for the default tab group.
pub const NO_GROUP_ID: GroupId = GroupId(Uuid::nil());
pub const NO_GROUP: Group = Group::none();
pub const NO_GROUP_REF: &Group = &NO_GROUP;

/// Tab group label for the default group.
const DEFAULT_GROUP_LABEL: &str = "Default";

pub trait Engine {
    /// Get the engine's unique ID.
    fn id(&self) -> EngineId;

    /// Check if the engine requires a redraw.
    ///
    /// This will always be called **before** any rendering is done.
    fn dirty(&mut self) -> bool;

    /// Attach the engine's buffer to the supplied surface.
    ///
    /// Should return `false` when no surface was attached.
    fn attach_buffer(&mut self, surface: &WlSurface) -> bool;

    /// Get the Wayland buffer's current physical size.
    fn buffer_size(&self) -> Size;

    /// Get the buffer damage since the last call to this function.
    ///
    /// A return value of [`None`] implies no damage information is present, so
    /// it is treated as full buffer damage.
    ///
    /// No damage is represented by a return value of `Some(Vec::new())`.
    fn take_buffer_damage(&mut self) -> Option<Vec<(i32, i32, i32, i32)>> {
        None
    }

    /// Notify engine that the frame was completed.
    fn frame_done(&mut self) {}

    /// Notify engine that a buffer was released.
    fn buffer_released(&mut self, _buffer: &WlBuffer) {}

    /// Update DMA buffer feedback.
    fn dmabuf_feedback(&mut self, _feedback: &DmabufFeedback) {}

    /// Get the buffer's opaque region.
    fn opaque_region(&self) -> Option<&WlRegion> {
        None
    }

    /// Update the browser engine's size.
    fn set_size(&mut self, size: Size);

    /// Update the browser engine's scale.
    fn set_scale(&mut self, scale: f64);

    /// Handle key down.
    fn press_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers);

    /// Handle key up.
    fn release_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers);

    /// Handle pointer axis scroll.
    fn pointer_axis(
        &mut self,
        time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        modifiers: Modifiers,
    );

    /// Handle pointer button press.
    fn pointer_button(
        &mut self,
        time: u32,
        position: Position<f64>,
        button: u32,
        down: bool,
        modifiers: Modifiers,
    );

    /// Handle pointer motion.
    fn pointer_motion(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers);

    /// Handle pointer enter.
    fn pointer_enter(&mut self, position: Position<f64>, modifiers: Modifiers);

    /// Handle pointer leave.
    fn pointer_leave(&mut self, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch press.
    fn touch_up(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch release.
    fn touch_down(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch motion.
    fn touch_motion(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Load a new page.
    fn load_uri(&mut self, uri: &str);

    /// Go to the previous page.
    fn load_prev(&mut self);

    /// Check if engine has any history.
    fn has_prev(&self) -> bool;

    /// Get current URI.
    fn uri(&self) -> Cow<'_, str>;

    /// Get tab title.
    fn title(&self) -> Cow<'_, str>;

    /// Get IME text_input state.
    fn text_input_state(&self) -> TextInputChange;

    /// Delete text around the current cursor position.
    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32);

    /// Insert text at the current cursor position.
    fn commit_string(&mut self, text: String);

    /// Set preedit text at the current cursor position.
    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32);

    /// Paste clipboard text.
    fn paste(&mut self, text: String);

    /// Clear engine focus.
    fn clear_focus(&mut self);

    /// Submit option menu item selection.
    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize);

    /// Close option menu.
    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>);

    /// Notify engine about change to the fullscreen state.
    fn set_fullscreen(&mut self, fullscreen: bool);

    /// Get a serialized version of the current session.
    fn session(&self) -> Vec<u8>;

    /// Restore a browser session.
    fn restore_session(&self, session: Vec<u8>);

    /// Get favicon for the current page.
    fn favicon(&self) -> Option<Favicon> {
        None
    }

    /// Get the current favicon resource URI.
    fn favicon_uri(&self) -> Option<glib::GString> {
        None
    }

    /// Cancel a file download.
    fn cancel_download(&mut self, _download_id: DownloadId) {}

    /// Set the engine's page scale.
    fn set_zoom_level(&mut self, _scale: f64) {}

    /// Web page scale.
    fn zoom_level(&self) -> f64 {
        1.
    }

    fn as_any(&mut self) -> &mut dyn Any;
}

#[funq::callbacks(State)]
pub trait EngineHandler {
    /// Update current URI for an engine.
    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String);

    /// Update current title for an engine.
    fn set_engine_title(&mut self, engine_id: EngineId, title: String);

    /// Handle fullscreen enter/leave.
    fn set_fullscreen(&mut self, engine_id: EngineId, enable: bool);

    /// Update page load progress.
    ///
    /// The `progress` argument is a percentage between 0.0 and 1.0.
    fn set_load_progress(&mut self, engine_id: EngineId, progress: f64);

    /// Open URI in a new tab.
    fn open_in_tab(&mut self, engine_id: EngineId, uri: String);

    /// Open URI in a new window.
    fn open_in_window(&mut self, uri: String);

    /// Add host to the cookie whitelist.
    fn add_cookie_exception(&mut self, host: String);

    /// Remove host from the cookie whitelist.
    fn remove_cookie_exception(&mut self, host: String);

    /// Trigger a favicon update for an engine.
    fn update_favicon(&mut self, engine_id: EngineId);

    /// Add a new download.
    fn add_download(&mut self, window_id: WindowId, download: Download);

    /// Update a download's progress.
    ///
    /// A progress value of `None` indicates that the download has failed and
    /// will not make any further progress.
    fn set_download_progress(&mut self, download_id: DownloadId, progress: Option<u8>);
}

impl EngineHandler for State {
    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_engine_uri(engine_id, uri);
        }
    }

    fn set_engine_title(&mut self, engine_id: EngineId, title: String) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_engine_title(&self.storage.history, engine_id, title);
        }
    }

    fn set_fullscreen(&mut self, engine_id: EngineId, enable: bool) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.request_fullscreen(engine_id, enable);
        }
    }

    fn set_load_progress(&mut self, engine_id: EngineId, progress: f64) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_load_progress(engine_id, progress);
        }
    }

    fn open_in_tab(&mut self, engine_id: EngineId, uri: String) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        let tab_id = window.add_tab_from_engine(false, false, engine_id.group_id(), engine_id);
        if let Some(engine) = window.tab_mut(tab_id) {
            engine.load_uri(&uri);
        }
    }

    fn open_in_window(&mut self, uri: String) {
        let window_id = self.create_window();
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.set_keyboard_focus(KeyboardFocus::None);
        if let Some(engine) = window.active_tab_mut() {
            engine.load_uri(&uri);
        }
    }

    fn add_cookie_exception(&mut self, host: String) {
        self.storage.cookie_whitelist.add(&host);
    }

    fn remove_cookie_exception(&mut self, host: String) {
        self.storage.cookie_whitelist.remove(&host);
    }

    fn update_favicon(&mut self, engine_id: EngineId) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.update_favicon(engine_id);
        }
    }

    fn add_download(&mut self, window_id: WindowId, download: Download) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.add_download(download);
        }
    }

    fn set_download_progress(&mut self, download_id: DownloadId, progress: Option<u8>) {
        if let Some(window) = self.windows.get_mut(&download_id.window_id()) {
            window.set_download_progress(download_id, progress);
        }
    }
}

/// Unique identifier for one engine instance.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EngineId {
    window_id: WindowId,
    group_id: GroupId,
    id: usize,
}

impl EngineId {
    pub fn new(window_id: WindowId, group_id: GroupId) -> Self {
        static NEXT_ENGINE_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed);
        Self { window_id, group_id, id }
    }

    /// Get the engine's window.
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// Get the engine's tab group.
    pub fn group_id(&self) -> GroupId {
        self.group_id
    }
}

/// Tab group, for engine context sharing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Group {
    /// Globally unique group ID.
    id: GroupId,

    /// Human-readable group identifier.
    pub label: Cow<'static, str>,

    /// Whether data for this group should be persisted.
    pub ephemeral: bool,
}

impl Group {
    /// Create a new tab group.
    pub fn new(ephemeral: bool) -> Self {
        let id = GroupId(Uuid::new_v4());
        let label = id.0.to_string().into();
        Self { id, label, ephemeral }
    }

    /// Create a tab group with a fixed UUID.
    ///
    /// Two different tab groups must never be created with the same UUID.
    pub fn with_uuid(uuid: Uuid, label: String, ephemeral: bool) -> Self {
        Self { label: label.into(), id: GroupId(uuid), ephemeral }
    }

    /// Get the default tab group.
    pub const fn none() -> Self {
        Self { id: NO_GROUP_ID, ephemeral: false, label: Cow::Borrowed(DEFAULT_GROUP_LABEL) }
    }

    /// Get this group's ID.
    pub const fn id(&self) -> GroupId {
        self.id
    }
}

/// Unique identifier for a tab group.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(Uuid);

impl GroupId {
    /// Get the raw group UUID value.
    pub fn uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for GroupId {
    fn default() -> Self {
        NO_GROUP_ID
    }
}

/// Page favicon data.
#[derive(Clone, Debug)]
pub struct Favicon {
    pub resource_uri: glib::GString,
    pub bytes: glib::Bytes,
    pub width: usize,
    pub height: usize,
}
