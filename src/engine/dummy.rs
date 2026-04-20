//! No-op engine implementation.

use std::any::Any;
use std::borrow::Cow;

use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;

use crate::engine::{Engine, EngineId, EngineType};
use crate::ui::overlay::option_menu::OptionMenuId;
use crate::window::TextInputChange;
use crate::{Position, Size};

/// Dummy engine.
///
/// This no-op engine can be used as a placeholder for a real engine.
pub struct DummyEngine {
    id: EngineId,

    surface: WlSurface,

    size: Size,
    scale: f64,
}

impl DummyEngine {
    pub fn new(id: EngineId, surface: WlSurface) -> Self {
        Self { surface, id, scale: 1., size: Default::default() }
    }
}

impl Engine for DummyEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn engine_type(&self) -> EngineType {
        EngineType::Dummy
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

    fn load_uri(&mut self, _uri: &str) {}

    fn load_prev(&mut self) {}

    fn has_prev(&self) -> bool {
        false
    }

    fn uri(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn title(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
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

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}
