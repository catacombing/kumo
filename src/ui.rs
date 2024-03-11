//! Non-browser UI.

use std::num::NonZeroU32;

use glutin::config::{Api, Config, ConfigTemplateBuilder};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::Display;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Proxy;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::Modifiers;
use smithay_client_toolkit::seat::pointer::AxisScroll;

use crate::gl;

/// Logical height of the UI surface.
pub const UI_HEIGHT: u32 = 75;

pub struct Ui {
    egl_surface: Option<Surface<WindowSurface>>,
    egl_context: PossiblyCurrentContext,
    egl_config: Config,

    subsurface: WlSubsurface,
    surface: WlSurface,
    viewport: WpViewport,

    width: u32,
    height: u32,
    scale: f64,

    dirty: bool,
}

impl Ui {
    pub fn new(
        display: &Display,
        (subsurface, surface): (WlSubsurface, WlSurface),
        viewport: WpViewport,
    ) -> Self {
        // Create EGL config.
        let config_template = ConfigTemplateBuilder::new().with_api(Api::GLES2).build();
        let egl_config = unsafe {
            display
                .find_configs(config_template)
                .ok()
                .and_then(|mut configs| configs.next())
                .unwrap()
        };

        // Create EGL context.
        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);
        let egl_context =
            unsafe { display.create_context(&egl_config, &context_attributes).unwrap() };
        let egl_context = egl_context.treat_as_possibly_current();

        Self {
            egl_context,
            egl_config,
            subsurface,
            viewport,
            surface,
            scale: 1.0,
            egl_surface: Default::default(),
            height: Default::default(),
            width: Default::default(),
            dirty: Default::default(),
        }
    }

    /// Update the surface geometry.
    pub fn set_geometry(&mut self, display: &Display, x: i32, y: i32, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.dirty = true;

        // Update subsurface location.
        self.subsurface.set_position(x, y);

        // Ensure EGL surface is initialized.
        if self.egl_surface.is_none() {
            self.egl_surface = Some(self.create_surface(display));
        }

        // Update surface size.
        self.egl_surface.as_ref().unwrap().resize(
            &self.egl_context,
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        );

        // Update browser's viewporter logical render size.
        self.viewport.set_destination(self.width as i32, self.height as i32);
    }

    /// Render current UI state.
    ///
    /// Returns `true` if rendering was performed.
    pub fn draw(&mut self) -> bool {
        if !self.dirty || self.egl_surface.is_none() {
            return false;
        }
        self.dirty = false;

        // Bind the current EGL surface for rendering.
        let egl_surface = self.egl_surface.as_ref().unwrap();
        self.egl_context.make_current(egl_surface).unwrap();

        // Render the UI.
        unsafe {
            gl::ClearColor(0.1, 0.1, 0.1, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Submit the frame.
        egl_surface.swap_buffers(&self.egl_context).unwrap();

        true
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;
    }

    /// Check whether a surface is owned by this UI.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        &self.surface == surface
    }

    /// Handle scroll axis events.
    pub fn pointer_axis(
        &self,
        _time: u32,
        _x: f64,
        _y: f64,
        _horizontal: AxisScroll,
        _vertical: AxisScroll,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle pointer button events.
    pub fn pointer_button(
        &self,
        _time: u32,
        _x: f64,
        _y: f64,
        _button: u32,
        _state: u32,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle pointer motion events.
    pub fn pointer_motion(&self, _time: u32, _x: f64, _y: f64, _modifiers: Modifiers) {}

    /// Handle touch press events.
    pub fn touch_down(&mut self, _time: u32, _id: i32, _x: f64, _y: f64, _modifiers: Modifiers) {}

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, _id: i32, _modifiers: Modifiers) {}

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, _time: u32, _id: i32, _x: f64, _y: f64, _modifiers: Modifiers) {}

    /// Create a new EGL surface.
    fn create_surface(&self, display: &Display) -> Surface<WindowSurface> {
        assert!(self.width > 0 && self.height > 0);

        let mut raw_window_handle = WaylandWindowHandle::empty();
        raw_window_handle.surface = self.surface.id().as_ptr().cast();
        let raw_window_handle = RawWindowHandle::Wayland(raw_window_handle);
        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_window_handle,
            NonZeroU32::new(self.width).unwrap(),
            NonZeroU32::new(self.height).unwrap(),
        );

        let egl_surface = unsafe {
            display.create_window_surface(&self.egl_config, &surface_attributes).unwrap()
        };

        // Ensure rendering never blocks.
        self.egl_context.make_current(&egl_surface).unwrap();
        egl_surface.set_swap_interval(&self.egl_context, SwapInterval::DontWait).unwrap();

        egl_surface
    }
}
