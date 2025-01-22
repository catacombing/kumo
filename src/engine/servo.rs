//! Servo browser engine.

use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::env;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Once;

use funq::MtQueueHandle;
use glutin::display::Display;
use raw_window_handle::{DisplayHandle, RawDisplayHandle, WaylandDisplayHandle};
use rustls::crypto::ring;
use servo::base::id::WebViewId;
use servo::compositing::windowing::{
    AnimationState, EmbedderCoordinates, EmbedderEvent, EmbedderMethods, MouseWindowEvent,
    WindowMethods,
};
use servo::compositing::CompositeTarget;
use servo::embedder_traits::{EmbedderMsg, EventLoopWaker};
use servo::euclid::{Box2D, Scale, Size2D};
use servo::script_traits::{
    MouseButton, TouchEventType, TouchId, TraversalDirection, WheelDelta, WheelMode,
};
use servo::url::ServoUrl;
use servo::webrender_traits::RenderingContext;
use servo::Servo;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Connection as WaylandConnection;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use surfman::platform::generic::multi::context::NativeContext as MultiNativeContext;
use surfman::platform::unix::wayland::context::NativeContext as WaylandNativeContext;
use surfman::{Connection, Context, NativeContext};

use crate::engine::{Engine, EngineHandler, EngineId};
use crate::ui::overlay::option_menu::OptionMenuId;
use crate::ui::renderer::Renderer;
use crate::window::{TextInputChange, WindowHandler};
use crate::{gl, Position, Size, State};

/// Default engine URI.
const DEFAULT_URI: &str = "about:blank";

#[funq::callbacks(State)]
trait ServoHandler {
    /// Handle wakeup from Servo event loop.
    fn servo_wakeup(&mut self, engine_id: EngineId);
}

impl ServoHandler for State {
    fn servo_wakeup(&mut self, engine_id: EngineId) {
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

        servo_engine.process_events();

        // Wakeup renderer if Servo requires a redraw.
        if servo_engine.dirty && window.active_tab().is_some_and(|tab| tab.id() == engine_id) {
            window.unstall();
        }
    }
}

/// Servo browser engine.
pub struct ServoEngine {
    servo: Option<Servo<ServoViewport>>,

    queue: MtQueueHandle<State>,
    servo_id: Option<WebViewId>,
    id: EngineId,

    rendering_context: RenderingContext,
    renderer: Option<ServoRenderer>,
    display: Option<Display>,

    title: String,
    uri: String,

    dirty: bool,
}

impl ServoEngine {
    pub fn new(
        queue: MtQueueHandle<State>,
        connection: WaylandConnection,
        display: Display,
        id: EngineId,
    ) -> Self {
        static SERVO_INIT: Once = Once::new();
        SERVO_INIT.call_once(|| {
            // Initialize TLS crypto provider at first engine spawn.
            ring::default_provider().install_default().unwrap();

            // Fix Servo context adoption by forcing surfman GLES support.
            env::set_var("SURFMAN_FORCE_GLES", "1");
        });

        // Create Servo Wayland connection from existing display.
        let display_ptr = NonNull::new(connection.backend().display_ptr().cast()).unwrap();
        let wayland_display = WaylandDisplayHandle::new(display_ptr);
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let display_handle = unsafe { DisplayHandle::borrow_raw(raw_display) };
        let connection = Connection::from_display_handle(display_handle).unwrap();

        // Create Servo rendering context.
        let adapter = connection.create_adapter().unwrap();
        let rendering_context =
            RenderingContext::create(&connection, &adapter, Some(Size2D::new(1, 1))).unwrap();

        // Create engine viewport interface.
        let embedder = Box::new(Embedder::new(queue.clone(), id));
        let viewport = Rc::new(ServoViewport::new());

        let mut servo = Servo::new(
            Default::default(),
            Default::default(),
            rendering_context.clone(),
            embedder,
            viewport,
            None,
            CompositeTarget::Window,
        );

        // Ensure Servo web view is created immediately.
        let url = ServoUrl::parse(DEFAULT_URI).unwrap();
        servo.handle_events([EmbedderEvent::NewWebView(url, WebViewId::new())]);

        Self {
            rendering_context,
            queue,
            id,
            uri: DEFAULT_URI.into(),
            display: Some(display),
            servo: Some(servo),
            renderer: Default::default(),
            servo_id: Default::default(),
            dirty: Default::default(),
            title: Default::default(),
        }
    }

    /// Handle new Servo events.
    fn process_events(&mut self) {
        // Get all pending events from Servo.
        let events: Vec<_> = self.servo.as_mut().unwrap().get_events().map(|(_, m)| m).collect();

        for msg in events {
            println!("Servo in : {msg:?}");

            match msg {
                EmbedderMsg::ReadyToPresent(..) => self.dirty = true,
                // Select our new web view as primary one for rendering.
                EmbedderMsg::WebViewOpened(id) => {
                    let servo = self.servo.as_mut().unwrap();
                    let viewport = servo.window().get_coordinates().viewport.to_f32();

                    // NOTE: Servo ignores the initial resize if it matches the window size,
                    // so we need this fake resize event to work around that.
                    let mut dummy_size = viewport.size();
                    dummy_size.width += 1.;
                    servo.handle_events([EmbedderEvent::MoveResizeWebView(
                        id,
                        Box2D::from_size(dummy_size),
                    )]);

                    self.servo_id = Some(id);
                    servo.handle_events([EmbedderEvent::FocusWebView(id)]);
                    servo.handle_events([EmbedderEvent::RaiseWebViewToTop(id, true)]);
                    servo.handle_events([EmbedderEvent::MoveResizeWebView(id, viewport)]);

                    // Immediately navigate if there's a pending URI change.
                    if self.uri != DEFAULT_URI {
                        self.update_engine_uri();
                    }
                },
                // TODO: This doesn't work, it might only show history (not current) URIs.
                //
                // Use history changes to update current URI.
                EmbedderMsg::HistoryChanged(history, _) => {
                    if let Some(uri) = history.last() {
                        self.queue.set_engine_uri(self.id, uri.to_string());
                    }
                },
                EmbedderMsg::ChangePageTitle(title) => {
                    self.title = title.unwrap_or_default();
                    self.queue.set_engine_title(self.id, self.title.clone());
                },
                EmbedderMsg::SetFullscreenState(fullscreen) => {
                    self.queue.set_fullscreen(self.id, fullscreen);
                },
                EmbedderMsg::ClearClipboardContents => self.queue.set_clipboard("".into()),
                EmbedderMsg::SetClipboardContents(text) => self.queue.set_clipboard(text),
                EmbedderMsg::GetClipboardContents(_) => {
                    // TODO: UNIMPLEMENTED
                },
                EmbedderMsg::ShowIME(..) => {
                    // TODO: UNIMPLEMENTED
                },
                EmbedderMsg::HideIME => {
                    // TODO: UNIMPLEMENTED
                },
                EmbedderMsg::ShowContextMenu(..) => {
                    // TODO: UNIMPLEMENTED
                },
                // Events irrelevant to Kumo.
                EmbedderMsg::AllowNavigationRequest(..)
                | EmbedderMsg::AllowOpeningWebView(..)
                | EmbedderMsg::AllowUnload(..)
                | EmbedderMsg::EventDelivered(..)
                | EmbedderMsg::GetSelectedBluetoothDevice(..)
                | EmbedderMsg::HeadParsed
                | EmbedderMsg::Keyboard(..)
                | EmbedderMsg::LoadComplete
                | EmbedderMsg::LoadStart
                | EmbedderMsg::MediaSessionEvent(..)
                | EmbedderMsg::MoveTo(..)
                | EmbedderMsg::NewFavicon(..)
                | EmbedderMsg::OnDevtoolsStarted(..)
                | EmbedderMsg::Panic(..)
                | EmbedderMsg::PlayGamepadHapticEffect(..)
                | EmbedderMsg::Prompt(..)
                | EmbedderMsg::PromptPermission(..)
                | EmbedderMsg::ReportProfile(..)
                | EmbedderMsg::ResizeTo(..)
                | EmbedderMsg::SelectFiles(..)
                | EmbedderMsg::SetCursor(..)
                | EmbedderMsg::Shutdown
                | EmbedderMsg::Status(..)
                | EmbedderMsg::StopGamepadHapticEffect(..)
                | EmbedderMsg::WebResourceRequested(..)
                | EmbedderMsg::WebViewBlurred
                | EmbedderMsg::WebViewClosed(..)
                | EmbedderMsg::WebViewFocused(..) => (),
            }
        }
    }

    /// Load current URL in the active Servo web view.
    fn update_engine_uri(&mut self) {
        // Debounce events until web view was created.
        let servo_id = match self.servo_id {
            Some(servo_id) => servo_id,
            None => return,
        };

        let url = ServoUrl::parse(&self.uri).unwrap();
        let event = EmbedderEvent::LoadUrl(servo_id, url);
        self.servo.as_mut().unwrap().handle_events([event]);
    }
}

impl Engine for ServoEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn dirty(&mut self) -> bool {
        self.process_events();

        // TODO: Servo does not reliably wake us up, so to be usable at all we have to
        // redraw constantly.
        //
        // self.dirty || self.servo.as_ref().unwrap().window().animating.get()
        true
    }

    fn attach_buffer(&mut self, surface: &WlSurface) -> bool {
        self.dirty = false;

        // Initialize renderer on first draw.
        let servo = self.servo.as_mut().unwrap();
        let window = servo.window();
        let size = window.size.get() * window.scale.get();
        let servo_renderer = self.renderer.get_or_insert_with(|| {
            let display = self.display.take().unwrap();
            ServoRenderer::new(display, &self.rendering_context, surface, size)
        });

        // Resize the renderer to avoid Servo latching the previous size when drawing.
        servo_renderer.renderer.resize(size);

        // Debounce resize events until redraw, since this latches the EGL surface.
        if window.resized.take() {
            servo.handle_events([EmbedderEvent::WindowResize]);
        }

        // TODO: This clearly is broken.
        //  => Servo painting asynchronously might be a problem.
        //
        // Request redraw from Servo.
        servo.present();
        // servo.repaint_synchronously();

        // Copy Servo surface content to the engine's rendering surface.
        servo_renderer.renderer.draw(size, |_| unsafe {
            self.rendering_context.with_front_buffer(|device, surface| {
                let texture =
                    device.create_surface_texture(&mut servo_renderer.context, surface).unwrap();
                let texture_id = device.surface_texture_object(&texture);

                // Blit Servo texture to Wayland surface.
                gl::BindFramebuffer(servo::gl::READ_FRAMEBUFFER, servo_renderer.read_fb);
                gl::FramebufferTexture2D(
                    gl::READ_FRAMEBUFFER,
                    gl::COLOR_ATTACHMENT0,
                    gl::TEXTURE_2D,
                    texture_id,
                    0,
                );
                gl::BlitFramebuffer(
                    0,
                    0,
                    size.width as i32,
                    size.height as i32,
                    0,
                    0,
                    size.width as i32,
                    size.height as i32,
                    gl::COLOR_BUFFER_BIT,
                    gl::LINEAR,
                );
                gl::BindFramebuffer(gl::READ_FRAMEBUFFER, 0);

                device.destroy_surface_texture(&mut servo_renderer.context, texture).unwrap()
            });
        });

        true
    }

    fn buffer_size(&self) -> Size {
        // Servo renders on-demand, so we just return the target size here.
        let window = self.servo.as_ref().unwrap().window();
        window.size.get() * window.scale.get()
    }

    fn set_size(&mut self, size: Size) {
        let servo = self.servo.as_mut().unwrap();
        servo.window().resized.replace(true);
        servo.window().size.replace(size);

        // Mark engine as ready for redraw.
        // Unstall is called automatically in window.rs on resize.
        self.dirty = true;
    }

    fn set_scale(&mut self, scale: f64) {
        let servo = self.servo.as_mut().unwrap();
        servo.window().resized.replace(true);
        servo.window().scale.replace(scale);

        // Mark engine as ready for redraw.
        // Unstall is called automatically in window.rs on scale change.
        self.dirty = true;
    }

    fn press_key(&mut self, _time: u32, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {
        // TODO: UNIMPLEMENTED
    }

    fn release_key(&mut self, _time: u32, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {
        // TODO: UNIMPLEMENTED
    }

    fn pointer_axis(
        &mut self,
        _time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        _modifiers: Modifiers,
    ) {
        let delta = WheelDelta {
            x: horizontal.absolute,
            y: vertical.absolute,
            z: 0.,
            mode: WheelMode::DeltaPixel,
        };
        self.servo.as_mut().unwrap().handle_events([EmbedderEvent::Wheel(delta, position.into())]);
    }

    fn pointer_button(
        &mut self,
        _time: u32,
        position: Position<f64>,
        button: u32,
        down: bool,
        _modifiers: Modifiers,
    ) {
        let button = match button {
            272 => MouseButton::Left,
            273 => MouseButton::Right,
            274 => MouseButton::Middle,
            // Auxiliary mouse buttons are not supported by Servo.
            _ => return,
        };
        let event = if down {
            MouseWindowEvent::MouseDown(button, position.into())
        } else {
            MouseWindowEvent::MouseUp(button, position.into())
        };

        self.servo.as_mut().unwrap().handle_events([EmbedderEvent::MouseWindowEventClass(event)]);
    }

    fn pointer_motion(&mut self, _time: u32, position: Position<f64>, _modifiers: Modifiers) {
        let event = EmbedderEvent::MouseWindowMoveEventClass(position.into());
        self.servo.as_mut().unwrap().handle_events([event]);
    }

    fn pointer_enter(&mut self, _position: Position<f64>, _modifiers: Modifiers) {
        // TODO: UNIMPLEMENTED
    }

    fn pointer_leave(&mut self, _position: Position<f64>, _modifiers: Modifiers) {
        // TODO: UNIMPLEMENTED
    }

    fn touch_up(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        let servo = self.servo.as_mut().unwrap();
        let position = position * servo.window().scale.get();

        let event = EmbedderEvent::Touch(TouchEventType::Up, TouchId(id), position.into());
        servo.handle_events([event]);
    }

    fn touch_down(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        let servo = self.servo.as_mut().unwrap();
        let position = position * servo.window().scale.get();

        let event = EmbedderEvent::Touch(TouchEventType::Down, TouchId(id), position.into());
        servo.handle_events([event]);
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        let servo = self.servo.as_mut().unwrap();
        let position = position * servo.window().scale.get();

        let event = EmbedderEvent::Touch(TouchEventType::Move, TouchId(id), position.into());
        servo.handle_events([event]);
    }

    fn load_uri(&mut self, uri: &str) {
        self.uri = uri.into();
        self.update_engine_uri();
    }

    fn load_prev(&mut self) {
        // Ignore events until web view was created.
        let servo_id = match self.servo_id {
            Some(servo_id) => servo_id,
            None => return,
        };

        let event = EmbedderEvent::Navigation(servo_id, TraversalDirection::Back(1));
        self.servo.as_mut().unwrap().handle_events([event]);
    }

    fn uri(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.uri)
    }

    fn title(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }

    fn text_input_state(&self) -> TextInputChange {
        // TODO: UNIMPLEMENTED
        TextInputChange::Disabled
    }

    fn delete_surrounding_text(&mut self, _before_length: u32, _after_length: u32) {
        // TODO: UNIMPLEMENTED
    }

    fn commit_string(&mut self, _text: String) {
        // TODO: UNIMPLEMENTED
    }

    fn set_preedit_string(&mut self, _text: String, _cursor_begin: i32, _cursor_end: i32) {
        // TODO: UNIMPLEMENTED
    }

    fn paste(&mut self, _text: String) {
        // TODO: UNIMPLEMENTED
    }

    fn clear_focus(&mut self) {
        // TODO: UNIMPLEMENTED
    }

    fn submit_option_menu(&mut self, _menu_id: OptionMenuId, _index: usize) {
        // TODO: UNIMPLEMENTED
    }

    fn close_option_menu(&mut self, _menu_id: Option<OptionMenuId>) {
        // TODO: UNIMPLEMENTED
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        // Ignore events until web view was created.
        let servo_id = match self.servo_id {
            Some(servo_id) => servo_id,
            None => return,
        };

        if fullscreen {
            // TODO: Unsupported by Servo?
        } else {
            self.servo.as_mut().unwrap().handle_events([EmbedderEvent::ExitFullScreen(servo_id)]);
        }
    }

    fn session(&self) -> Vec<u8> {
        // TODO: UNIMPLEMENTED
        Vec::new()
    }

    fn restore_session(&self, _session: Vec<u8>) {
        // TODO: UNIMPLEMENTED
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for ServoEngine {
    fn drop(&mut self) {
        if let Some(servo) = self.servo.take() {
            servo.deinit();
        }
    }
}

/// Servo rendering context.
struct ServoRenderer {
    renderer: Renderer,
    context: Context,
    read_fb: u32,
}

impl ServoRenderer {
    fn new(
        display: Display,
        rendering_context: &RenderingContext,
        surface: &WlSurface,
        size: Size,
    ) -> Self {
        let mut renderer = Renderer::new(display, surface.clone());
        let mut context = None;
        let mut read_fb = 0;
        renderer.draw(size, |_| unsafe {
            // Initialize the FB used to blit Servo textures.
            gl::GenFramebuffers(1, &mut read_fb);

            // Wrap rendering context for Servo use.
            let wayland_context = WaylandNativeContext::current().unwrap();
            let nativer_context = MultiNativeContext::Default(wayland_context);
            let native_context = NativeContext::Default(nativer_context);
            let device = rendering_context.device();
            context = Some(device.create_context_from_native_context(native_context).unwrap());
        });

        Self { renderer, read_fb, context: context.unwrap() }
    }
}

impl Drop for ServoRenderer {
    fn drop(&mut self) {
        self.renderer.draw(Size::new(1, 1), |_| unsafe {
            gl::DeleteFramebuffers(1, &self.read_fb);
        });
    }
}

struct Embedder {
    queue: MtQueueHandle<State>,
    engine_id: EngineId,
}

impl Embedder {
    fn new(queue: MtQueueHandle<State>, engine_id: EngineId) -> Self {
        Self { queue, engine_id }
    }
}

impl EmbedderMethods for Embedder {
    fn create_event_loop_waker(&mut self) -> Box<dyn EventLoopWaker> {
        Box::new(Waker { queue: self.queue.clone(), engine_id: self.engine_id })
    }
}

#[derive(Clone)]
struct Waker {
    queue: MtQueueHandle<State>,
    engine_id: EngineId,
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        self.queue.clone().servo_wakeup(self.engine_id);
    }
}

struct ServoViewport {
    animating: Cell<bool>,
    resized: Cell<bool>,
    scale: Cell<f64>,
    size: Cell<Size>,
}

impl ServoViewport {
    fn new() -> Self {
        Self {
            scale: Cell::new(1.),
            animating: Default::default(),
            resized: Default::default(),
            size: Default::default(),
        }
    }
}

impl WindowMethods for ServoViewport {
    fn get_coordinates(&self) -> EmbedderCoordinates {
        let scale = self.scale.get();
        let size = self.size.get() * scale;
        EmbedderCoordinates {
            window_rect: Box2D::from_size(size.into()),
            viewport: Box2D::from_size(size.into()),
            hidpi_factor: Scale::new(scale as f32),
            available_screen_size: size.into(),
            framebuffer: size.into(),
            screen_size: size.into(),
        }
    }

    fn set_animation_state(&self, state: AnimationState) {
        self.animating.replace(state == AnimationState::Animating);
    }
}
