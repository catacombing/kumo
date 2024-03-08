use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::WindowId;

pub mod webkit;

pub trait Engine {
    /// Get unique engine ID.
    fn id(&self) -> EngineId;

    /// Get the Wayland buffer for rendering the engine's current content.
    fn wl_buffer(&self) -> Option<&WlBuffer>;

    /// Notify engine that the frame was completed.
    fn frame_done(&self);

    /// Update the browser engine's size.
    fn set_size(&mut self, width: u32, height: u32);

    /// Update the browser engine's scale.
    fn set_scale(&mut self, scale: f64);

    /// Handle key down.
    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers);

    /// Handle key up.
    fn release_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers);

    /// Load a new page.
    fn load_uri(&self, uri: &str);

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
