use std::any::Any;
use std::borrow::Cow;
use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;
#[cfg(feature = "servo")]
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "servo")]
use ::servo::ContextMenuElementInformationFlags;
use bitflags::bitflags;
use configory::docgen::Docgen;
use funq::MtQueueHandle;
use rusqlite::types::{FromSql, FromSqlError, ToSql, ToSqlOutput, ValueRef};
use serde::Deserialize;
use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use url::Url;
use uuid::Uuid;
#[cfg(feature = "webkit")]
use wpe_webkit::HitTestResultContext;

use crate::gl::types::GLenum;
use crate::storage::cookie_whitelist::CookieWhitelist;
#[cfg(feature = "webkit")]
use crate::ui::overlay::downloads::Download;
use crate::ui::overlay::downloads::DownloadId;
use crate::ui::overlay::option_menu::{OptionMenuId, OptionMenuItem};
use crate::window::{PasteTarget, TextInputChange, WindowHandler};
use crate::{KeyboardFocus, Position, Size, State, WindowId};

pub mod dummy;
#[cfg(feature = "servo")]
pub mod servo;
#[cfg(feature = "webkit")]
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

    /// Get this engine's type.
    fn engine_type(&self) -> EngineType;

    /// Check if the engine requires a redraw.
    ///
    /// This will always be called **before** any rendering is done.
    fn dirty(&mut self) -> bool;

    /// Set whether the browser engine is currently visible for rendering.
    fn set_visible(&mut self, _visible: bool) {}

    /// Draw the engine's content to its surface.
    ///
    /// Should return `false` if no buffer was attached to the surface.
    fn draw(&mut self) -> bool;

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
    fn pointer_enter(&mut self, _position: Position<f64>, _modifiers: Modifiers) {}

    /// Handle pointer leave.
    fn pointer_leave(&mut self, _position: Position<f64>, _modifiers: Modifiers) {}

    /// Handle touch press.
    fn touch_up(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch release.
    fn touch_down(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch motion.
    fn touch_motion(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers);

    /// Reload the current page.
    fn reload(&mut self);

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
    fn text_input_state(&mut self) -> TextInputChange;

    /// Delete text around the current cursor position.
    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32);

    /// Insert text at the current cursor position.
    fn commit_string(&mut self, text: String);

    /// Set preedit text at the current cursor position.
    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32);

    /// Clear engine focus.
    fn clear_focus(&mut self);

    /// Submit option menu item selection.
    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize);

    /// Close option menu.
    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>);

    /// Notify engine about change to the fullscreen state.
    fn set_fullscreen(&mut self, fullscreen: bool);

    /// Get a serialized version of the current session.
    fn session(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore a browser session.
    fn restore_session(&self, _session: Vec<u8>) {}

    /// Get favicon for the current page.
    fn favicon(&self) -> Option<Favicon> {
        None
    }

    // Servo currently does not support any way to cheaply identify favicons:
    // <https://github.com/servo/servo/issues/43159>
    //
    /// Get the current favicon resource URI.
    #[cfg(feature = "webkit")]
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

    /// Start or update a text search.
    fn update_search(&mut self, _text: &str) {}

    /// Stop all active text searches.
    fn stop_search(&mut self) {}

    /// Go to the next search match.
    fn search_next(&mut self) {}

    /// Go to the previous search match.
    fn search_prev(&mut self) {}

    fn as_any(&mut self) -> &mut dyn Any;
}

#[funq::callbacks(State)]
pub trait EngineHandler {
    /// Update current URI for an engine.
    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String, update_history: bool);

    /// Reload a page.
    fn reload(&mut self, engine_id: EngineId);

    /// Reload tab in a different browser engine.
    #[cfg(all(feature = "servo", feature = "webkit"))]
    fn switch_engine(&mut self, engine_id: EngineId, engine_type: EngineType);

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
    ///
    /// This will automatically copy the parent's group settings to ensure
    /// ephemeral mode is persisted for the new window.
    fn open_in_window(&mut self, parent: EngineId, uri: String);

    /// Add host to the cookie whitelist.
    fn add_cookie_exception(&mut self, host: String);

    /// Remove host from the cookie whitelist.
    fn remove_cookie_exception(&mut self, host: String);

    /// Trigger a favicon update for an engine.
    fn update_favicon(&mut self, engine_id: EngineId);

    /// Add a new download.
    #[cfg(feature = "webkit")]
    fn add_download(&mut self, window_id: WindowId, download: Download);

    /// Update a download's progress.
    ///
    /// A progress value of `None` indicates that the download has failed and
    /// will not make any further progress.
    #[cfg(feature = "webkit")]
    fn set_download_progress(&mut self, download_id: DownloadId, progress: Option<u8>);

    /// Start page text search.
    #[cfg(feature = "webkit")]
    fn start_search(&mut self, engine_id: EngineId);

    /// Update number of text search matches.
    #[cfg(feature = "webkit")]
    fn set_search_match_count(&mut self, engine_id: EngineId, count: usize);

    /// Set whether a tab is playing audio.
    #[cfg(feature = "webkit")]
    fn set_audio_playing(&mut self, engine_id: EngineId, playing: bool);
}

impl EngineHandler for State {
    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String, update_history: bool) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_engine_uri(engine_id, uri, update_history);
        }
    }

    fn reload(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine = match window.tab_mut(engine_id) {
            Some(engine) => engine,
            None => return,
        };

        engine.reload();
    }

    #[cfg(all(feature = "servo", feature = "webkit"))]
    fn switch_engine(&mut self, engine_id: EngineId, engine_type: EngineType) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        window.switch_engine(engine_id, engine_type);
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

        window.add_tab_from_engine(false, false, engine_id.group_id(), Some(&uri), engine_id);
    }

    fn open_in_window(&mut self, parent: EngineId, uri: String) {
        // Check if group of the tab spawning the window is ephemeral.
        let parent_ephemeral = self
            .windows
            .get(&parent.window_id())
            .and_then(|window| window.group(parent.group_id()))
            .is_some_and(|group| group.ephemeral);

        let window = self.create_window();

        // If the parent is ephemeral, switch to a new ephemeral group.
        if parent_ephemeral {
            let group_id = window.create_tab_group(None, true);
            window.set_ephemeral_mode(group_id, true);
            window.add_tab(false, true, group_id, Some(&uri));
        } else {
            window.add_tab(false, true, NO_GROUP_ID, Some(&uri));
        }

        window.set_keyboard_focus(KeyboardFocus::None);
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

    #[cfg(feature = "webkit")]
    fn add_download(&mut self, window_id: WindowId, download: Download) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.add_download(download);
        }
    }

    #[cfg(feature = "webkit")]
    fn set_download_progress(&mut self, download_id: DownloadId, progress: Option<u8>) {
        if let Some(window) = self.windows.get_mut(&download_id.window_id()) {
            window.set_download_progress(download_id, progress);
        }
    }

    #[cfg(feature = "webkit")]
    fn start_search(&mut self, engine_id: EngineId) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.start_search(engine_id);
        }
    }

    #[cfg(feature = "webkit")]
    fn set_search_match_count(&mut self, engine_id: EngineId, count: usize) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_search_match_count(engine_id, count);
        }
    }

    #[cfg(feature = "webkit")]
    fn set_audio_playing(&mut self, engine_id: EngineId, playing: bool) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.set_audio_playing(engine_id, playing);
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
    pub id: FaviconId,
    pub bytes: glib::Bytes,
    pub width: usize,
    pub height: usize,
    pub format: GLenum,
}

/// Unique identifier for a Servo favicon.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaviconId {
    #[cfg(feature = "webkit")]
    WebKit(glib::GString),
    #[cfg(feature = "servo")]
    Servo(u32),
}

impl FaviconId {
    #[cfg(feature = "servo")]
    pub fn new_servo() -> Self {
        static NEXT_SERVO_FAVICON_ID: AtomicU32 = AtomicU32::new(0);
        Self::Servo(NEXT_SERVO_FAVICON_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Browser engine context menu.
#[derive(Debug)]
struct ContextMenu {
    target_flags: ContextMenuTargetFlags,
    has_cookie_exception: bool,
    target_url: Option<String>,
    #[cfg(feature = "webkit")]
    engine_type: EngineType,
    host: Option<String>,
}

impl ContextMenu {
    fn new(
        #[cfg(feature = "webkit")] engine_type: EngineType,
        target_flags: ContextMenuTargetFlags,
        cookie_whitelist: &CookieWhitelist,
        engine_url: &str,
        target_url: Option<String>,
    ) -> Self {
        let (host, has_cookie_exception) = if target_flags.is_empty()
            && let Some(host) = Url::parse(engine_url).ok().as_ref().and_then(|url| url.host_str())
        {
            let has_cookie_exception = cookie_whitelist.contains(host);
            (Some(host.into()), has_cookie_exception)
        } else {
            (None, false)
        };

        Self {
            has_cookie_exception,
            target_flags,
            #[cfg(feature = "webkit")]
            engine_type,
            target_url,
            host,
        }
    }

    /// Number of items in the menu.
    fn len(&self) -> u32 {
        let mut n_items = 0;

        // Only show these entries without any targeted element.
        if self.target_flags.is_empty() {
            #[cfg(feature = "webkit")]
            if self.engine_type == EngineType::WebKit {
                n_items += 1;
            }
            n_items += 1;
        }

        n_items += 1;
        #[cfg(all(feature = "servo", feature = "webkit"))]
        {
            n_items += 1;
        }

        if self
            .target_flags
            .intersects(ContextMenuTargetFlags::LINK | ContextMenuTargetFlags::MEDIA)
        {
            n_items += 3;
        }

        if self.target_flags.contains(ContextMenuTargetFlags::EDITABLE) {
            n_items += 1;
        }

        n_items
    }

    /// Get option menu items for this menu.
    fn items(&self) -> Vec<OptionMenuItem> {
        let mut items = Vec::new();
        for i in 0..self.len() {
            if let Some(item) = self.item(i) {
                items.push(OptionMenuItem {
                    description: String::new(),
                    label: item.label().into(),
                    disabled: false,
                    selected: false,
                });
            }
        }
        items
    }

    /// Get the item at the specified index.
    #[allow(clippy::single_match)]
    fn item(&self, mut index: u32) -> Option<ContextMenuItem> {
        // Only show these entries without any targeted element.
        if self.target_flags.is_empty() {
            match index {
                0 if self.has_cookie_exception => {
                    return Some(ContextMenuItem::RemoveCookieException);
                },
                0 => return Some(ContextMenuItem::AddCookieException),
                #[cfg(feature = "webkit")]
                1 if self.engine_type == EngineType::WebKit => {
                    return Some(ContextMenuItem::Search);
                },
                _ => (),
            }
            #[cfg(feature = "webkit")]
            if self.engine_type == EngineType::WebKit {
                index -= 1;
            }
            index -= 1;
        }

        match index {
            0 => return Some(ContextMenuItem::Reload),
            #[cfg(all(feature = "servo", feature = "webkit"))]
            1 => match self.engine_type {
                EngineType::Servo => return Some(ContextMenuItem::ReloadInWebKit),
                EngineType::WebKit => return Some(ContextMenuItem::ReloadInServo),
                EngineType::Dummy => unreachable!("dummy engine context menu"),
            },
            _ => (),
        }
        index -= 1;
        #[cfg(all(feature = "servo", feature = "webkit"))]
        {
            index -= 1;
        }

        if self
            .target_flags
            .intersects(ContextMenuTargetFlags::LINK | ContextMenuTargetFlags::MEDIA)
        {
            match index {
                0 => return Some(ContextMenuItem::CopyLink),
                1 => return Some(ContextMenuItem::OpenInNewWindow),
                2 => return Some(ContextMenuItem::OpenInNewTab),
                _ => (),
            }
            index -= 3;
        }

        if self.target_flags.contains(ContextMenuTargetFlags::EDITABLE) {
            match index {
                0 => return Some(ContextMenuItem::Paste),
                _ => (),
            }
            // index -= 1;
        }

        None
    }

    /// Activate item at the specified position.
    fn activate_item(&self, mut queue: MtQueueHandle<State>, engine_id: EngineId, index: u32) {
        match self.item(index) {
            Some(ContextMenuItem::AddCookieException) => {
                if let Some(host) = &self.host {
                    queue.add_cookie_exception(host.to_string());
                }
            },
            Some(ContextMenuItem::RemoveCookieException) => {
                if let Some(host) = &self.host {
                    queue.remove_cookie_exception(host.to_string());
                }
            },
            #[cfg(feature = "webkit")]
            Some(ContextMenuItem::Search) => queue.start_search(engine_id),
            #[cfg(all(feature = "servo", feature = "webkit"))]
            Some(ContextMenuItem::ReloadInWebKit) => {
                queue.switch_engine(engine_id, EngineType::WebKit)
            },
            #[cfg(all(feature = "servo", feature = "webkit"))]
            Some(ContextMenuItem::ReloadInServo) => {
                queue.switch_engine(engine_id, EngineType::Servo)
            },
            Some(ContextMenuItem::Reload) => queue.reload(engine_id),
            Some(ContextMenuItem::OpenInNewTab) => {
                if let Some(url) = self.url() {
                    queue.open_in_tab(engine_id, url.into());
                }
            },
            Some(ContextMenuItem::OpenInNewWindow) => {
                if let Some(url) = self.url() {
                    queue.open_in_window(engine_id, url.into());
                }
            },
            Some(ContextMenuItem::CopyLink) => {
                if let Some(url) = self.url() {
                    queue.set_clipboard(url.into());
                }
            },
            Some(ContextMenuItem::Paste) => queue.request_paste(PasteTarget::Browser(engine_id)),
            None => (),
        }
    }

    /// Get URL for the context menu's target resource.
    fn url(&self) -> Option<&str> {
        self.target_url.as_deref()
    }
}

bitflags! {
    /// Properties of the item a context menu was created for.
    #[derive(Copy, Clone, Debug)]
    pub struct ContextMenuTargetFlags: u8 {
        const LINK = 1 << 1;
        const MEDIA = 1 << 2;
        const EDITABLE = 1 << 3;
    }
}

#[cfg(feature = "webkit")]
impl From<HitTestResultContext> for ContextMenuTargetFlags {
    fn from(context: HitTestResultContext) -> Self {
        let mut flags = ContextMenuTargetFlags::empty();
        flags.set(
            ContextMenuTargetFlags::MEDIA,
            context.contains(HitTestResultContext::MEDIA | HitTestResultContext::IMAGE),
        );
        flags.set(
            ContextMenuTargetFlags::EDITABLE,
            context.contains(HitTestResultContext::EDITABLE),
        );
        flags.set(ContextMenuTargetFlags::LINK, context.contains(HitTestResultContext::LINK));
        flags
    }
}

#[cfg(feature = "servo")]
impl From<ContextMenuElementInformationFlags> for ContextMenuTargetFlags {
    fn from(context_flags: ContextMenuElementInformationFlags) -> Self {
        let mut flags = ContextMenuTargetFlags::empty();
        flags.set(
            ContextMenuTargetFlags::MEDIA,
            context_flags.contains(ContextMenuElementInformationFlags::Image),
        );
        flags.set(
            ContextMenuTargetFlags::EDITABLE,
            context_flags.contains(ContextMenuElementInformationFlags::EditableText),
        );
        flags.set(
            ContextMenuTargetFlags::LINK,
            context_flags.contains(ContextMenuElementInformationFlags::Link),
        );
        flags
    }
}

/// Custom context menu item based on a hit test.
#[derive(Debug)]
enum ContextMenuItem {
    AddCookieException,
    RemoveCookieException,
    #[cfg(all(feature = "servo", feature = "webkit"))]
    ReloadInWebKit,
    #[cfg(all(feature = "servo", feature = "webkit"))]
    ReloadInServo,
    Reload,
    OpenInNewTab,
    OpenInNewWindow,
    CopyLink,
    Paste,
    #[cfg(feature = "webkit")]
    Search,
}

impl ContextMenuItem {
    fn label(&self) -> &'static str {
        match self {
            Self::AddCookieException => "Add Cookie Exception",
            Self::RemoveCookieException => "Remove Cookie Exception",
            #[cfg(all(feature = "servo", feature = "webkit"))]
            Self::ReloadInWebKit => "Reload in WebKit",
            #[cfg(all(feature = "servo", feature = "webkit"))]
            Self::ReloadInServo => "Reload in Servo",
            Self::Reload => "Reload",
            Self::OpenInNewTab => "Open in New Tab",
            Self::OpenInNewWindow => "Open in New Window",
            Self::CopyLink => "Copy Link",
            Self::Paste => "Paste",
            #[cfg(feature = "webkit")]
            Self::Search => "Search Page",
        }
    }
}

/// Browser engine backend.
#[derive(Docgen, Deserialize, PartialEq, Eq, Copy, Clone, Debug)]
#[docgen(doc_type = "\"WebKit\" \\| \"Servo\"")]
pub enum EngineType {
    WebKit,
    Servo,
    #[serde(skip)]
    Dummy,
}

impl FromSql for EngineType {
    fn column_result(value: ValueRef<'_>) -> Result<Self, FromSqlError> {
        value.as_str().and_then(|s| match s {
            "webkit" => Ok(Self::WebKit),
            "servo" => Ok(Self::Servo),
            _ => Err(FromSqlError::InvalidType),
        })
    }
}

impl ToSql for EngineType {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>, rusqlite::Error> {
        let s = match self {
            Self::Dummy => unreachable!("dummy engine sql write"),
            Self::WebKit => "webkit",
            Self::Servo => "servo",
        };
        Ok(ToSqlOutput::from(s))
    }
}

/// Dynamically initialized engine state.
pub struct OnDemandState<T> {
    #[expect(clippy::type_complexity)]
    init: Rc<Cell<Option<Box<dyn FnOnce() -> T>>>>,
    state: Rc<RefCell<Option<T>>>,
}

impl<T> OnDemandState<T> {
    pub fn new(init: Box<dyn FnOnce() -> T>) -> Self {
        Self { init: Rc::new(Cell::new(Some(init))), state: Default::default() }
    }

    /// Get the initialized engine state.
    pub fn get(&self) -> RefMut<'_, T> {
        if let Some(init) = self.init.take() {
            *self.state.borrow_mut() = Some(init());
        }

        RefMut::map(self.state.borrow_mut(), |state| state.as_mut().unwrap())
    }

    /// Get the engine state, without initializing it if missing.
    pub fn try_get(&self) -> RefMut<'_, Option<T>> {
        self.state.borrow_mut()
    }
}

impl<T> Clone for OnDemandState<T> {
    fn clone(&self) -> Self {
        Self { init: self.init.clone(), state: self.state.clone() }
    }
}
