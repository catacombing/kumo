//! Servo browser engine.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::LazyLock;
use std::{env, mem};

use _text_input::zwp_text_input_v3::ContentPurpose;
use euclid::Scale;
use funq::MtQueueHandle;
use keyboard_types::{Code, Key, KeyState, KeyboardEvent, Location, Modifiers as ServoModifiers};
use raw_window_handle::{
    DisplayHandle, RawDisplayHandle, RawWindowHandle, WaylandWindowHandle, WindowHandle,
};
use servo::{
    ClipboardDelegate, CompositionEvent, CompositionState, ContextMenu as ServoContextMenu,
    EmbedderControl, EmbedderControlId, EventLoopWaker, ImeEvent, InputEvent, InputMethodType,
    LoadStatus, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, Opts,
    PixelFormat, Preferences, RenderingContext, Servo, ServoBuilder, StringRequest, TouchEvent,
    TouchEventType, TouchId, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent,
    WheelMode, WindowRenderingContext,
};
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use surfman::error::Error as SurfmanError;
use tracing::{debug, error};
use url::Url;
use xkbcommon_dl::{XkbCommon, xkbcommon_handle};

use crate::config::CONFIG;
use crate::engine::{ContextMenu, Engine, EngineHandler, EngineId, EngineType, Favicon, FaviconId};
use crate::storage::cookie_whitelist::CookieWhitelist;
use crate::ui::overlay::option_menu::{Anchor, OptionMenuId, OptionMenuPosition};
use crate::window::{self, PasteTarget, TextInputChange, TextInputState, WindowHandler, WindowId};
use crate::{Position, Size, State, Window, gl};

#[funq::callbacks(State)]
trait ServoHandler {
    /// Handle wakeup from Servo event loop.
    fn wakeup(&mut self);

    /// Handle frame ready events.
    fn frame(&mut self, engine_id: EngineId);

    /// Open popup.
    fn open_menu(&mut self, engine_id: EngineId, menu: ServoContextMenu);

    /// Close popup.
    fn close_menu(&mut self, menu_id: OptionMenuId);

    /// Handle Servo embedder control closure.
    fn close_embedder_control(&mut self, engine_id: EngineId, id: EmbedderControlId);

    /// Update IME state.
    fn set_text_input_state(
        &mut self,
        engine_id: EngineId,
        id: EmbedderControlId,
        text_input_state: TextInputState,
        full_text: String,
    );

    /// Handle animation state changes.
    fn set_animating(&mut self, engine_id: EngineId, animating: bool);
}

impl ServoHandler for State {
    fn wakeup(&mut self) {
        self.servo_state.get().servo.spin_event_loop();
    }

    fn frame(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let servo_engine = match servo_engine_by_id(window, engine_id) {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        servo_engine.dirty = true;

        // Wakeup renderer if this engine is active.
        if window.active_tab().is_some_and(|tab| tab.id() == engine_id) {
            window.unstall();
        }
    }

    fn open_menu(&mut self, engine_id: EngineId, menu: ServoContextMenu) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let servo_engine = match servo_engine_by_id(window, engine_id) {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        // Create generic context menu from Servo menu.
        let engine_url = servo_engine.uri();
        let menu_target = menu.element_info();
        let target_flags = menu_target.flags.into();
        let target_url = menu_target.link_url.as_ref().or(menu_target.image_url.as_ref());
        let context_menu = ContextMenu::new(
            #[cfg(feature = "webkit")]
            EngineType::Servo,
            target_flags,
            &servo_engine.cookie_whitelist,
            &engine_url,
            target_url.map(|url| url.to_string()),
        );
        let items = context_menu.items();

        // Get popup position.
        let menu_rect = menu.position();
        let menu_position = OptionMenuPosition::new(menu_rect.min.into(), Anchor::BottomRight);
        let item_width = match menu_rect.max.x - menu_rect.min.x {
            item_width @ 1.. => Some(item_width as u32),
            _ => None,
        };

        // Update engine's active popup for close/activate handling.
        let menu_id = OptionMenuId::with_engine(engine_id);
        if let Some((menu_id, ..)) = &servo_engine.menu {
            servo_engine.close_option_menu(Some(*menu_id));
        }
        servo_engine.menu = Some((menu_id, menu.id(), menu, context_menu));

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
        let servo_engine = match servo_engine_by_id(window, engine_id) {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        // Clear engine's option menu if it matches the menu's ID.
        if servo_engine.menu.as_ref().is_some_and(|(id, ..)| menu_id == *id) {
            servo_engine.menu = None;
        }

        window.close_option_menu(menu_id);
    }

    fn close_embedder_control(&mut self, engine_id: EngineId, id: EmbedderControlId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let servo_engine = match servo_engine_by_id(window, engine_id) {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        match (&servo_engine.menu, servo_engine.text_input_id) {
            // Handle context menu dismissal.
            (&Some((menu_id, embedder_id, ..)), _) if embedder_id == id => {
                servo_engine.menu = None;
                window.close_option_menu(menu_id);
            },
            // Handle IME dismissal.
            (None, Some(embedder_id)) if embedder_id == id => {
                servo_engine.text_input_id = None;
                servo_engine.text_input_change = TextInputChange::Disabled;
                window.unstall();
            },
            _ => (),
        }
    }

    fn set_text_input_state(
        &mut self,
        engine_id: EngineId,
        id: EmbedderControlId,
        text_input_state: TextInputState,
        full_text: String,
    ) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let servo_engine = match servo_engine_by_id(window, engine_id) {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        // Update current text field state.
        servo_engine.text_input_cursor = text_input_state.cursor_index as u32;
        servo_engine.text_input_text = full_text;

        // Update IME state.
        servo_engine.text_input_change = TextInputChange::Dirty(text_input_state);
        servo_engine.text_input_id = Some(id);

        window.unstall();
    }

    fn set_animating(&mut self, engine_id: EngineId, animating: bool) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine = match window.tab_mut(engine_id) {
            Some(engine) => engine,
            None => return,
        };
        let servo_engine = match engine.as_any().downcast_mut::<ServoEngine>() {
            Some(servo_engine) => servo_engine,
            None => return,
        };

        servo_engine.animating = animating;
        window.unstall();
    }
}

/// Global Servo engine state.
pub struct ServoState {
    rendering_contexts: HashMap<WindowId, Rc<dyn RenderingContext>>,
    cookie_whitelist: CookieWhitelist,
    queue: MtQueueHandle<State>,
    display: RawDisplayHandle,
    servo: Rc<Servo>,
}

impl ServoState {
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn new(
        display: RawDisplayHandle,
        queue: MtQueueHandle<State>,
        cookie_whitelist: CookieWhitelist,
    ) -> Self {
        let config = CONFIG.read().unwrap();
        let [bg_r, bg_g, bg_b] = config.colors.background.as_f64();
        let monospace_font = config.font.monospace_family.to_string();
        let font = config.font.family.to_string();
        let font_size = config.font.size.round() as i64;

        // Initialize Servo engine.
        let waker = Box::new(Waker::new(queue.clone()));
        let servo = Rc::new(
            ServoBuilder::default()
                .preferences(Preferences {
                    shell_background_color_rgba: [bg_r, bg_g, bg_b, 1.],
                    fonts_default_monospace_size: font_size,
                    fonts_default_size: font_size,
                    fonts_monospace: monospace_font,
                    fonts_sans_serif: font.clone(),
                    fonts_default: font.clone(),
                    fonts_serif: font,
                    ..Default::default()
                })
                .opts(Opts { hard_fail: false, ..Default::default() })
                .event_loop_waker(waker)
                .build(),
        );

        Self { cookie_whitelist, display, queue, servo, rendering_contexts: Default::default() }
    }

    /// Create a new Servo engine from this state.
    pub fn create_engine(
        &mut self,
        surface: &WlSurface,
        id: EngineId,
        size: Size,
        scale: f64,
        url: Option<&str>,
    ) -> Result<ServoEngine, SurfmanError> {
        // Create a new rendering context for this web view.
        let size = size * scale;
        let entry = self.rendering_contexts.entry(id.window_id());
        let renderer = match entry {
            Entry::Occupied(occupied) => occupied.into_mut(),
            Entry::Vacant(vacant) => {
                // Get handles to Wayland window and display.
                let surface_ptr = NonNull::new(surface.id().as_ptr().cast()).unwrap();
                let wayland_window = WaylandWindowHandle::new(surface_ptr);
                let raw_window = RawWindowHandle::Wayland(wayland_window);
                let window = unsafe { WindowHandle::borrow_raw(raw_window) };
                let display = unsafe { DisplayHandle::borrow_raw(self.display) };

                let size = size.into();
                let context = match WindowRenderingContext::new(display, window, size) {
                    Ok(context) => context,
                    // Retry with GLES to support older hardware.
                    Err(_) => {
                        unsafe { env::set_var("SURFMAN_FORCE_GLES", "1") };
                        WindowRenderingContext::new(display, window, size)?
                    },
                };

                vacant.insert(Rc::new(context))
            },
        };

        // Create this engine's servo web view.
        let url = url
            .and_then(|url| Url::parse(url).ok())
            .unwrap_or_else(|| Url::parse("about:blank").unwrap());
        let web_view_handler = Rc::new(WebViewHandler::new(self.queue.clone(), id));
        let web_view = WebViewBuilder::new(&self.servo, renderer.clone())
            .hidpi_scale_factor(Scale::new(scale as f32))
            .clipboard_delegate(web_view_handler.clone())
            .delegate(web_view_handler)
            .url(url)
            .build();

        Ok(ServoEngine {
            web_view,
            scale,
            size,
            id,
            cookie_whitelist: self.cookie_whitelist.clone(),
            text_input_change: TextInputChange::Disabled,
            renderer: renderer.clone(),
            servo: self.servo.clone(),
            queue: self.queue.clone(),
            dirty: true,
            text_input_cursor: Default::default(),
            text_input_text: Default::default(),
            pending_resize: Default::default(),
            text_input_id: Default::default(),
            animating: Default::default(),
            menu: Default::default(),
        })
    }

    /// Handle window destruction.
    pub fn on_window_close(&mut self, window_id: WindowId) {
        self.rendering_contexts.retain(|id, _| *id != window_id);
    }
}

/// Servo browser engine.
pub struct ServoEngine {
    renderer: Rc<dyn RenderingContext>,
    web_view: WebView,
    servo: Rc<Servo>,

    menu: Option<(OptionMenuId, EmbedderControlId, ServoContextMenu, ContextMenu)>,
    cookie_whitelist: CookieWhitelist,

    pending_resize: Option<Size>,
    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    id: EngineId,

    text_input_id: Option<EmbedderControlId>,
    text_input_change: TextInputChange,
    text_input_text: String,
    text_input_cursor: u32,

    animating: bool,
    dirty: bool,
}

impl Engine for ServoEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn engine_type(&self) -> EngineType {
        EngineType::Servo
    }

    fn dirty(&mut self) -> bool {
        self.dirty || self.animating
    }

    fn set_visible(&mut self, visible: bool) {
        if visible {
            self.web_view.show();
        } else {
            self.web_view.hide();
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self) -> bool {
        self.dirty = false;

        // Update Wayland surface dimensions.
        self.size = self.renderer.size().into();

        // Process all pending Servo events.
        self.servo.spin_event_loop();

        // If a size change is pending, resize the webview and trigger a redraw.
        //
        // While this should be done before rendering to avoid drawing twice, Servo
        // frequently calls `make_current` causing the surface size to latch to
        // arbitrary sizes. By resizing right after rendering, we are able to  ensure
        // the requested size will be accurate for the next redraw.
        if let Some(pending_resize) = self.pending_resize.take() {
            let size = pending_resize * self.scale;
            self.web_view.resize(size.into());
            self.dirty = true;
        }

        self.web_view.paint();
        self.renderer.present();

        true
    }

    fn buffer_size(&self) -> Size {
        self.size
    }

    fn set_size(&mut self, size: Size) {
        self.pending_resize = Some(size);

        // Mark engine as ready for redraw.
        // Unstall is called automatically in window.rs on resize.
        self.dirty = true;
    }

    fn set_scale(&mut self, scale: f64) {
        if scale == self.scale {
            return;
        }
        self.scale = scale;

        // Ensure engine is resized to its new dimensions.
        if self.pending_resize.is_none() {
            self.pending_resize = Some(self.size);
        }

        // Update the engine's render scale.
        self.web_view.set_hidpi_scale_factor(Scale::new(scale as f32));

        // Mark engine as ready for redraw.
        // Unstall is called automatically in window.rs on scale change.
        self.dirty = true;
    }

    fn press_key(&mut self, _time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        let (key, code) = match (keysym_to_key(keysym), Code::from_xkb_kecode(raw)) {
            (Some(key), Some(code)) => (key, code),
            _ => return,
        };
        let location = Location::from_xkb_keysym(keysym.raw());
        let modifiers = servo_modifiers(modifiers);

        let event = KeyboardEvent {
            modifiers,
            location,
            code,
            key,
            state: KeyState::Down,
            is_composing: Default::default(),
            repeat: Default::default(),
        };
        self.web_view.notify_input_event(InputEvent::Keyboard(servo::KeyboardEvent::new(event)));
    }

    fn release_key(&mut self, _time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        let (key, code) = match (keysym_to_key(keysym), Code::from_xkb_kecode(raw)) {
            (Some(key), Some(code)) => (key, code),
            _ => return,
        };
        let location = Location::from_xkb_keysym(keysym.raw());
        let modifiers = servo_modifiers(modifiers);

        let event = KeyboardEvent {
            modifiers,
            location,
            code,
            key,
            state: KeyState::Up,
            is_composing: Default::default(),
            repeat: Default::default(),
        };
        self.web_view.notify_input_event(InputEvent::Keyboard(servo::KeyboardEvent::new(event)));
    }

    fn pointer_axis(
        &mut self,
        _time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        _modifiers: Modifiers,
    ) {
        let position = position * self.scale;
        let delta = WheelDelta {
            x: horizontal.absolute,
            y: -vertical.absolute,
            z: 0.,
            mode: WheelMode::DeltaPixel,
        };
        let event = WheelEvent::new(delta, position.into());
        self.web_view.notify_input_event(InputEvent::Wheel(event));
    }

    fn pointer_button(
        &mut self,
        _time: u32,
        position: Position<f64>,
        button: u32,
        down: bool,
        _modifiers: Modifiers,
    ) {
        self.web_view.focus();

        let position = position * self.scale;
        let button = match button {
            272 => MouseButton::Left,
            273 => MouseButton::Right,
            274 => MouseButton::Middle,
            // Auxiliary mouse buttons are not supported by Servo.
            _ => return,
        };
        let action = if down { MouseButtonAction::Down } else { MouseButtonAction::Up };
        let event = MouseButtonEvent::new(action, button, position.into());

        self.web_view.notify_input_event(InputEvent::MouseButton(event));
    }

    fn pointer_motion(&mut self, _time: u32, position: Position<f64>, _modifiers: Modifiers) {
        let position = position * self.scale;
        let event = MouseMoveEvent::new(position.into());
        self.web_view.notify_input_event(InputEvent::MouseMove(event));
    }

    fn touch_down(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        self.web_view.focus();

        let position = position * self.scale;
        let event = TouchEvent::new(TouchEventType::Down, TouchId(id), position.into());
        self.web_view.notify_input_event(InputEvent::Touch(event));
    }

    fn touch_up(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        let position = position * self.scale;
        let event = TouchEvent::new(TouchEventType::Up, TouchId(id), position.into());
        self.web_view.notify_input_event(InputEvent::Touch(event));
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        let position = position * self.scale;
        let event = TouchEvent::new(TouchEventType::Move, TouchId(id), position.into());
        self.web_view.notify_input_event(InputEvent::Touch(event));
    }

    fn reload(&mut self) {
        self.web_view.reload();
    }

    fn load_uri(&mut self, url: &str) {
        match Url::parse(url) {
            Ok(url) => self.web_view.load(url),
            Err(err) => error!("Invalid URL {url:?}: {err}"),
        }
    }

    fn load_prev(&mut self) {
        self.web_view.go_back(1);
    }

    fn has_prev(&self) -> bool {
        self.web_view.can_go_back()
    }

    fn uri(&self) -> Cow<'_, str> {
        self.web_view.url().map_or(Cow::Borrowed(""), |url| url.to_string().into())
    }

    fn title(&self) -> Cow<'_, str> {
        self.web_view.page_title().map_or(Cow::Borrowed(""), |title| title.into())
    }

    fn text_input_state(&mut self) -> TextInputChange {
        mem::replace(&mut self.text_input_change, TextInputChange::Unchanged)
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        let start = self.text_input_cursor.saturating_sub(before_length) as usize;
        let end = (self.text_input_cursor + after_length) as usize;

        // Remove deleted text section.
        let mut text = self.text_input_text.clone();
        text.drain(start..end.min(text.len()));

        let event =
            ImeEvent::Composition(CompositionEvent { state: CompositionState::Update, data: text });
        self.web_view.notify_input_event(InputEvent::Ime(event));
    }

    fn commit_string(&mut self, text: String) {
        let event =
            ImeEvent::Composition(CompositionEvent { state: CompositionState::End, data: text });
        self.web_view.notify_input_event(InputEvent::Ime(event));
    }

    fn set_preedit_string(&mut self, text: String, _cursor_begin: i32, _cursor_end: i32) {
        let event =
            ImeEvent::Composition(CompositionEvent { state: CompositionState::Update, data: text });
        self.web_view.notify_input_event(InputEvent::Ime(event));
    }

    fn clear_focus(&mut self) {
        self.web_view.blur();
    }

    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize) {
        if self.menu.as_ref().is_none_or(|(id, ..)| *id != menu_id) {
            return;
        }
        let (id, _, _, menu) = self.menu.take().unwrap();

        // Activate selected option.
        menu.activate_item(self.queue.clone(), self.id, index as u32);

        // Close Kumo's option menu UI.
        self.queue.close_menu(id);
    }

    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>) {
        if menu_id.is_some_and(|menu_id| Some(&menu_id) != self.menu.as_ref().map(|(id, ..)| id))
            || self.menu.is_none()
        {
            return;
        }
        let (id, _, servo_menu, _) = self.menu.take().unwrap();

        // Notify menu about being closed from our end.
        servo_menu.dismiss();

        // Close Kumo's option menu UI.
        self.queue.close_menu(id);
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        if !fullscreen {
            self.web_view.exit_fullscreen();
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn favicon(&self) -> Option<Favicon> {
        let favicon = self.web_view.favicon()?;

        let format = match favicon.format {
            PixelFormat::K8 | PixelFormat::KA8 => {
                debug!("Unsupported Servo favicon format");
                return None;
            },
            PixelFormat::RGB8 => gl::RGB,
            PixelFormat::RGBA8 => gl::RGBA,
            PixelFormat::BGRA8 => gl::BGRA_EXT,
        };

        (favicon.width > 0 && favicon.height > 0).then_some(Favicon {
            format,
            bytes: glib::Bytes::from(favicon.data()),
            id: FaviconId::new_servo(),
            width: favicon.width as usize,
            height: favicon.height as usize,
        })
    }

    fn set_zoom_level(&mut self, zoom_level: f64) {
        // Zoom towards the center of the webview.
        let x = self.size.width as f32 * 0.5;
        let y = self.size.height as f32 * 0.5;
        let center = Position::new(x, y);

        // Calculate factor based on current zoom level.
        let delta = zoom_level / self.web_view.pinch_zoom() as f64;

        // Update zoom level.
        self.web_view.adjust_pinch_zoom(delta as f32, center.into());

        // Dispatch events to trigger a redraw.
        self.servo.spin_event_loop();
    }

    fn zoom_level(&self) -> f64 {
        self.web_view.pinch_zoom() as f64
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Servo event loop waker.
#[derive(Clone)]
struct Waker {
    queue: MtQueueHandle<State>,
}

impl Waker {
    fn new(queue: MtQueueHandle<State>) -> Self {
        Self { queue }
    }
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        self.queue.clone().wakeup();
    }
}

/// Servo web view event handler.
#[derive(Clone)]
struct WebViewHandler {
    queue: MtQueueHandle<State>,
    engine_id: EngineId,
}

impl WebViewHandler {
    fn new(queue: MtQueueHandle<State>, engine_id: EngineId) -> Self {
        Self { engine_id, queue }
    }
}

impl WebViewDelegate for WebViewHandler {
    fn notify_new_frame_ready(&self, _web_view: WebView) {
        self.queue.clone().frame(self.engine_id);
    }

    fn notify_url_changed(&self, _webview: WebView, _url: Url) {
        self.queue.clone().update_engine_uri(self.engine_id, true);
    }

    fn notify_page_title_changed(&self, _webview: WebView, _title: Option<String>) {
        self.queue.clone().update_engine_title(self.engine_id);
    }

    fn notify_animating_changed(&self, _webview: WebView, animating: bool) {
        self.queue.clone().set_animating(self.engine_id, animating);
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        match status {
            LoadStatus::Started => self.queue.clone().set_load_progress(self.engine_id, 0.),
            LoadStatus::HeadParsed => self.queue.clone().set_load_progress(self.engine_id, 0.5),
            LoadStatus::Complete => self.queue.clone().set_load_progress(self.engine_id, 1.),
        }
    }

    fn notify_favicon_changed(&self, _webview: WebView) {
        self.queue.clone().update_favicon(self.engine_id);
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, backtrace: Option<String>) {
        error!("Servo WebView crashed: {reason}");

        // Print full backtrace when `RUST_BACKTRACE=1` is set.
        if let Some(backtrace) = backtrace
            && env::var("RUST_BACKTRACE").as_deref() == Ok("1")
        {
            error!("Servo backtrace:\n{backtrace}");
        }
    }

    fn notify_fullscreen_state_changed(&self, _webview: WebView, fullscreen: bool) {
        self.queue.clone().set_fullscreen(self.engine_id, fullscreen);
    }

    fn show_embedder_control(&self, _webview: WebView, embedder_control: EmbedderControl) {
        let embedder_id = embedder_control.id();

        match embedder_control {
            EmbedderControl::InputMethod(ime) => {
                // Calculate input field dimensions.
                let position = ime.position();
                let width = position.max.x - position.min.x;
                let height = position.max.y - position.min.y;
                let cursor_rect = (position.min.x, position.min.y, width, height);

                // Convert Servo to Wayland input purpose.
                let purpose = match ime.input_method_type() {
                    InputMethodType::Date => ContentPurpose::Date,
                    InputMethodType::DatetimeLocal => ContentPurpose::Datetime,
                    InputMethodType::Email => ContentPurpose::Email,
                    InputMethodType::Number => ContentPurpose::Number,
                    InputMethodType::Password => ContentPurpose::Password,
                    InputMethodType::Tel => ContentPurpose::Phone,
                    InputMethodType::Time => ContentPurpose::Time,
                    InputMethodType::Url => ContentPurpose::Url,
                    InputMethodType::Color
                    | InputMethodType::Month
                    | InputMethodType::Search
                    | InputMethodType::Text
                    | InputMethodType::Week => ContentPurpose::Normal,
                };

                // Clamp IME text to protocol limits.
                let full_text = ime.text();
                let cursor_index = ime.insertion_point().unwrap_or(0);
                let (surrounding_text, cursor_index, _) = window::clamp_surrounding_text(
                    &full_text,
                    cursor_index as usize,
                    cursor_index as usize,
                    window::MAX_SURROUNDING_BYTES,
                );

                let state = TextInputState {
                    surrounding_text,
                    cursor_index,
                    cursor_rect,
                    purpose,
                    ..Default::default()
                };

                self.queue.clone().set_text_input_state(
                    self.engine_id,
                    embedder_id,
                    state,
                    full_text,
                );
            },
            EmbedderControl::ContextMenu(menu) => {
                self.queue.clone().open_menu(self.engine_id, menu);
            },
            EmbedderControl::ColorPicker(_)
            | EmbedderControl::FilePicker(_)
            | EmbedderControl::SelectElement(_)
            | EmbedderControl::SimpleDialog(_) => {
                debug!("Servo requested unsupported embedder controls")
            },
        }
    }

    fn hide_embedder_control(&self, _webview: WebView, embedder_id: EmbedderControlId) {
        self.queue.clone().close_embedder_control(self.engine_id, embedder_id);
    }
}

impl ClipboardDelegate for WebViewHandler {
    fn get_text(&self, _webview: WebView, request: StringRequest) {
        self.queue.clone().request_paste(PasteTarget::Servo(self.engine_id, request));
    }

    fn set_text(&self, _webview: WebView, text: String) {
        self.queue.clone().set_clipboard(text);
    }

    fn clear(&self, _webview: WebView) {
        self.queue.clone().set_clipboard(String::new());
    }
}

/// Get and downcast a Servo engine from a window.
fn servo_engine_by_id(window: &mut Window, engine_id: EngineId) -> Option<&mut ServoEngine> {
    let engine = window.tab_mut(engine_id)?;
    engine.as_any().downcast_mut::<ServoEngine>()
}

/// Convert Wayland to Servo modifiers.
fn servo_modifiers(modifiers: Modifiers) -> ServoModifiers {
    let mut servo_modifiers = ServoModifiers::empty();
    servo_modifiers.set(ServoModifiers::CAPS_LOCK, modifiers.caps_lock);
    servo_modifiers.set(ServoModifiers::NUM_LOCK, modifiers.num_lock);
    servo_modifiers.set(ServoModifiers::CONTROL, modifiers.ctrl);
    servo_modifiers.set(ServoModifiers::SHIFT, modifiers.shift);
    servo_modifiers.set(ServoModifiers::META, modifiers.logo);
    servo_modifiers.set(ServoModifiers::ALT, modifiers.alt);
    servo_modifiers
}

/// Convert a Wayland [`Keysym`] to a Servo [`Key`].
fn keysym_to_key(keysym: Keysym) -> Option<Key> {
    let raw_keysym = keysym.raw();
    if raw_keysym == 0 {
        return None;
    }

    Key::from_xkb_keysym(raw_keysym).or_else(|| {
        let mut buffer = [0; 8];
        let key_utf8 = raw_keysym_to_utf8(&mut buffer, raw_keysym)?;
        Some(Key::Character(key_utf8.into()))
    })
}

/// Try to convert a raw keysym to a UTF8 string.
fn raw_keysym_to_utf8(buffer: &mut [u8], keysym: u32) -> Option<&str> {
    static XKBH: LazyLock<&'static XkbCommon> = LazyLock::new(xkbcommon_handle);

    let bytes_written =
        unsafe { (XKBH.xkb_keysym_to_utf8)(keysym, buffer.as_mut_ptr().cast(), buffer.len()) };

    match bytes_written {
        ..0 => {
            error!("Insufficient space to convert keysym {keysym}");
            None
        },
        0 | 1 => None,
        bytes_written => str::from_utf8(&buffer[..bytes_written as usize - 1])
            .inspect_err(|err| error!("Failed to parse bytes for keysym {keysym} as UTF-8: {err}"))
            .ok(),
    }
}

#[cfg(test)]
mod tests {
    use keyboard_types::NamedKey;
    use xkbcommon_dl::keysyms;

    use super::*;

    #[test]
    fn raw_to_utf8() {
        let mut buffer = [0; 8];

        assert_eq!(raw_keysym_to_utf8(&mut buffer, keysyms::Aacute), Some("Á"));
        assert_eq!(raw_keysym_to_utf8(&mut buffer, keysyms::T), Some("T"));

        assert_eq!(raw_keysym_to_utf8(&mut buffer, keysyms::NoSymbol), None);
        assert_eq!(raw_keysym_to_utf8(&mut buffer, 0), None);
    }

    #[test]
    fn sym_to_key() {
        assert_eq!(keysym_to_key(Keysym::Escape), Some(Key::Named(NamedKey::Escape)));
        assert_eq!(keysym_to_key(Keysym::T), Some(Key::Character("T".into())));

        assert_eq!(keysym_to_key(Keysym::NoSymbol), None);
    }
}
