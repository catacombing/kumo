use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_region::WlRegion;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::AxisScroll;
use uuid::Uuid;

use crate::ui::overlay::option_menu::OptionMenuId;
use crate::window::TextInputChange;
use crate::{Position, Size, WindowId};

pub mod webkit;

/// Default engine background color.
pub const BG: [f64; 3] = [0.1, 0.1, 0.1];

// Constants for the default tab group.
pub const NO_GROUP: Group = Group::none();
pub const NO_GROUP_ID: GroupId = NO_GROUP.id();

pub trait Engine {
    /// Get the engine's unique ID.
    fn id(&self) -> EngineId;

    /// Check if the engine requires a redraw.
    fn dirty(&self) -> bool;

    /// Get the Wayland buffer for rendering the engine's current content.
    fn wl_buffer(&self) -> Option<&WlBuffer>;

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
    fn frame_done(&mut self);

    /// Notify engine that a buffer was released.
    fn buffer_released(&mut self, buffer: &WlBuffer);

    /// Update DMA buffer feedback.
    fn dmabuf_feedback(&mut self, feedback: &DmabufFeedback);

    /// Get the buffer's opaque region.
    fn opaque_region(&self) -> Option<&WlRegion>;

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
    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32);

    /// Clear engine focus.
    fn clear_focus(&mut self);

    /// Submit option menu item selection.
    fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize);

    /// Close option menu.
    fn close_option_menu(&mut self, menu_id: Option<OptionMenuId>);

    /// Notify engine about change to the fullscreen state.
    fn set_fullscreen(&mut self, fulscreened: bool);

    /// Get a serialized version of the current session.
    fn session(&self) -> Vec<u8>;

    /// Restore a browser session.
    fn restore_session(&self, session: Vec<u8>);

    fn as_any(&mut self) -> &mut dyn Any;
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
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Group {
    /// Globally unique group ID.
    id: GroupId,

    /// Whether data for this group should be persisted.
    pub ephemeral: bool,
}

impl Group {
    /// Create a new tab group.
    pub fn new(ephemeral: bool) -> Self {
        Self { id: GroupId(Uuid::new_v4()), ephemeral }
    }

    /// Create a tab group with a fixed UUID.
    ///
    /// Two different tab groups must never be created with the same UUID.
    pub fn with_uuid(uuid: Uuid, ephemeral: bool) -> Self {
        Self { id: GroupId(uuid), ephemeral }
    }

    /// Get the default tab group.
    pub const fn none() -> Self {
        Self { id: GroupId(Uuid::nil()), ephemeral: false }
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
