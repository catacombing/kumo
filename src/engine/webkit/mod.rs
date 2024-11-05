//! WebKit browser engine.

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Deref;
use std::os::fd::BorrowedFd;
use std::rc::Rc;

use _dmabuf::zwp_linux_buffer_params_v1::Flags as DmabufFlags;
use funq::StQueueHandle;
use gio::Cancellable;
use glib::object::{Cast, ObjectExt};
use glib::prelude::*;
use glib::{Bytes, GString, Uri, UriFlags};
use glutin::display::Display;
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::dmabuf::{DmabufFeedback, DmabufState};
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::reexports::client::{Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::linux_dmabuf::zv1::client as _dmabuf;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use tracing::{error, trace, warn};
use wpe_platform::ffi::WPERectangle;
use wpe_platform::{Buffer, BufferDMABuf, BufferExt, BufferSHM, EventType};
use wpe_webkit::{
    Color, CookieAcceptPolicy, CookiePersistentStorage, HitTestResult, HitTestResultContext,
    NetworkSession, OptionMenu, OptionMenuItem as WebKitOptionMenuItem, UserContentFilterStore,
    WebView, WebViewExt, WebViewSessionState,
};

use crate::engine::webkit::platform::WebKitDisplay;
use crate::engine::{Engine, EngineId, BG};
use crate::storage::cookie_whitelist::CookieWhitelist;
use crate::ui::overlay::option_menu::{Anchor, OptionMenuId, OptionMenuItem, OptionMenuPosition};
use crate::window::TextInputChange;
use crate::{KeyboardFocus, Position, Size, State, WindowId};

mod input_method_context;
mod platform;

/// Content filter store ID for the adblock json.
const ADBLOCK_FILTER_ID: &str = "adblock";

/// Maximum number of buffers kept for release tracking.
///
/// If the number of buffers pending release exceeds this number,
/// then the oldest buffer is automatically assumed to be released.
const MAX_PENDING_BUFFERS: usize = 3;

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

    /// Update current URI for an engine.
    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String);

    /// Update current title for an engine.
    fn set_engine_title(&mut self, engine_id: EngineId, title: String);

    /// Open popup.
    fn open_menu(&mut self, engine_id: EngineId, menu: Menu, rect: Option<(i32, i32, i32, i32)>);

    /// Close popup.
    fn close_menu(&mut self, menu_id: OptionMenuId);

    /// Handle fullscreen enter/leave.
    fn set_fullscreen(&mut self, engine_id: EngineId, enable: bool);

    /// Open URI in a new tab.
    fn open_in_tab(&mut self, window_id: WindowId, uri: String);

    /// Open URI in a new window.
    fn open_in_window(&mut self, uri: String);

    /// Write text to the system clipboard.
    fn set_clipboard(&mut self, text: String);

    /// Add host to the cookie whitelist.
    fn add_cookie_exception(&mut self, host: String);

    /// Remove host from the cookie whitelist.
    fn remove_cookie_exception(&mut self, host: String);
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
        let engine = match window.tabs_mut().get_mut(&engine_id) {
            Some(engine) => engine,
            None => return,
        };
        let webkit_engine = match engine.as_any().downcast_mut::<WebKitEngine>() {
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
        if window.active_tab() == engine_id {
            window.unstall();
        }
    }

    fn set_opaque_rectangles(&mut self, engine_id: EngineId, rects: Vec<WPERectangle>) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine = match window.tabs_mut().get_mut(&engine_id) {
            Some(engine) => engine,
            None => return,
        };
        let webkit_engine = match engine.as_any().downcast_mut::<WebKitEngine>() {
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

    fn set_engine_uri(&mut self, engine_id: EngineId, uri: String) {
        let window_id = engine_id.window_id();

        if let Some(window) = self.windows.get_mut(&window_id) {
            window.set_engine_uri(&self.storage, engine_id, uri);
        }
    }

    fn set_engine_title(&mut self, engine_id: EngineId, title: String) {
        let window_id = engine_id.window_id();

        if let Some(window) = self.windows.get_mut(&window_id) {
            window.set_engine_title(&self.storage.history, engine_id, title);
        }
    }

    fn open_menu(&mut self, engine_id: EngineId, menu: Menu, rect: Option<(i32, i32, i32, i32)>) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine = match window.tabs_mut().get_mut(&engine_id) {
            Some(engine) => engine,
            None => return,
        };
        let webkit_engine = match engine.as_any().downcast_mut::<WebKitEngine>() {
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
                let position = webkit_engine.last_input_position;
                let position = Position::new(position.x.round() as i32, position.y.round() as i32);
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
        let window = match self.windows.get_mut(&menu_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine_id = match menu_id.engine_id() {
            Some(engine_id) => engine_id,
            None => return,
        };
        let engine = match window.tabs_mut().get_mut(&engine_id) {
            Some(engine) => engine,
            None => return,
        };
        let webkit_engine = match engine.as_any().downcast_mut::<WebKitEngine>() {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Clear engine's option menu if it matches the menu's ID.
        if webkit_engine.menu.as_ref().map_or(false, |(id, _)| menu_id == *id) {
            webkit_engine.menu = None;
        }

        window.close_option_menu(menu_id);
    }

    fn set_fullscreen(&mut self, engine_id: EngineId, enable: bool) {
        if let Some(window) = self.windows.get_mut(&engine_id.window_id()) {
            window.request_fullscreen(engine_id, enable);
        }
    }

    fn open_in_tab(&mut self, window_id: WindowId, uri: String) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        if let Some(engine) =
            window.add_tab(false, false).ok().and_then(|id| window.tabs().get(&id))
        {
            engine.load_uri(&uri);
        }
    }

    fn open_in_window(&mut self, uri: String) {
        let window = match self.create_window().ok().and_then(|id| self.windows.get_mut(&id)) {
            Some(window) => window,
            None => return,
        };

        window.set_keyboard_focus(KeyboardFocus::None);
        if let Some(engine) = window.tabs().get(&window.active_tab()) {
            engine.load_uri(&uri);
        }
    }

    fn set_clipboard(&mut self, text: String) {
        self.set_clipboard(text);
    }

    fn add_cookie_exception(&mut self, host: String) {
        self.storage.cookie_whitelist.add(&host);
    }

    fn remove_cookie_exception(&mut self, host: String) {
        self.storage.cookie_whitelist.remove(&host);
    }
}

/// WebKit shared engine state.
pub struct WebKitState {
    pub dmabuf_feedback: Rc<RefCell<Option<DmabufFeedback>>>,

    cookie_whitelist: CookieWhitelist,
    network_session: NetworkSession,
    queue: StQueueHandle<State>,
    display: Display,
}

impl WebKitState {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new(
        display: Display,
        queue: StQueueHandle<State>,
        cookie_whitelist: CookieWhitelist,
    ) -> Self {
        let network_session =
            xdg_network_session(&cookie_whitelist).unwrap_or_else(NetworkSession::new_ephemeral);

        Self {
            cookie_whitelist,
            network_session,
            display,
            queue,
            dmabuf_feedback: Default::default(),
        }
    }

    /// Create a new WebKit engine from this state.
    pub fn create_engine(
        &self,
        engine_id: EngineId,
        size: Size,
        scale: f64,
    ) -> Result<WebKitEngine, WebKitError> {
        // Get the DRM render node.
        let Display::Egl(egl_display) = &self.display;
        let device = egl_display.device().expect("get DRM device");
        let render_node = device.drm_render_device_node_path().expect("get render node");

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
        let web_view = WebView::builder()
            .network_session(&self.network_session)
            .display(&webkit_display)
            .build();

        // Set browser background color.
        let mut color = Color::new(BG[0], BG[1], BG[2], 1.);
        web_view.set_background_color(&mut color);

        // Notify UI about URI and title changes.
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

        // Load adblock content filter.
        load_adblock(web_view.clone());

        Ok(WebKitEngine {
            webkit_display,
            web_view,
            scale,
            queue: self.queue.clone(),
            id: engine_id,
            buffers_pending_release: Default::default(),
            last_input_position: Default::default(),
            opaque_region: Default::default(),
            buffer_damage: Default::default(),
            buffer_size: Default::default(),
            buffer: Default::default(),
            dirty: Default::default(),
            menu: Default::default(),
        })
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

    fn dirty(&self) -> bool {
        self.dirty
    }

    fn wl_buffer(&self) -> Option<&WlBuffer> {
        self.buffer.as_ref().map(|(wl_buffer, _)| wl_buffer.deref())
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
        self.dirty = false;

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

    fn load_uri(&self, uri: &str) {
        self.web_view.load_uri(uri);
    }

    fn load_prev(&self) {
        self.web_view.go_back();
    }

    fn uri(&self) -> String {
        self.web_view.uri().unwrap_or_default().into()
    }

    fn title(&self) -> String {
        self.web_view.title().unwrap_or_default().into()
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

    fn clear_focus(&mut self) {
        self.set_focused(false);
    }

    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize) {
        if self.menu.as_ref().map_or(false, |(id, _)| *id == menu_id) {
            let (id, menu) = self.menu.take().unwrap();

            // Activate selected option.
            menu.activate_item(self, index as u32);

            // Close our option menu UI.
            self.queue.close_menu(id);
        }
    }

    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>) {
        if let Some((id, menu)) = &self.menu {
            if menu_id.map_or(true, |menu_id| *id == menu_id) {
                // Notify menu about being closed from our end.
                menu.close();

                // Close our option menu UI.
                self.queue.close_menu(*id);

                self.menu = None;
            }
        }
    }

    fn set_fullscreen(&mut self, fullscreened: bool) {
        self.webkit_display.set_fullscreen(fullscreened);
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

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Get WebKit network session using XDG-based backing storage.
#[cfg_attr(feature = "profiling", profiling::function)]
fn xdg_network_session(cookie_whitelist: &CookieWhitelist) -> Option<NetworkSession> {
    // Create the network session using kumo-suffixed XDG directories.
    let data_dir = dirs::data_dir()?.join("kumo/default");
    let cache_dir = dirs::cache_dir()?.join("kumo/default");
    let network_session = NetworkSession::new(Some(data_dir.to_str()?), Some(cache_dir.to_str()?));

    // Setup SQLite cookie storage in xdg data dir.
    let cookie_manager = network_session.cookie_manager()?;
    let cookies_path = data_dir.join("cookies.sqlite");
    cookie_manager.set_persistent_storage(cookies_path.to_str()?, CookiePersistentStorage::Sqlite);

    // Prohibit third-party cookies.
    cookie_manager.set_accept_policy(CookieAcceptPolicy::NoThirdParty);

    // Delete all non-whitelisted cookies on startup.
    let whitelisted = cookie_whitelist.hosts();
    cookie_manager.clone().all_cookies(None::<&Cancellable>, move |cookies| {
        let mut cookies = match cookies {
            Ok(cookies) => cookies,
            Err(_) => return,
        };

        for cookie in &mut cookies {
            if !whitelisted.iter().any(|host| cookie.domain_matches(host)) {
                cookie_manager.delete_cookie(cookie, None::<&Cancellable>, |_| {});
            }
        }
    });

    Some(network_session)
}

/// Load the content filter for adblocking.
fn load_adblock(web_view: WebView) {
    // Initialize content filter cache at the default user data directory.
    let filter_dir = match dirs::data_dir() {
        Some(data_dir) => data_dir.join("kumo/default/content_filters"),
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
            n_items += 1;
        }

        if self.context.contains(HitTestResultContext::DOCUMENT) {
            n_items += 1;
        }

        if self.context.intersects(
            HitTestResultContext::LINK | HitTestResultContext::IMAGE | HitTestResultContext::MEDIA,
        ) {
            n_items += 3;
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
                    return Some(ContextMenuItem::RemoveCookieException)
                },
                0 => return Some(ContextMenuItem::AddCookieException),
                _ => (),
            }
            index -= 1;
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
            // index -= 3;
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
            Some(ContextMenuItem::Reload) => engine.web_view.reload(),
            Some(ContextMenuItem::OpenInNewTab) => {
                if let Some(uri) = self.uri() {
                    let window_id = engine.id.window_id();
                    engine.queue.open_in_tab(window_id, uri);
                }
            },
            Some(ContextMenuItem::OpenInNewWindow) => {
                if let Some(uri) = self.uri() {
                    engine.queue.open_in_window(uri);
                }
            },
            Some(ContextMenuItem::CopyLink) => {
                if let Some(uri) = self.uri() {
                    engine.queue.set_clipboard(uri);
                }
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
        }
    }
}
