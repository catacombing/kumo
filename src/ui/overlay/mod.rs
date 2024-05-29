//! Rendering to the overlay surface.

use funq::MtQueueHandle;
use glutin::display::Display;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::ui::overlay::tabs::Tabs;
use crate::ui::Renderer;
use crate::{gl, rect_contains, Position, Size, State, WindowId};

pub mod tabs;

/// Overlay surface element.
pub trait Popup {
    /// Check whether the popup is active.
    fn visible(&self) -> bool;

    /// Show or hide a popup.
    fn set_visible(&mut self, visible: bool);

    /// Check whether the popup requires a redraw.
    fn dirty(&self) -> bool;

    /// Redraw the popup.
    fn draw(&mut self, renderer: &Renderer);

    /// Popup's logical location.
    fn position(&self) -> Position;

    /// Update maximum available size.
    fn set_size(&mut self, size: Size);

    /// Get the logical popup size.
    fn size(&self) -> Size;

    /// Update scale factor.
    fn set_scale(&mut self, scale: f64);

    /// Popup's logical opaque size.
    fn opaque_region(&self) -> Size;

    /// Handle touch press events.
    fn touch_down(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle touch motion events.
    fn touch_motion(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle touch release events.
    fn touch_up(&mut self, _time: u32, _id: i32, _modifiers: Modifiers) {}
}

/// Overlay UI surface.
pub struct Overlay {
    renderer: Renderer,

    surface: WlSurface,
    viewport: WpViewport,

    popups: Popups,

    touch_focus: Option<usize>,

    size: Size,
    scale: f64,
}

impl Overlay {
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        surface: WlSurface,
        viewport: WpViewport,
    ) -> Self {
        let renderer = Renderer::new(display, surface.clone());

        let popups = Popups::new(window_id, queue);

        Self {
            viewport,
            renderer,
            surface,
            popups,
            scale: 1.0,
            touch_focus: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the logical UI size.
    pub fn set_size(&mut self, compositor: &CompositorState, size: Size) {
        self.size = size;

        // Update popups.
        for popup in self.popups.iter_mut() {
            popup.set_size(size);
        }

        // Update opaque region.
        if let Ok(region) = Region::new(compositor) {
            for popup in self.popups.iter() {
                let pos = popup.position();
                let size: Size<i32> = popup.opaque_region().into();
                region.add(pos.x, pos.y, size.width, size.height);
            }
            self.surface.set_opaque_region(Some(region.wl_region()));
        }

        // Update input region.
        if let Ok(region) = Region::new(compositor) {
            for popup in self.popups.iter() {
                let pos = popup.position();
                let size: Size<i32> = popup.size().into();
                region.add(pos.x, pos.y, size.width, size.height);
            }
            self.surface.set_input_region(Some(region.wl_region()));
        }
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;

        // Update popups.
        for popup in self.popups.iter_mut() {
            popup.set_scale(scale);
        }
    }

    /// Render current overlay state.
    ///
    /// Returns `true` if rendering was performed.
    pub fn draw(&mut self) -> bool {
        let mut any_visible = false;
        let mut any_dirty = false;
        for popup in self.popups.iter() {
            any_visible |= popup.visible();
            any_dirty |= popup.dirty();

            // Stop as soon as redraw requirement has been determined.
            if any_visible && any_dirty {
                break;
            }
        }

        // Hide surface if there's no visible popups.
        if !any_visible {
            self.surface.attach(None, 0, 0);
            self.surface.commit();
            return false;
        }

        // Don't redraw if rendering is up to date.
        if !any_dirty {
            return false;
        }

        // Update viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Mark entire surface as damaged.
        self.surface.damage(0, 0, self.size.width as i32, self.size.height as i32);

        // Redraw all popups.
        let physical_size = self.size * self.scale;
        self.renderer.draw(physical_size, |renderer| {
            unsafe {
                // Clear background.
                gl::ClearColor(0., 0., 0., 0.);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw the popups.
                //
                // NOTE: We still draw invisible popups to allow them to clear their dirty flags
                // after going invisible. Popups must not draw anything while invisible.
                for popup in self.popups.iter_mut() {
                    popup.draw(renderer);
                }
            }
        });

        true
    }

    /// Check if the popup surface is fully opaque.
    pub fn opaque(&self) -> bool {
        // NOTE: This is a simplification of actual popup opaque region combination
        // since it's currently not possible to make the overlay surface fully
        // opaque by combining multiple smaller popups.
        self.popups.tabs.visible()
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        self.touch_focus = None;
        for (i, popup) in self.popups.iter_mut().enumerate() {
            let popup_position = popup.position().into();
            if rect_contains(popup_position, popup.size().into(), position) {
                popup.touch_down(time, id, position - popup_position, modifiers);
                self.touch_focus = Some(i);
                break;
            }
        }
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        let focused_popup = self.touch_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            let popup_position = popup.position().into();
            popup.touch_motion(time, id, position - popup_position, modifiers);
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32, id: i32, modifiers: Modifiers) {
        let focused_popup = self.touch_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.touch_up(time, id, modifiers);
        }
    }

    /// Check if any popup is dirty.
    pub fn dirty(&self) -> bool {
        self.popups.iter().any(|popup| popup.dirty())
    }

    /// Get the underlying Wayland surface.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Get mutable access to the tabs popup.
    pub fn tabs_mut(&mut self) -> &mut Tabs {
        &mut self.popups.tabs
    }
}

/// Overlay popup windows.
struct Popups {
    tabs: Tabs,
}

impl Popups {
    fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        let tabs = Tabs::new(window_id, queue);
        Self { tabs }
    }

    /// Non-mutable popup iterator.
    fn iter(&self) -> impl Iterator<Item = &dyn Popup> {
        [&self.tabs as &dyn Popup].into_iter()
    }

    /// Mutable popup iterator.
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut dyn Popup> {
        [&mut self.tabs as &mut dyn Popup].into_iter()
    }
}
