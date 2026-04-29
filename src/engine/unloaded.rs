//! No-op engine implementation.

use std::any::Any;
use std::borrow::Cow;

use glib::GString;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;

use crate::engine::{Engine, EngineId, EngineType, Favicon};
use crate::ui::overlay::option_menu::OptionMenuId;
use crate::window::TextInputChange;
use crate::{Position, Size};

/// An unloaded browser engine.
///
/// This is used to store data necessary to immitate a generic browser engine in
/// the UI, without having to use up the resources to keep the engine alive.
pub struct UnloadedEngine {
    id: EngineId,

    surface: WlSurface,

    favicon_uri: Option<GString>,
    favicon: Option<Favicon>,
    uri: Option<String>,
    session: Vec<u8>,
    title: String,

    size: Size,
    scale: f64,
}

impl UnloadedEngine {
    pub fn new(surface: WlSurface, id: EngineId, uri: Option<&str>) -> Self {
        Self {
            surface,
            id,
            uri: uri.map(String::from),
            scale: 1.,
            favicon_uri: Default::default(),
            favicon: Default::default(),
            session: Default::default(),
            title: Default::default(),
            size: Default::default(),
        }
    }

    /// Convert any engine into an unloaded engine.
    pub fn from_engine(surface: WlSurface, size: Size, scale: f64, engine: &dyn Engine) -> Self {
        // Get current URL, filtering out blank pages.
        let uri = engine.uri().to_string();
        let uri = if uri.is_empty() || uri == "about:blank" { None } else { Some(uri) };

        let favicon_uri = engine.favicon_uri();
        let title = engine.title().to_string();
        let favicon = engine.favicon();
        let session = engine.session();
        let id = engine.id();

        Self { favicon_uri, favicon, session, surface, title, scale, size, uri, id }
    }
}

impl Engine for UnloadedEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn engine_type(&self) -> EngineType {
        EngineType::Unloaded
    }

    fn dirty(&mut self) -> bool {
        false
    }

    fn draw(&mut self) -> bool {
        self.surface.attach(None, 0, 0);
        false
    }

    fn buffer_size(&self) -> Size {
        self.size * self.scale
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }

    fn press_key(&mut self, _time: u32, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {}

    fn release_key(&mut self, _time: u32, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {}

    fn pointer_axis(
        &mut self,
        _time: u32,
        _position: Position<f64>,
        _horizontal: AxisScroll,
        _vertical: AxisScroll,
        _modifiers: Modifiers,
    ) {
    }

    fn pointer_button(
        &mut self,
        _time: u32,
        _position: Position<f64>,
        _button: u32,
        _down: bool,
        _modifiers: Modifiers,
    ) {
    }

    fn pointer_motion(&mut self, _time: u32, _position: Position<f64>, _modifiers: Modifiers) {}

    fn touch_up(&mut self, _time: u32, _id: i32, _position: Position<f64>, _modifiers: Modifiers) {}

    fn touch_down(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    fn reload(&mut self) {}

    fn load_uri(&mut self, uri: &str) {
        self.uri = Some(uri.into());
    }

    fn load_prev(&mut self) {}

    fn has_prev(&self) -> bool {
        false
    }

    fn uri(&self) -> Cow<'_, str> {
        self.uri.as_ref().map_or(Cow::Borrowed(""), |uri| uri.into())
    }

    fn title(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }

    fn text_input_state(&mut self) -> TextInputChange {
        TextInputChange::Disabled
    }

    fn delete_surrounding_text(&mut self, _before_length: u32, _after_length: u32) {}

    fn commit_string(&mut self, _text: String) {}

    fn set_preedit_string(&mut self, _text: String, _cursor_begin: i32, _cursor_end: i32) {}

    fn clear_focus(&mut self) {}

    fn submit_option_menu(&mut self, _menu_id: OptionMenuId, _index: usize) {}

    fn close_option_menu(&mut self, _menu_id: Option<OptionMenuId>) {}

    fn set_fullscreen(&mut self, _fullscreen: bool) {}

    fn session(&self) -> Vec<u8> {
        self.session.clone()
    }

    fn restore_session(&mut self, session: Vec<u8>) {
        self.session = session;
    }

    fn favicon(&self) -> Option<Favicon> {
        self.favicon.clone()
    }

    fn favicon_uri(&self) -> Option<GString> {
        self.favicon_uri.clone()
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}
