//! Engine backdrop to ensure window opacity.
//!
//! This surface is rendered behind the engine surface to ensure that the window
//! is fully opaque even if the engine is pending a resize or fully transparent.

use _spb::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use glutin::display::Display;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::single_pixel_buffer::v1::client as _spb;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::config::colors::BG;
use crate::ui::Ui;
use crate::ui::renderer::Renderer;
use crate::wayland::protocols::ProtocolStates;
use crate::{Size, State, gl};

/// Single-color engine backdrop surface.
pub struct EngineBackdrop {
    backend: Backend,

    wayland_queue: QueueHandle<State>,

    surface: WlSurface,
    viewport: WpViewport,
    compositor: CompositorState,

    size: Size,
    scale: f64,

    dirty: bool,
}

impl EngineBackdrop {
    pub fn new(
        display: Display,
        surface: WlSurface,
        viewport: WpViewport,
        protocol_states: &ProtocolStates,
        wayland_queue: QueueHandle<State>,
    ) -> Self {
        let compositor = protocol_states.compositor.clone();
        let backend = match protocol_states.single_pixel_buffer.clone() {
            Some(spb) => Backend::Spb(spb),
            None => Backend::Gl(Renderer::new(display, surface.clone())),
        };

        Self {
            wayland_queue,
            compositor,
            viewport,
            backend,
            surface,
            scale: 1.,
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the logical UI size.
    pub fn set_size(&mut self, size: Size) {
        let toolbar_height = Ui::toolbar_height();
        self.size = Size::new(size.width, size.height - toolbar_height);
        self.dirty = true;

        // Update opaque region.
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, size.width as i32, size.height as i32);
            self.surface.set_opaque_region(Some(region.wl_region()));
        }
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;
    }

    /// Render engine backdrop.
    ///
    /// Returns `true` if rendering was performed.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&mut self) -> bool {
        // Abort early if backdrop is up to date.
        if !self.dirty {
            return false;
        }
        self.dirty = false;

        // Update viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Mark entire UI as damaged.
        self.surface.damage(0, 0, self.size.width as i32, self.size.height as i32);

        // Render the UI.
        match &mut self.backend {
            Backend::Gl(renderer) => {
                let physical_size = self.size * self.scale;
                renderer.draw(physical_size, |_renderer| unsafe {
                    gl::ClearColor(BG[0] as f32, BG[1] as f32, BG[2] as f32, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                });
            },
            Backend::Spb(spb) => {
                let queue = &self.wayland_queue;
                let [r, g, b] = [
                    (BG[0] * u32::MAX as f64).round() as u32,
                    (BG[1] * u32::MAX as f64).round() as u32,
                    (BG[2] * u32::MAX as f64).round() as u32,
                ];
                let buffer = spb.create_u32_rgba_buffer(r, g, b, u32::MAX, queue, ());
                self.surface.attach(Some(&buffer), 0, 0);
            },
        }

        true
    }

    /// Get underlying Wayland surface.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }
}

/// Single-color buffer implementation backend.
enum Backend {
    Gl(Renderer),
    Spb(WpSinglePixelBufferManagerV1),
}
