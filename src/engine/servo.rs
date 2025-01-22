//! Servo browser engine.

// TODO: Temporary to make sure errors show up.
#![allow(unused)]

use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Once;

use funq::MtQueueHandle;
use glutin::context::AsRawContext;
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
use servo::keyboard_types::{
    Code, Key, KeyState, KeyboardEvent, Location, Modifiers as ServoModifiers,
};
use servo::script_traits::{
    MouseButton, TouchEventType, TouchId, TraversalDirection, WheelDelta, WheelMode,
};
use servo::url::ServoUrl;
use servo::webrender_traits::RenderingContext;
use servo::{Servo, TopLevelBrowsingContextId};
use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Connection as WaylandConnection;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use surfman::platform::generic::multi::context::NativeContext as MultiNativeContext;
use surfman::platform::unix::wayland::context::NativeContext as WaylandNativeContext;
use surfman::{Connection, Context, NativeContext, SurfaceType};

use crate::engine::{Engine, EngineHandler, EngineId};
use crate::ui::overlay::option_menu::OptionMenuId;
use crate::ui::renderer::Renderer;
use crate::window::TextInputChange;
use crate::{gl, Position, Size, State};

#[funq::callbacks(State)]
trait ServoHandler {
    /// Handle new Servo events.
    fn handle_servo_events(&mut self, engine_id: EngineId);
}

impl ServoHandler for State {
    fn handle_servo_events(&mut self, engine_id: EngineId) {
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

        // TODO: Just doing window.unstall() if engine is active is probably
        // sufficient here?
        //  => Rename method if so
    }
}

/// Servo browser engine.
pub struct ServoEngine {
    servo: Option<Servo<ServoViewport>>,
    events: Vec<EmbedderEvent>,

    queue: MtQueueHandle<State>,
    servo_id: WebViewId,
    id: EngineId,

    rendering_context: RenderingContext,
    renderer: Option<ServoRenderer>,
    display: Option<Display>,

    title: String,
    uri: String,
}

impl ServoEngine {
    pub fn new(
        queue: MtQueueHandle<State>,
        connection: WaylandConnection,
        display: Display,
        id: EngineId,
    ) -> Self {
        // TODO: Export SURFMAN_FORCE_GLES=1?
        //
        // Initialize TLS crypto provider at first engine spawn.
        static CRYT_INIT: Once = Once::new();
        CRYT_INIT.call_once(|| ring::default_provider().install_default().unwrap());

        // Create Servo Wayland connection from existing display.
        let display_ptr = NonNull::new(connection.backend().display_ptr().cast()).unwrap();
        let wayland_display = WaylandDisplayHandle::new(display_ptr);
        let raw_display = RawDisplayHandle::Wayland(wayland_display);
        let display_handle = unsafe { DisplayHandle::borrow_raw(raw_display) };
        let connection = Connection::from_display_handle(display_handle).unwrap();

        // TODO: Is it okay that this is a 1x1 headless buffer?
        //
        // Create Servo rendering context.
        let adapter = connection.create_adapter().unwrap();
        let rendering_context =
            RenderingContext::create(&connection, &adapter, Some(Size2D::new(1, 1))).unwrap();

        // Create engine viewport interface.
        let embedder = Box::new(Embedder::new(queue.clone(), id));
        let viewport = Rc::new(ServoViewport::new());

        let mut opts = servo::servo_config::opts::Opts::default();
        opts.initial_window_size = Size2D::new(100, 100);
        let servo = Servo::new(
            opts,
            Default::default(),
            rendering_context.clone(),
            embedder,
            viewport,
            None,
            CompositeTarget::Window,
        );
        let mut events = Vec::new();

        // Ensure Servo web view is created immediately.
        let uri = String::from("https://example.org"); // TODO: Just for testing
        let servo_id = WebViewId::new();
        let url = ServoUrl::parse(&uri).unwrap();
        events.push(EmbedderEvent::NewWebView(url, servo_id));

        Self {
            rendering_context,
            servo_id,
            events,
            queue,
            uri,
            id,
            display: Some(display),
            servo: Some(servo),
            renderer: Default::default(),
            title: Default::default(),
        }
    }
}

impl Engine for ServoEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn dirty(&mut self) -> bool {
        // Dispatch all pending events to Servo.
        let servo = self.servo.as_mut().unwrap();
        let needs_resize = servo.handle_events(self.events.drain(..));

        // TODO
        //
        // Handle all events from Servo.
        for (web_view_id, msg) in servo.get_events() {
            println!("Servo event: {msg:?}");

            match msg {
                // TODO: Allow?
                EmbedderMsg::AllowNavigationRequest(..) => (),
                EmbedderMsg::AllowOpeningWebView(..) => (),
                EmbedderMsg::AllowUnload(..) => (),
                EmbedderMsg::ChangePageTitle(title) => {
                    self.title = title.unwrap_or_default();
                    self.queue.set_engine_title(self.id, self.title.clone());
                },
                EmbedderMsg::ClearClipboardContents => (),
                EmbedderMsg::EventDelivered(..) => (),
                EmbedderMsg::GetClipboardContents(..) => (),
                EmbedderMsg::GetSelectedBluetoothDevice(..) => (),
                EmbedderMsg::HeadParsed => (),
                EmbedderMsg::HideIME => (),
                EmbedderMsg::HistoryChanged(..) => (),
                EmbedderMsg::Keyboard(..) => (),
                EmbedderMsg::LoadComplete => (),
                EmbedderMsg::LoadStart => (),
                EmbedderMsg::MediaSessionEvent(..) => (),
                EmbedderMsg::MoveTo(..) => (),
                EmbedderMsg::NewFavicon(..) => (),
                EmbedderMsg::OnDevtoolsStarted(..) => (),
                EmbedderMsg::Panic(..) => (),
                EmbedderMsg::PlayGamepadHapticEffect(..) => (),
                EmbedderMsg::Prompt(..) => (),
                EmbedderMsg::PromptPermission(..) => (),
                EmbedderMsg::ReadyToPresent(..) => (),
                EmbedderMsg::ReportProfile(..) => (),
                EmbedderMsg::ResizeTo(..) => (),
                EmbedderMsg::SelectFiles(..) => (),
                EmbedderMsg::SetClipboardContents(..) => (),
                EmbedderMsg::SetCursor(..) => (),
                EmbedderMsg::SetFullscreenState(..) => (),
                EmbedderMsg::ShowContextMenu(..) => (),
                EmbedderMsg::ShowIME(..) => (),
                EmbedderMsg::Shutdown => (),
                EmbedderMsg::Status(..) => (),
                EmbedderMsg::StopGamepadHapticEffect(..) => (),
                EmbedderMsg::WebResourceRequested(..) => (),
                EmbedderMsg::WebViewBlurred => (),
                EmbedderMsg::WebViewClosed(..) => (),
                EmbedderMsg::WebViewFocused(..) => (),
                // Select our new web view as primary one for rendering.
                EmbedderMsg::WebViewOpened(id) => {
                    // TODO: EmbedderEvent::MoveResizeWebView ?
                    self.events.push(EmbedderEvent::FocusWebView(id));
                    self.events.push(EmbedderEvent::RaiseWebViewToTop(id, true));
                },
            }
        }

        // TODO: Is there actually a way to get dirtiness?
        //  => EmbedderMsg::ReadyToPresent?
        true
    }

    fn wl_buffer(&mut self) -> Option<&WlBuffer> {
        // TODO: Combine with todo(surface)
        unreachable!()
    }

    fn todo(&mut self, surface: &WlSurface) -> bool {
        // Initialize renderer on first draw.
        let servo = self.servo.as_mut().unwrap();
        let window = servo.window();
        let size = window.size.get() * window.scale.get();
        let servo_renderer = self.renderer.get_or_insert_with(|| {
            let display = self.display.take().unwrap();
            ServoRenderer::new(display, &self.rendering_context, surface, size)
        });

        // Request redraw from Servo.
        servo.present();

        // TODO: Clean up this monstrosity if it works.
        servo_renderer.renderer.draw(size, |renderer| unsafe {
            self.rendering_context.with_front_buffer(|device, surface| {
                let texture =
                    device.create_surface_texture(&mut servo_renderer.context, surface).unwrap();
                // TODO: Texture ID here is 1, but 2 if we create our own texture first
                //  => Context the texture is created on should be correct
                let texture_id = device.surface_texture_object(&texture);

                // TODO: Framebuffer blitting will not work on the PP.
                //  => Temporary implementation
                // TODO: Testing.
                gl::ClearColor(1.0, 0.0, 1.0, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // TODO: This renders stuff upside down.
                //  => Either blit and require gles3, or maybe just implement mirroring in the
                //  vertex selection?
                let tmp_texture = crate::ui::renderer::Texture::from_raw(texture_id, 360, 720);
                renderer.draw_texture_at(&tmp_texture, Position::new(0., 0.), None);

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
                    360,
                    0,
                    360 + size.width as i32,
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
        self.servo.as_ref().unwrap().window().size.replace(size);
        self.events.push(EmbedderEvent::WindowResize);
    }

    fn set_scale(&mut self, scale: f64) {
        self.servo.as_ref().unwrap().window().scale.replace(scale);
        self.events.push(EmbedderEvent::WindowResize);
    }

    fn press_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        let event = KeyboardEvent {
            modifiers: servo_modifiers(modifiers),
            state: KeyState::Down,
            // TODO: These are going to be a nightmare.
            key: todo!(),
            code: todo!(),
            location: todo!(),
            is_composing: false,
            repeat: false,
        };
        self.events.push(EmbedderEvent::Keyboard(event));
    }

    fn release_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        let event = KeyboardEvent {
            modifiers: servo_modifiers(modifiers),
            state: KeyState::Up,
            // TODO: These are going to be a nightmare.
            key: todo!(),
            code: todo!(),
            location: todo!(),
            is_composing: false,
            repeat: false,
        };
        self.events.push(EmbedderEvent::Keyboard(event));
    }

    fn pointer_axis(
        &mut self,
        time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        modifiers: Modifiers,
    ) {
        let delta = WheelDelta {
            x: horizontal.absolute,
            y: vertical.absolute,
            z: 0.,
            mode: WheelMode::DeltaPixel,
        };
        self.events.push(EmbedderEvent::Wheel(delta, position.into()));
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

        self.events.push(EmbedderEvent::MouseWindowEventClass(event));
    }

    fn pointer_motion(&mut self, _time: u32, position: Position<f64>, _modifiers: Modifiers) {
        self.events.push(EmbedderEvent::MouseWindowMoveEventClass(position.into()));
    }

    fn pointer_enter(&mut self, position: Position<f64>, modifiers: Modifiers) {
        // TODO
    }

    fn pointer_leave(&mut self, position: Position<f64>, modifiers: Modifiers) {
        // TODO
    }

    fn touch_up(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        self.events.push(EmbedderEvent::Touch(TouchEventType::Up, TouchId(id), position.into()));
    }

    fn touch_down(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        self.events.push(EmbedderEvent::Touch(TouchEventType::Down, TouchId(id), position.into()));
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        self.events.push(EmbedderEvent::Touch(TouchEventType::Move, TouchId(id), position.into()));
    }

    // TODO: This is called for 'unknown browsing context'
    fn load_uri(&mut self, uri: &str) {
        self.uri = uri.into();
        let url = ServoUrl::parse(uri).unwrap();
        self.events.push(EmbedderEvent::LoadUrl(self.servo_id, url));
    }

    fn load_prev(&mut self) {
        self.events.push(EmbedderEvent::Navigation(self.servo_id, TraversalDirection::Back(1)));
    }

    fn uri(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.uri)
    }

    fn title(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }

    fn text_input_state(&self) -> TextInputChange {
        todo!()
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        todo!()
    }

    fn commit_string(&mut self, text: String) {
        todo!()
    }

    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        todo!()
    }

    fn paste(&mut self, text: String) {
        todo!()
    }

    fn clear_focus(&mut self) {
        self.events.push(EmbedderEvent::BlurWebView);
    }

    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize) {
        todo!()
    }

    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>) {
        todo!()
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        if fullscreen {
            todo!();
        } else {
            self.events.push(EmbedderEvent::ExitFullScreen(self.servo_id));
        }
    }

    fn session(&self) -> Vec<u8> {
        // TODO
        Vec::new()
    }

    fn restore_session(&self, session: Vec<u8>) {
        // TODO
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
        self.queue.clone().handle_servo_events(self.engine_id);
    }
}

struct ServoViewport {
    scale: Cell<f64>,
    size: Cell<Size>,
}

impl ServoViewport {
    fn new() -> Self {
        Self { scale: Cell::new(1.), size: Default::default() }
    }
}

impl WindowMethods for ServoViewport {
    fn get_coordinates(&self) -> EmbedderCoordinates {
        let scale = self.scale.get();
        let size = self.size.get();
        EmbedderCoordinates {
            window_rect: Box2D::from_size(size.into()),
            viewport: Box2D::from_size(size.into()),
            hidpi_factor: Scale::new(scale as f32),
            available_screen_size: size.into(),
            framebuffer: size.into(),
            screen_size: size.into(),
        }
    }

    fn set_animation_state(&self, _state: AnimationState) {}
}

/// Convert SCTK to Servo modifiers.
fn servo_modifiers(modifiers: Modifiers) -> ServoModifiers {
    let mut servo_modifiers = ServoModifiers::default();
    servo_modifiers.set(ServoModifiers::SHIFT, modifiers.shift);
    servo_modifiers.set(ServoModifiers::CONTROL, modifiers.ctrl);
    servo_modifiers.set(ServoModifiers::ALT, modifiers.alt);
    servo_modifiers.set(ServoModifiers::CAPS_LOCK, modifiers.caps_lock);
    servo_modifiers.set(ServoModifiers::META, modifiers.logo);
    servo_modifiers.set(ServoModifiers::NUM_LOCK, modifiers.num_lock);
    servo_modifiers
}
