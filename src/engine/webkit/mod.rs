//! WebKit browser engine.

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Deref;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;

use _dmabuf::zwp_linux_buffer_params_v1::Flags as DmabufFlags;
use funq::StQueueHandle;
use gio::Cancellable;
use glib::object::{Cast, ObjectExt};
use glib::prelude::*;
use glib::{Bytes, GString, TimeSpan, Uri, UriFlags, UserDirectory};
use glutin::display::Display;
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::dmabuf::{DmabufFeedback, DmabufState};
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::linux_dmabuf::zv1::client as _dmabuf;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use tracing::{error, trace, warn};
use uuid::Uuid;
use wpe_platform::ffi::WPERectangle;
use wpe_platform::{Buffer, BufferDMABuf, BufferExt, BufferSHM, EventType};
use wpe_webkit::{
    Color, CookieAcceptPolicy, CookiePersistentStorage, Download as WebKitDownload, FindOptions,
    HitTestResult, HitTestResultContext, NetworkSession, OptionMenu,
    OptionMenuItem as WebKitOptionMenuItem, UserContentFilterStore, WebView, WebViewExt,
    WebViewSessionState, WebsiteDataManagerExtManual, WebsiteDataTypes,
};

use crate::config::CONFIG;
use crate::engine::webkit::platform::WebKitDisplay;
use crate::engine::{Engine, EngineHandler, EngineId, Favicon, GroupId};
use crate::storage::cookie_whitelist::CookieWhitelist;
use crate::ui::overlay::downloads::{Download, DownloadId};
use crate::ui::overlay::option_menu::{Anchor, OptionMenuId, OptionMenuItem, OptionMenuPosition};
use crate::window::{TextInputChange, Window, WindowHandler};
use crate::{PasteTarget, Position, Size, State};

mod input_method_context;
mod platform;

/// Content filter store ID for the adblock json.
const ADBLOCK_FILTER_ID: &str = "adblock";

/// Maximum number of buffers kept for release tracking.
///
/// If the number of buffers pending release exceeds this number,
/// then the oldest buffer is automatically assumed to be released.
const MAX_PENDING_BUFFERS: usize = 3;

/// Maximum number of days before website content disk caches will be cleared.
const MAX_DISK_CACHE_DAYS: i64 = 30;

/// Maximum matches for text search.
const MAX_MATCH_COUNT: u32 = 100;

/// WebKit-specific errors.
#[derive(thiserror::Error, Debug)]
pub enum WebKitError {}

#[funq::callbacks(State, thread_local)]
trait WebKitHandler {
    /// Handle a new WebKit frame.
    fn render_buffer(
        &mut self,
        engine_id: EngineId,
        buffer: Buffer,
        damage_rects: Vec<WPERectangle>,
    );

    /// Update buffer's opaque region.
    fn set_opaque_rectangles(&mut self, engine_id: EngineId, rects: Vec<WPERectangle>);

    /// Open popup.
    fn open_menu(&mut self, engine_id: EngineId, menu: Menu, rect: Option<(i32, i32, i32, i32)>);

    /// Close popup.
    fn close_menu(&mut self, menu_id: OptionMenuId);

    /// Add new download for a WebKit engine.
    fn add_webkit_download(&mut self, download_id: DownloadId, webkit_download: WebKitDownload);

    /// Remove a download from WebKit's cache.
    fn remove_webkit_download(&mut self, download_id: DownloadId);
}

impl WebKitHandler for State {
    fn render_buffer(
        &mut self,
        engine_id: EngineId,
        buffer: Buffer,
        damage_rects: Vec<WPERectangle>,
    ) {
        let wayland_queue = self.wayland_queue();
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let webkit_engine = match webkit_engine_by_id(window, engine_id) {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Update engine's buffer.
        match buffer.downcast::<BufferDMABuf>() {
            Ok(dma_buffer) => webkit_engine.import_buffer(
                &wayland_queue,
                &self.protocol_states.dmabuf,
                dma_buffer,
                damage_rects,
            ),
            Err(buffer) => {
                if buffer.is::<BufferSHM>() {
                    error!("WebKit SHM buffers are not supported");
                } else {
                    error!("Unknown WebKit buffer format");
                }
                webkit_engine.cleanup_buffer(&buffer);
                return;
            },
        }

        // Offer new WlBuffer to window.
        if window.active_tab().is_some_and(|engine| engine.id() == engine_id) {
            window.unstall();
        }
    }

    fn set_opaque_rectangles(&mut self, engine_id: EngineId, rects: Vec<WPERectangle>) {
        let webkit_engine = match self
            .windows
            .get_mut(&engine_id.window_id())
            .and_then(|window| webkit_engine_by_id(window, engine_id))
        {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        match Region::new(&self.protocol_states.compositor) {
            Ok(region) => {
                // Convert WebKit's buffer scale to surface scale.
                //
                // We intentionally round the rect size down here to avoid rendering artifacts.
                for rect in rects {
                    let x = (rect.x as f64 / webkit_engine.scale).ceil() as i32;
                    let y = (rect.y as f64 / webkit_engine.scale).ceil() as i32;
                    let width = (rect.width as f64 / webkit_engine.scale).floor() as i32;
                    let height = (rect.height as f64 / webkit_engine.scale).floor() as i32;
                    region.add(x, y, width, height);
                }

                webkit_engine.set_opaque_region(Some(region));
            },
            Err(err) => {
                error!("Could not create Wayland region: {err}");
                webkit_engine.set_opaque_region(None);
            },
        };
    }

    fn open_menu(&mut self, engine_id: EngineId, menu: Menu, rect: Option<(i32, i32, i32, i32)>) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let webkit_engine = match webkit_engine_by_id(window, engine_id) {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Get properties from WebKit menu items.
        let mut items = Vec::new();
        for i in 0..menu.n_items() {
            if let Some(mut item) = menu.item(i) {
                if let Some(label) = item.label() {
                    items.push(OptionMenuItem {
                        label: label.into(),
                        description: String::new(),
                        disabled: !item.is_enabled(),
                        selected: item.is_selected(),
                    });
                }
            }
        }

        // Get popup position.
        let (menu_position, item_width) = match rect {
            Some((x, y, width, height)) => {
                (Position::new(x, y + height).into(), Some(width as u32))
            },
            None => {
                let position = webkit_engine.last_input_position.i32_round();
                let menu_position = OptionMenuPosition::new(position, Anchor::BottomRight);
                (menu_position, None)
            },
        };

        // Hookup close callback.
        let menu_id = OptionMenuId::with_engine(engine_id);
        let close_queue = self.queue.clone();
        menu.connect_close(move || close_queue.clone().close_menu(menu_id));

        // Update engine's active popup for close/activate handling.
        if let Some((menu_id, _)) = webkit_engine.menu.take() {
            webkit_engine.close_option_menu(Some(menu_id));
        }
        webkit_engine.menu = Some((menu_id, menu));

        // Show the popup.
        window.open_option_menu(menu_id, menu_position, item_width, items.into_iter());
    }

    fn close_menu(&mut self, menu_id: OptionMenuId) {
        let engine_id = match menu_id.engine_id() {
            Some(engine_id) => engine_id,
            None => return,
        };
        let window = match self.windows.get_mut(&menu_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let webkit_engine = match webkit_engine_by_id(window, engine_id) {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Clear engine's option menu if it matches the menu's ID.
        if webkit_engine.menu.as_ref().is_some_and(|(id, _)| menu_id == *id) {
            webkit_engine.menu = None;
        }

        window.close_option_menu(menu_id);
    }

    fn add_webkit_download(&mut self, download_id: DownloadId, webkit_download: WebKitDownload) {
        let webkit_engine = match self
            .windows
            .get_mut(&download_id.window_id())
            .and_then(|window| webkit_engine_by_id(window, download_id.engine_id()))
        {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Transform WebKit into Kumo download.
        let uri = webkit_download
            .request()
            .and_then(|request| request.uri())
            .map_or_else(|| String::from("unkown"), |uri| uri.to_string());
        let destination =
            webkit_download.destination().map_or_else(|| "unkown".into(), |dst| dst.to_string());
        let download = Download {
            uri,
            id: download_id,
            destination,
            progress: Default::default(),
            failed: Default::default(),
        };

        // Add WebKit download to cache to allow cancellation.
        webkit_engine.downloads.insert(download_id, webkit_download);

        // Forward download for generic engine handling.
        self.add_download(download_id.window_id(), download);
    }

    fn remove_webkit_download(&mut self, download_id: DownloadId) {
        let webkit_engine = match self
            .windows
            .get_mut(&download_id.window_id())
            .and_then(|window| webkit_engine_by_id(window, download_id.engine_id()))
        {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        webkit_engine.downloads.remove(&download_id);
    }
}

/// WebKit shared engine state.
pub struct WebKitState {
    pub dmabuf_feedback: Rc<RefCell<Option<DmabufFeedback>>>,

    network_sessions: HashMap<GroupId, NetworkSession>,
    cookie_whitelist: CookieWhitelist,
    queue: StQueueHandle<State>,
    display: Display,
}

impl WebKitState {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new(
        display: Display,
        queue: StQueueHandle<State>,
        cookie_whitelist: CookieWhitelist,
        all_groups: &HashSet<Uuid>,
    ) -> Self {
        // Delete unused persistent data from the filesystem.
        //
        // This is also responsible for deleting previous ephemeral sessions.
        let cache_dir = cache_dir().map(|dir| dir.join("groups"));
        let data_dir = data_dir().map(|dir| dir.join("groups"));
        for base_dir in [cache_dir, data_dir].iter().flatten() {
            for entry in fs::read_dir(base_dir).into_iter().flatten().flatten() {
                let file_name = entry.file_name();
                let file_name = match file_name.to_str() {
                    Some(file_name) => file_name,
                    None => continue,
                };

                // Delete directory if it doesn't match any existing group UUID.
                if Uuid::parse_str(file_name).is_ok_and(|uuid| !all_groups.contains(&uuid)) {
                    let path = entry.path();
                    match fs::remove_dir_all(&path) {
                        Ok(_) => trace!("successfully removed unused group dir {path:?}"),
                        Err(err) => error!("failed removing unused group dir {path:?}: {err}"),
                    }
                }
            }
        }

        Self {
            cookie_whitelist,
            display,
            queue,
            network_sessions: Default::default(),
            dmabuf_feedback: Default::default(),
        }
    }

    /// Create a new WebKit engine from this state.
    pub fn create_engine(&mut self, engine_id: EngineId, size: Size, scale: f64) -> WebKitEngine {
        // Create a new network session for this group if necessary.
        let group_id = engine_id.group_id();
        let network_session = self.network_sessions.entry(group_id).or_insert_with(|| {
            let network_session =
                xdg_network_session(&self.cookie_whitelist, engine_id, self.queue.clone())
                    .unwrap_or_else(NetworkSession::new_ephemeral);
            network_session.website_data_manager().unwrap().set_favicons_enabled(true);
            network_session
        });

        // Get the DRM render node.
        let Display::Egl(egl_display) = &self.display;
        let device = egl_display.device().expect("get DRM device");
        let render_node = device
            .drm_render_device_node_path()
            .or_else(|| device.drm_device_node_path())
            .expect("DRM node has no path");

        // Create WebKit platform.
        let webkit_display = WebKitDisplay::new(
            self.queue.clone(),
            engine_id,
            render_node,
            size,
            scale,
            self.dmabuf_feedback.borrow().as_ref(),
        );

        // Create web view with initial blank page.
        let web_view =
            WebView::builder().network_session(network_session).display(&webkit_display).build();

        // Set browser background color.
        let bg = CONFIG.read().unwrap().colors.background.as_f64();
        let mut color = Color::new(bg[0], bg[1], bg[2], 1.);
        web_view.set_background_color(&mut color);

        // Notify UI about URI and title changes.
        let load_queue = self.queue.clone();
        web_view.connect_load_changed(move |web_view, _| {
            let uri = web_view.uri().unwrap_or_default().to_string();
            load_queue.clone().set_engine_uri(engine_id, uri);
        });
        let uri_queue = self.queue.clone();
        web_view.connect_uri_notify(move |web_view| {
            let uri = web_view.uri().unwrap_or_default().to_string();
            uri_queue.clone().set_engine_uri(engine_id, uri);
        });
        let title_queue = self.queue.clone();
        web_view.connect_title_notify(move |web_view| {
            let title = web_view.title().unwrap_or_default().to_string();
            title_queue.clone().set_engine_title(engine_id, title);
        });

        // Listen for option menu open events.
        let option_menu_queue = self.queue.clone();
        web_view.connect_show_option_menu(move |_, menu, rect| {
            option_menu_queue.clone().open_menu(engine_id, menu.into(), Some(rect.geometry()));
            true
        });

        // Listen for context menu open events.
        let cookie_whitelist = self.cookie_whitelist.clone();
        let context_menu_queue = self.queue.clone();
        web_view.connect_context_menu(move |web_view, _, hit_test_result| {
            let uri = web_view.uri().unwrap_or_default().to_string();
            let context_menu = ContextMenu::new(&cookie_whitelist, &uri, hit_test_result.clone());
            let menu = Menu::ContextMenu(context_menu);
            context_menu_queue.clone().open_menu(engine_id, menu, None);
            true
        });

        // Listen for page load progress.
        let load_progress_queue = self.queue.clone();
        web_view.connect_estimated_load_progress_notify(move |web_view| {
            let progress = web_view.estimated_load_progress();
            load_progress_queue.clone().set_load_progress(engine_id, progress);
        });

        // Update tabs view when on favicon change.
        let favicon_queue = self.queue.clone();
        web_view.connect_favicon_notify(move |_web_view| {
            favicon_queue.clone().update_favicon(engine_id);
        });

        // Update search input on failed/successful search.
        if let Some(find_controller) = web_view.find_controller() {
            let failed_queue = self.queue.clone();
            find_controller.connect_failed_to_find_text(move |_| {
                failed_queue.clone().set_search_match_count(engine_id, 0)
            });
            let success_queue = self.queue.clone();
            find_controller.connect_found_text(move |_, match_count| {
                success_queue.clone().set_search_match_count(engine_id, match_count as usize)
            });
        }

        // Listen for audio playback changes.
        let audio_queue = self.queue.clone();
        web_view.connect_is_playing_audio_notify(move |web_view| {
            audio_queue.clone().set_audio_playing(engine_id, web_view.is_playing_audio());
        });

        // Load adblock content filter.
        load_adblock(web_view.clone());

        WebKitEngine {
            webkit_display,
            web_view,
            scale,
            bg,
            queue: self.queue.clone(),
            id: engine_id,
            buffers_pending_release: Default::default(),
            last_input_position: Default::default(),
            opaque_region: Default::default(),
            buffer_damage: Default::default(),
            buffer_size: Default::default(),
            downloads: Default::default(),
            buffer: Default::default(),
            dirty: Default::default(),
            menu: Default::default(),
        }
    }
}

/// WebKit browser engine.
pub struct WebKitEngine {
    id: EngineId,
    web_view: WebView,
    menu: Option<(OptionMenuId, Menu)>,
    opaque_region: Option<Region>,
    queue: StQueueHandle<State>,

    webkit_display: WebKitDisplay,
    buffer: Option<(WaylandBuffer, BufferDMABuf)>,
    buffer_damage: Option<Vec<WPERectangle>>,
    buffers_pending_release: [Option<(WaylandBuffer, BufferDMABuf)>; MAX_PENDING_BUFFERS],

    buffer_size: Size,
    scale: f64,

    last_input_position: Position<f64>,

    downloads: HashMap<DownloadId, WebKitDownload>,

    bg: [f64; 3],
    dirty: bool,
}

impl WebKitEngine {
    /// Import a new DMA buffer.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn import_buffer(
        &mut self,
        wayland_queue: &QueueHandle<State>,
        dmabuf_state: &DmabufState,
        buffer: BufferDMABuf,
        damage_rects: Vec<WPERectangle>,
    ) {
        let params = match dmabuf_state.create_params(wayland_queue) {
            Ok(params) => params,
            Err(err) => {
                error!("Failed creating params for WebKit buffer: {err}");
                self.cleanup_buffer(&buffer);
                return;
            },
        };

        // Add parameters for each plane.
        let modifier = buffer.modifier();
        for plane_index in 0..buffer.n_planes() {
            let offset = buffer.offset(plane_index);
            let stride = buffer.stride(plane_index);
            let fd = unsafe { BorrowedFd::borrow_raw(buffer.fd(plane_index)) };
            params.add(fd, plane_index, offset, stride, modifier);
        }

        // Create the WlBuffer.
        let width = buffer.width();
        let height = buffer.height();
        let format = buffer.format();
        let flags = DmabufFlags::empty();
        let (wl_buffer, _) = params.create_immed(width, height, format, flags, wayland_queue);
        let wl_buffer = WaylandBuffer(wl_buffer);

        // Ensure buffer was created successfully.
        if !wl_buffer.is_alive() {
            error!("WebKit buffer creation failed");
            self.cleanup_buffer(&buffer);
            return;
        }

        // Setup release tracking for the old buffer.
        if let Some((wl_buffer, dmabuf)) = self.buffer.take() {
            // Assume release if the maximum number of pending buffers is reached.
            self.buffers_pending_release.rotate_right(1);
            if let Some((_, dmabuf)) = self.buffers_pending_release[0].take() {
                self.webkit_display.buffer_released(&dmabuf);
            }

            self.buffers_pending_release[0] = Some((wl_buffer, dmabuf));
        }

        // Update buffer and flag engine as dirty.
        self.buffer_size = Size::new(width as u32, height as u32);
        self.buffer_damage = Some(damage_rects);
        self.buffer = Some((wl_buffer, buffer));
        self.dirty = true;
    }

    /// Dispose buffer that wasn't rendered.
    fn cleanup_buffer(&self, buffer: &impl IsA<Buffer>) {
        self.webkit_display.frame_done(buffer);
        self.webkit_display.buffer_released(buffer);
    }

    /// Update the engine surface's opaque region.
    fn set_opaque_region(&mut self, region: Option<Region>) {
        self.opaque_region = region;
    }

    /// Update engine focus.
    fn set_focused(&mut self, focused: bool) {
        // Force text-input update.
        self.webkit_display.input_method_context().mark_text_input_dirty();

        self.webkit_display.set_focus(focused);
    }
}

impl Engine for WebKitEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn dirty(&mut self) -> bool {
        self.dirty
    }

    fn attach_buffer(&mut self, surface: &WlSurface) -> bool {
        self.dirty = false;

        // Regularly check for background color config changes.
        let bg = CONFIG.read().unwrap().colors.background.as_f64();
        if self.bg != bg {
            let mut color = Color::new(bg[0], bg[1], bg[2], 1.);
            self.web_view.set_background_color(&mut color);
            self.bg = bg;
        }

        match &self.buffer {
            Some((buffer, _)) => {
                surface.attach(Some(buffer), 0, 0);
                true
            },
            None => false,
        }
    }

    fn buffer_size(&self) -> Size {
        self.buffer_size
    }

    fn take_buffer_damage(&mut self) -> Option<Vec<(i32, i32, i32, i32)>> {
        match self.buffer_damage.take() {
            Some(damage) if damage.is_empty() => None,
            Some(damage) => Some(
                damage
                    .into_iter()
                    .map(|damage| (damage.x, damage.y, damage.width, damage.height))
                    .collect(),
            ),
            None => Some(Vec::new()),
        }
    }

    fn frame_done(&mut self) {
        if let Some((_, dmabuf)) = &mut self.buffer {
            self.webkit_display.frame_done(dmabuf);
        }
    }

    fn buffer_released(&mut self, released_buffer: &WlBuffer) {
        // Release matching pending buffer.
        //
        // We intentionally do not check the current buffer here, since it might be
        // attached again in the future and we cannot determine if it should be
        // released or not.
        for i in 0..self.buffers_pending_release.len() {
            let dmabuf = match &self.buffers_pending_release[i] {
                Some((wl_buffer, dmabuf)) if released_buffer == wl_buffer.deref() => dmabuf,
                Some(_) => continue,
                None => break,
            };

            // Notify WebKit about buffer release.
            self.webkit_display.buffer_released(dmabuf);

            // Remove the buffer from the pending buffers.
            self.buffers_pending_release[i] = None;
            self.buffers_pending_release[i..].rotate_left(1);

            break;
        }
    }

    /// Update DMA buffer feedback.
    fn dmabuf_feedback(&mut self, feedback: &DmabufFeedback) {
        self.webkit_display.dmabuf_feedback(feedback);
    }

    /// Get the buffer's opaque region.
    fn opaque_region(&self) -> Option<&WlRegion> {
        self.opaque_region.as_ref().map(|region| region.wl_region())
    }

    fn set_size(&mut self, size: Size) {
        self.webkit_display.set_size(size.width as i32, size.height as i32);
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.webkit_display.set_scale(scale);
    }

    fn press_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        self.webkit_display.key(time, raw, keysym, modifiers, true);
    }

    fn release_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        self.webkit_display.key(time, raw, keysym, modifiers, false);
    }

    fn pointer_axis(
        &mut self,
        time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        modifiers: Modifiers,
    ) {
        self.webkit_display.pointer_axis(time, position, horizontal, vertical, modifiers);
    }

    fn pointer_button(
        &mut self,
        time: u32,
        position: Position<f64>,
        button: u32,
        down: bool,
        modifiers: Modifiers,
    ) {
        self.set_focused(true);
        self.webkit_display.pointer_button(time, position, button, down, modifiers);
        self.last_input_position = position;
    }

    fn pointer_motion(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers) {
        self.webkit_display.pointer_motion(time, position, modifiers, EventType::PointerMove);
    }

    fn pointer_enter(&mut self, position: Position<f64>, modifiers: Modifiers) {
        self.webkit_display.pointer_motion(0, position, modifiers, EventType::PointerEnter);
    }

    fn pointer_leave(&mut self, position: Position<f64>, modifiers: Modifiers) {
        self.webkit_display.pointer_motion(0, position, modifiers, EventType::PointerLeave);
    }

    fn touch_down(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers) {
        self.set_focused(true);
        self.webkit_display.touch(time, id, position, modifiers, EventType::TouchDown);
        self.last_input_position = position;
    }

    fn touch_up(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers) {
        self.webkit_display.touch(time, id, position, modifiers, EventType::TouchUp);
    }

    fn touch_motion(&mut self, time: u32, id: i32, position: Position<f64>, modifiers: Modifiers) {
        self.webkit_display.touch(time, id, position, modifiers, EventType::TouchMove);
    }

    fn load_uri(&mut self, uri: &str) {
        self.web_view.load_uri(uri);
    }

    fn load_prev(&mut self) {
        self.web_view.go_back();
    }

    fn has_prev(&self) -> bool {
        self.web_view.can_go_back()
    }

    fn uri(&self) -> Cow<'_, str> {
        self.web_view.uri().unwrap_or_default().to_string().into()
    }

    fn title(&self) -> Cow<'_, str> {
        self.web_view.title().unwrap_or_default().to_string().into()
    }

    fn text_input_state(&self) -> TextInputChange {
        self.webkit_display.input_method_context().text_input_state()
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        self.webkit_display
            .input_method_context()
            .emit_by_name::<()>("delete-surrounding", &[&before_length, &after_length]);
    }

    fn commit_string(&mut self, text: String) {
        self.webkit_display.input_method_context().emit_by_name::<()>("committed", &[&text]);
    }

    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        self.webkit_display.input_method_context().emit_by_name::<()>("preedit-started", &[]);

        self.webkit_display.input_method_context().set_preedit_string(
            text,
            cursor_begin,
            cursor_end,
        );

        self.webkit_display.input_method_context().emit_by_name::<()>("preedit-changed", &[]);
        self.webkit_display.input_method_context().emit_by_name::<()>("preedit-finished", &[]);
    }

    fn paste(&mut self, text: String) {
        self.commit_string(text);
    }

    fn clear_focus(&mut self) {
        self.set_focused(false);
    }

    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize) {
        if self.menu.as_ref().is_some_and(|(id, _)| *id == menu_id) {
            let (id, menu) = self.menu.take().unwrap();

            // Activate selected option.
            menu.activate_item(self, index as u32);

            // Close our option menu UI.
            self.queue.close_menu(id);
        }
    }

    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>) {
        if let Some((id, menu)) = &self.menu {
            if menu_id.is_none_or(|menu_id| *id == menu_id) {
                // Notify menu about being closed from our end.
                menu.close();

                // Close our option menu UI.
                self.queue.close_menu(*id);

                self.menu = None;
            }
        }
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        self.webkit_display.set_fullscreen(fullscreen);
    }

    fn session(&self) -> Vec<u8> {
        self.web_view
            .session_state()
            .and_then(|session| session.serialize())
            .map_or(Vec::new(), |session| session.to_vec())
    }

    fn restore_session(&self, session: Vec<u8>) {
        let session = WebViewSessionState::new(&Bytes::from_owned(session));
        self.web_view.restore_session_state(&session);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn favicon(&self) -> Option<Favicon> {
        let resource_uri = self.favicon_uri()?;
        let favicon = self.web_view.favicon()?;

        let bytes = favicon.bytes()?;
        let width = favicon.width();
        let height = favicon.height();

        (width > 0 && height > 0).then_some(Favicon {
            resource_uri,
            bytes,
            width: width as usize,
            height: height as usize,
        })
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn favicon_uri(&self) -> Option<glib::GString> {
        let data_manager = self.web_view.network_session()?.website_data_manager()?;
        let favicon_database = data_manager.favicon_database()?;
        favicon_database.favicon_uri(&self.uri())
    }

    fn cancel_download(&mut self, download_id: DownloadId) {
        if let Some(download) = self.downloads.get(&download_id) {
            download.cancel();
        }
    }

    fn set_zoom_level(&mut self, zoom_level: f64) {
        self.web_view.set_zoom_level(zoom_level);
    }

    fn zoom_level(&self) -> f64 {
        self.web_view.zoom_level()
    }

    /// Start or update a text search.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn update_search(&mut self, text: &str) {
        if let Some(find_controller) = self.web_view.find_controller() {
            // Setup search options to use smart case and wrapping.
            let mut options = FindOptions::WRAP_AROUND;
            if text.chars().all(|c| c.is_lowercase()) {
                options |= FindOptions::CASE_INSENSITIVE;
            }

            find_controller.search(text, options.bits(), MAX_MATCH_COUNT);
        }
    }

    /// Stop all active text searches.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn stop_search(&mut self) {
        if let Some(find_controller) = self.web_view.find_controller() {
            find_controller.search_finish();
        }
    }

    /// Go to the next search match.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn search_next(&mut self) {
        if let Some(find_controller) = self.web_view.find_controller() {
            find_controller.search_next();
        }
    }

    /// Go to the previous search match.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn search_prev(&mut self) {
        if let Some(find_controller) = self.web_view.find_controller() {
            find_controller.search_previous();
        }
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Get WebKit network session using XDG-based backing storage.
#[cfg_attr(feature = "profiling", profiling::function)]
fn xdg_network_session(
    cookie_whitelist: &CookieWhitelist,
    engine_id: EngineId,
    queue: StQueueHandle<State>,
) -> Option<NetworkSession> {
    // Create the network session using kumo-suffixed XDG directories.
    let group_id = engine_id.group_id().uuid().to_string();
    let cache_dir = cache_dir()?.join("groups").join(&group_id);
    let data_dir = data_dir()?.join("groups").join(&group_id);
    let network_session = NetworkSession::new(Some(cache_dir.to_str()?), Some(data_dir.to_str()?));

    // Propagate download updates.
    network_session.connect_download_started(move |_network_session, download| {
        // Start download and find a non-conflicting download destination.
        let id = DownloadId::new(engine_id);
        let destination_queue = queue.clone();
        download.connect_decide_destination(move |download, suggested_destination| {
            // Get XDG download directory.
            let download_dir = glib::user_special_dir(UserDirectory::Downloads)
                .unwrap_or_else(|| PathBuf::from("/tmp"));

            // Try adding suffixes to filename until unused path is found.
            let mut suffix = 0;
            let destination = loop {
                let name = if suffix == 0 {
                    suggested_destination.to_string()
                } else {
                    format!("{suggested_destination}_{suffix:x}")
                };
                suffix += 1;

                let destination = download_dir.join(&name);
                if !destination.exists() {
                    break destination;
                }
            };

            // Since the GIR bindings expect a `&str`, we only support utf8 paths.
            match destination.to_str() {
                Some(destination) => download.set_destination(destination),
                None => download.cancel(),
            }

            // Officially start the download process.
            destination_queue.clone().add_webkit_download(id, download.clone());

            false
        });

        // Handle download progress updates.
        let progress_queue = queue.clone();
        download.connect_estimated_progress_notify(move |download| {
            let progress = (download.estimated_progress() * 100.).ceil() as u8;
            progress_queue.clone().set_download_progress(id, Some(progress));
        });

        // Handle completion and WebKit cache cleanup.
        let finished_queue = queue.clone();
        download.connect_finished(move |_| {
            // Ensure download is always marked as completed.
            let mut queue = finished_queue.clone();
            queue.set_download_progress(id, Some(100));

            // Avoid WebKit download cache memory leak.
            queue.remove_webkit_download(id);
        });

        // Handle download failure updates.
        let download_queue = queue.clone();
        download.connect_failed(move |download, err| {
            let uri = download.request().and_then(|request| request.uri());
            error!(?uri, "Download failed: {err}");
            download_queue.clone().set_download_progress(id, None);
        });
    });

    // Setup SQLite cookie storage in xdg data dir.
    let cookie_manager = network_session.cookie_manager()?;
    let cookies_path = data_dir.join("cookies.sqlite");
    cookie_manager.set_persistent_storage(cookies_path.to_str()?, CookiePersistentStorage::Sqlite);

    // Prohibit third-party cookies.
    cookie_manager.set_accept_policy(CookieAcceptPolicy::NoThirdParty);

    // Delete all persistent data for websites without cookie exception.
    let persistent_types = WebsiteDataTypes::ALL & !WebsiteDataTypes::DISK_CACHE;
    let whitelisted = cookie_whitelist.hosts();
    let website_data_manager = network_session.website_data_manager().unwrap();
    let disk_cache_data_manager = website_data_manager.clone();
    website_data_manager.clone().fetch(persistent_types, None::<&Cancellable>, move |data| {
        let mut data = match data {
            Ok(data) => data,
            Err(_) => return,
        };

        // Filter data to only include domains which aren't whitelisted.
        data.retain(|data| {
            data.name().is_some_and(|domain| whitelisted.iter().all(|host| host != &domain))
        });

        // Remove all non-whitelisted data.
        if !data.is_empty() {
            let data: Vec<_> = data.iter().collect();
            website_data_manager.remove(persistent_types, &data, None::<&Cancellable>, |_| {});
        }
    });

    // Separately clear disk caches after a certain age.
    disk_cache_data_manager.clear(
        WebsiteDataTypes::DISK_CACHE,
        TimeSpan::from_days(MAX_DISK_CACHE_DAYS),
        None::<&Cancellable>,
        |_| {},
    );

    Some(network_session)
}

/// Load the content filter for adblocking.
fn load_adblock(web_view: WebView) {
    // Initialize content filter cache at the default user data directory.
    let filter_dir = match data_dir() {
        Some(data_dir) => data_dir.join("content_filters"),
        None => {
            warn!("Missing user data directory, skipping adblock setup");
            return;
        },
    };
    let filter_store = match filter_dir.to_str() {
        Some(filter_dir) => UserContentFilterStore::new(filter_dir),
        None => {
            warn!("Non-utf8 user data directory ({filter_dir:?}), skipping adblock setup");
            return;
        },
    };

    // Attempt to load the adblock filter from the cache.
    filter_store.clone().load(ADBLOCK_FILTER_ID, None::<&Cancellable>, move |filter| {
        let content_manager = web_view.user_content_manager().unwrap();

        // If the filter was in the cache, just add it to the content manager.
        if let Ok(filter) = filter {
            content_manager.add_filter(&filter);
            trace!("Successfully initialized adblock filter from cache");
            return;
        }

        // Load filter into the cache, then add it to the content manager.
        let filter_bytes = Bytes::from_static(include_bytes!("../../../adblock.json"));
        filter_store.save(ADBLOCK_FILTER_ID, &filter_bytes, None::<&Cancellable>, move |filter| {
            match filter {
                Ok(filter) => content_manager.add_filter(&filter),
                Err(err) => error!("Could not load adblock filter: {err}"),
            }
        });
    });
}

/// WlBuffer wrapper to ensure `destroy()` is called on drop.
struct WaylandBuffer(WlBuffer);

impl Drop for WaylandBuffer {
    fn drop(&mut self) {
        self.destroy()
    }
}

impl Deref for WaylandBuffer {
    type Target = WlBuffer;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// WebKit popup menu.
#[derive(Debug)]
enum Menu {
    /// Right-click menu.
    ContextMenu(ContextMenu),
    /// Dropdown menu.
    OptionMenu(OptionMenu),
}

impl Menu {
    /// Number of items in the menu.
    fn n_items(&self) -> u32 {
        match self {
            Self::ContextMenu(menu) => menu.n_items(),
            Self::OptionMenu(menu) => menu.n_items(),
        }
    }

    /// Get the item at the specified index.
    fn item(&self, index: u32) -> Option<MenuItem> {
        match self {
            Self::ContextMenu(menu) => {
                let item = menu.item(index)?;
                Some(MenuItem::ContextMenuItem(item))
            },
            Self::OptionMenu(menu) => {
                let item = menu.item(index)?;
                Some(MenuItem::OptionMenuItem(item))
            },
        }
    }

    /// Listen for menu close.
    fn connect_close<F>(&self, f: F)
    where
        F: Fn() + 'static,
    {
        match self {
            Self::ContextMenu(_) => (),
            Self::OptionMenu(menu) => {
                menu.connect_close(move |_| f());
            },
        }
    }

    /// Notify menu about its termination.
    fn close(&self) {
        match self {
            Self::ContextMenu(_) => (),
            Self::OptionMenu(menu) => menu.close(),
        }
    }

    /// Activate item at the specified position.
    fn activate_item(&self, engine: &mut WebKitEngine, index: u32) {
        match self {
            Self::ContextMenu(menu) => menu.activate_item(engine, index),
            Self::OptionMenu(menu) => menu.activate_item(index),
        }
    }
}

impl From<&OptionMenu> for Menu {
    fn from(menu: &OptionMenu) -> Menu {
        Self::OptionMenu(menu.clone())
    }
}

/// WebKit popup menu item.
enum MenuItem {
    /// Right-click menu item.
    ContextMenuItem(ContextMenuItem),
    /// Dropdown menu item.
    OptionMenuItem(WebKitOptionMenuItem),
}

impl MenuItem {
    /// Get the text of the item.
    fn label<'a>(&mut self) -> Option<Cow<'a, str>> {
        match self {
            Self::ContextMenuItem(item) => Some(item.label().into()),
            Self::OptionMenuItem(item) => item.label().map(|label| label.to_string().into()),
        }
    }

    /// Check whether an item is enabled.
    fn is_enabled(&mut self) -> bool {
        match self {
            Self::ContextMenuItem(_) => true,
            Self::OptionMenuItem(item) => item.is_enabled(),
        }
    }

    /// Check whether an item is selected.
    fn is_selected(&mut self) -> bool {
        match self {
            Self::ContextMenuItem(_) => false,
            Self::OptionMenuItem(item) => item.is_selected(),
        }
    }
}

/// Custom context menu based on a hit test.
#[derive(Debug)]
struct ContextMenu {
    hit_test_result: HitTestResult,
    context: HitTestResultContext,
    has_cookie_exception: bool,
    host: Option<GString>,
}

impl ContextMenu {
    fn new(cookie_whitelist: &CookieWhitelist, uri: &str, hit_test_result: HitTestResult) -> Self {
        let context = HitTestResultContext::from_bits(hit_test_result.context())
            .unwrap_or(HitTestResultContext::empty());

        let mut context_menu = Self {
            hit_test_result,
            context,
            has_cookie_exception: Default::default(),
            host: Default::default(),
        };

        // Set correct cookie exception message if we are going to display it.
        if context == HitTestResultContext::DOCUMENT {
            if let Some(host) = Uri::parse(uri, UriFlags::NONE).ok().and_then(|uri| uri.host()) {
                context_menu.has_cookie_exception = cookie_whitelist.contains(&host);
                context_menu.host = Some(host);
            }
        }

        context_menu
    }

    /// Number of items in the menu.
    fn n_items(&self) -> u32 {
        let mut n_items = 0;

        // Only show these entries without any targeted element.
        if self.context == HitTestResultContext::DOCUMENT {
            n_items += 2;
        }

        if self.context.contains(HitTestResultContext::DOCUMENT) {
            n_items += 1;
        }

        if self.context.intersects(
            HitTestResultContext::LINK | HitTestResultContext::IMAGE | HitTestResultContext::MEDIA,
        ) {
            n_items += 3;
        }

        if self.context.contains(HitTestResultContext::EDITABLE) {
            n_items += 1;
        }

        n_items
    }

    /// Get the item at the specified index.
    #[allow(clippy::single_match)]
    fn item(&self, mut index: u32) -> Option<ContextMenuItem> {
        // Only show these entries without any targeted element.
        if self.context == HitTestResultContext::DOCUMENT {
            match index {
                0 if self.has_cookie_exception => {
                    return Some(ContextMenuItem::RemoveCookieException);
                },
                0 => return Some(ContextMenuItem::AddCookieException),
                1 => return Some(ContextMenuItem::Search),
                _ => (),
            }
            index -= 2;
        }

        if self.context.contains(HitTestResultContext::DOCUMENT) {
            match index {
                0 => return Some(ContextMenuItem::Reload),
                _ => (),
            }
            index -= 1;
        }

        if self.context.intersects(
            HitTestResultContext::LINK | HitTestResultContext::IMAGE | HitTestResultContext::MEDIA,
        ) {
            match index {
                0 => return Some(ContextMenuItem::CopyLink),
                1 => return Some(ContextMenuItem::OpenInNewWindow),
                2 => return Some(ContextMenuItem::OpenInNewTab),
                _ => (),
            }
            index -= 3;
        }

        if self.context.contains(HitTestResultContext::EDITABLE) {
            match index {
                0 => return Some(ContextMenuItem::Paste),
                _ => (),
            }
            // index -= 1;
        }

        None
    }

    /// Activate item at the specified position.
    fn activate_item(&self, engine: &mut WebKitEngine, index: u32) {
        match self.item(index) {
            Some(ContextMenuItem::AddCookieException) => {
                if let Some(host) = &self.host {
                    engine.queue.add_cookie_exception(host.to_string());
                }
            },
            Some(ContextMenuItem::RemoveCookieException) => {
                if let Some(host) = &self.host {
                    engine.queue.remove_cookie_exception(host.to_string());
                }
            },
            Some(ContextMenuItem::Search) => engine.queue.start_search(engine.id),
            Some(ContextMenuItem::Reload) => engine.web_view.reload(),
            Some(ContextMenuItem::OpenInNewTab) => {
                if let Some(uri) = self.uri() {
                    engine.queue.open_in_tab(engine.id, uri);
                }
            },
            Some(ContextMenuItem::OpenInNewWindow) => {
                if let Some(uri) = self.uri() {
                    engine.queue.open_in_window(engine.id, uri);
                }
            },
            Some(ContextMenuItem::CopyLink) => {
                if let Some(uri) = self.uri() {
                    engine.queue.set_clipboard(uri);
                }
            },
            Some(ContextMenuItem::Paste) => {
                engine.queue.request_paste(PasteTarget::Browser(engine.id()))
            },
            None => (),
        }
    }

    /// Get URI for the context menu's target resourc.
    fn uri(&self) -> Option<String> {
        if self.context.contains(HitTestResultContext::LINK) {
            self.hit_test_result.link_uri().map(String::from)
        } else if self.context.contains(HitTestResultContext::IMAGE) {
            self.hit_test_result.image_uri().map(String::from)
        } else if self.context.contains(HitTestResultContext::MEDIA) {
            self.hit_test_result.media_uri().map(String::from)
        } else {
            None
        }
    }
}

/// Custom context menu item based on a hit test.
#[derive(Debug)]
enum ContextMenuItem {
    AddCookieException,
    RemoveCookieException,
    Reload,
    OpenInNewTab,
    OpenInNewWindow,
    CopyLink,
    Paste,
    Search,
}

impl ContextMenuItem {
    fn label(&self) -> &'static str {
        match self {
            Self::AddCookieException => "Add Cookie Exception",
            Self::RemoveCookieException => "Remove Cookie Exception",
            Self::Reload => "Reload",
            Self::OpenInNewTab => "Open in New Tab",
            Self::OpenInNewWindow => "Open in New Window",
            Self::CopyLink => "Copy Link",
            Self::Paste => "Paste",
            Self::Search => "Search Page",
        }
    }
}

/// Get base data directory.
fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("kumo/default"))
}

/// Get base cache directory.
fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("kumo/default"))
}

/// Get and downcast a WebKit engine from a window.
fn webkit_engine_by_id(window: &mut Window, engine_id: EngineId) -> Option<&mut WebKitEngine> {
    let engine = window.tab_mut(engine_id)?;
    engine.as_any().downcast_mut::<WebKitEngine>()
}
