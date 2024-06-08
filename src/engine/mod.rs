use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;

use crate::ui::overlay::option_menu::OptionMenuId;
use crate::window::TextInputChange;
use crate::{Position, Size, WindowId};

pub mod webkit;

/// Default engine background color.
pub const BG: [f64; 3] = [0.1, 0.1, 0.1];

pub trait Engine {
    /// Get the engine's unique ID.
    fn id(&self) -> EngineId;

    /// Get the Wayland buffer for rendering the engine's current content.
    fn wl_buffer(&self) -> Option<&WlBuffer>;

    /// Check if the engine requires a redraw.
    fn dirty(&self) -> bool;

    /// Notify engine that the frame was completed.
    fn frame_done(&mut self);

    /// Update the browser engine's size.
    fn set_size(&mut self, size: Size);

    /// Get the Wayland buffer's current physical size.
    fn buffer_size(&self) -> Size;

    /// Update the browser engine's scale.
    fn set_scale(&mut self, scale: f64);

    /// Handle key down.
    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers);

    /// Handle key up.
    fn release_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers);

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
        state: u32,
        modifiers: Modifiers,
    );

    /// Handle pointer motion.
    fn pointer_motion(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers);

    /// Handle touch press.
    fn touch_up(
        &mut self,
        touch_points: &HashMap<i32, Position<f64>>,
        time: u32,
        id: i32,
        modifiers: Modifiers,
    );

    /// Handle touch release.
    fn touch_down(
        &mut self,
        touch_points: &HashMap<i32, Position<f64>>,
        time: u32,
        id: i32,
        modifiers: Modifiers,
    );

    /// Handle touch motion.
    fn touch_motion(
        &mut self,
        touch_points: &HashMap<i32, Position<f64>>,
        time: u32,
        id: i32,
        modifiers: Modifiers,
    );

    /// Load a new page.
    fn load_uri(&self, uri: &str);

    /// Go to the previous page.
    fn load_prev(&self);

    /// Get current URI.
    fn uri(&self) -> String;

    /// Get tab title.
    fn title(&self) -> String;

    /// Get IME text_input state.
    fn text_input_state(&self) -> TextInputChange;

    /// Delete text around the current cursor position.
    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32);

    /// Insert text at the current cursor position.
    fn commit_string(&mut self, text: String);

    /// Set preedit text at the current cursor position.
    fn preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32);

    /// Clear engine focus.
    fn clear_focus(&mut self);

    /// Submit option menu item selection.
    fn option_menu_submit(&mut self, menu_id: OptionMenuId, index: usize);

    /// Close option menu.
    fn option_menu_close(&mut self, menu_id: Option<OptionMenuId>);

    /// Notify engine that entering fullscreen was successful.
    fn confirm_enter_fullscreen(&mut self);

    /// Notify engine that leaving fullscreen was successful.
    fn confirm_leave_fullscreen(&mut self);

    fn as_any(&mut self) -> &mut dyn Any;
}

/// Unique identifier for one engine instance.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EngineId {
    window_id: WindowId,
    id: usize,
}

impl EngineId {
    pub fn new(window_id: WindowId) -> Self {
        static NEXT_ENGINE_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed);
        Self { id, window_id }
    }

    /// Get the engine's window.
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }
}
