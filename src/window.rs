//! Browser window handling.

use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};

use _text_input::zwp_text_input_v3::{ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3};
use funq::StQueueHandle;
use glutin::display::Display;
use indexmap::IndexMap;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{AxisScroll, BTN_LEFT};
use smithay_client_toolkit::shell::xdg::window::{Window as XdgWindow, WindowDecorations};
use smithay_client_toolkit::shell::WaylandSurface;

use crate::engine::webkit::{WebKitEngine, WebKitError};
use crate::engine::{Engine, EngineId};
use crate::ui::tabs::TabsUi;
use crate::ui::{Ui, UI_HEIGHT};
use crate::wayland::protocols::ProtocolStates;
use crate::{Position, Size, State};

// Default window size.
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

/// Fallback color when waiting for engine frame.
const BACKGROUND_COLOR: [u32; 4] = [402653184, 402653184, 402653184, u32::MAX];

#[funq::callbacks(State)]
pub trait WindowHandler {
    /// Close a browser window.
    fn close_window(&mut self, window_id: WindowId);
}

impl WindowHandler for State {
    fn close_window(&mut self, window_id: WindowId) {
        // Remove the window and mark it as closed.
        self.windows.retain(|id, window| {
            let retain = *id != window_id;
            if !retain {
                window.closed = true;
            }
            retain
        });

        // Quit if all windows were closed.
        if self.windows.is_empty() {
            self.main_loop.quit();
        }
    }
}

/// Wayland window.
pub struct Window {
    id: WindowId,

    tabs: IndexMap<EngineId, Box<dyn Engine>>,
    active_tab: EngineId,

    text_input: Option<TextInput>,
    wayland_queue: QueueHandle<State>,
    initial_configure_done: bool,
    connection: Connection,
    egl_display: Display,
    xdg: XdgWindow,
    viewport: WpViewport,
    scale: f64,
    size: Size,

    fallback_buffer: Option<WlBuffer>,

    queue: StQueueHandle<State>,

    tabs_ui: TabsUi,
    ui: Ui,

    // Touch point position tracking.
    touch_points: HashMap<i32, Position<f64>>,

    stalled: bool,
    closed: bool,
    dirty: bool,
}

impl Window {
    pub fn new(
        protocol_states: &ProtocolStates,
        connection: Connection,
        egl_display: Display,
        queue: StQueueHandle<State>,
        wayland_queue: QueueHandle<State>,
    ) -> Result<Self, WebKitError> {
        let surface = protocol_states.compositor.create_surface(&wayland_queue);

        // Enable fractional scaling.
        protocol_states.fractional_scale.fractional_scaling(&wayland_queue, &surface);

        // Enable viewporter for the browser surface.
        let viewport = protocol_states.viewporter.viewport(&wayland_queue, &surface);

        // Create UI renderer.
        let id = WindowId::new();
        let ui_surface =
            protocol_states.subcompositor.create_subsurface(surface.clone(), &wayland_queue);
        let ui_viewport = protocol_states.viewporter.viewport(&wayland_queue, &ui_surface.1);
        let mut ui = Ui::new(id, queue.handle(), egl_display.clone(), ui_surface, ui_viewport);

        // Create tabs UI renderer.
        let (_, tabs_ui_surface) =
            protocol_states.subcompositor.create_subsurface(surface.clone(), &wayland_queue);
        let tabs_ui_viewport =
            protocol_states.viewporter.viewport(&wayland_queue, &tabs_ui_surface);
        let mut tabs_ui =
            TabsUi::new(id, queue.handle(), egl_display.clone(), tabs_ui_surface, tabs_ui_viewport);

        // Create XDG window.
        let decorations = WindowDecorations::RequestServer;
        let xdg = protocol_states.xdg_shell.create_window(surface, decorations, &wayland_queue);
        xdg.set_title("Kumo");
        xdg.set_app_id("Kumo");
        xdg.commit();

        // Create single-pixel buffer as fallback when waiting for engine rendering.
        let fallback_buffer = protocol_states.single_pixel_buffer.as_ref().map(|spb| {
            let [r, g, b, a] = BACKGROUND_COLOR;
            spb.create_u32_rgba_buffer(r, g, b, a, &wayland_queue, ())
        });

        let size = Size::new(DEFAULT_WIDTH, DEFAULT_HEIGHT);
        let active_tab = EngineId::new(id);

        // Resize UI elements to the initial window size.
        tabs_ui.set_size(Size::new(size.width, size.height));
        let ui_pos = Position::new(0, (size.height - UI_HEIGHT) as i32);
        let ui_size = Size::new(size.width, UI_HEIGHT);
        ui.set_geometry(ui_pos, ui_size);

        let mut window = Self {
            fallback_buffer,
            wayland_queue,
            egl_display,
            connection,
            active_tab,
            viewport,
            tabs_ui,
            queue,
            size,
            xdg,
            ui,
            id,
            stalled: true,
            scale: 1.,
            initial_configure_done: Default::default(),
            touch_points: Default::default(),
            text_input: Default::default(),
            closed: Default::default(),
            dirty: Default::default(),
            tabs: Default::default(),
        };

        // Create initial browser tab.
        window.add_tab()?;

        Ok(window)
    }

    /// Get the ID of this window.
    pub fn id(&self) -> WindowId {
        self.id
    }

    /// Get this window's tabs.
    pub fn tabs(&self) -> &IndexMap<EngineId, Box<dyn Engine>> {
        &self.tabs
    }

    /// Get mutable reference to this window's tabs.
    pub fn tabs_mut(&mut self) -> &mut IndexMap<EngineId, Box<dyn Engine>> {
        &mut self.tabs
    }

    /// Add a tab to the window.
    pub fn add_tab(&mut self) -> Result<(), WebKitError> {
        // Create a new browser engine.
        let size = Size::new(self.size.width, self.size.height - UI_HEIGHT);
        let engine_id = EngineId::new(self.id);
        let engine =
            WebKitEngine::new(&self.egl_display, self.queue.clone(), engine_id, size, self.scale)?;
        self.tabs.insert(engine_id, Box::new(engine));

        // Switch the active tab.
        self.active_tab = engine_id;

        // Immediately focus URI bar to allow text input.
        self.ui.keyboard_focus_uribar();
        self.ui.set_uri("");

        self.unstall();

        Ok(())
    }

    /// Get this window's active tab.
    pub fn active_tab(&self) -> EngineId {
        self.active_tab
    }

    /// Switch between tabs.
    pub fn set_active_tab(&mut self, engine_id: EngineId) {
        self.active_tab = engine_id;
        let uri = self.tabs.get_mut(&self.active_tab).unwrap().uri();
        self.ui.set_uri(&uri);
        self.unstall();
    }

    /// Close a tabs.
    pub fn close_tab(&mut self, engine_id: EngineId) {
        // Remove engine and get the position it was in.
        let index = match self.tabs.shift_remove_full(&engine_id) {
            Some((index, ..)) => index,
            None => return,
        };

        if engine_id == self.active_tab {
            match self.tabs.get_index(index.saturating_sub(1)) {
                // If the closed tab was active, switch to the first one.
                Some((&engine_id, _)) => self.set_active_tab(engine_id),
                // If there's no more tabs, close the window.
                None => {
                    self.queue.close_window(self.id);
                    self.closed = true;
                },
            }
        }

        // Force tabs UI redraw.
        self.tabs_ui.mark_dirty();
        self.unstall();
    }

    /// Redraw the window.
    pub fn draw(&mut self) {
        // Ignore rendering before initial configure or after shutdown.
        if self.closed || !self.initial_configure_done {
            return;
        }

        // Mark window as stalled if no rendering is performed.
        self.stalled = true;

        // Redraw the active browser engine.
        let surface = self.xdg.wl_surface();
        let tabs_ui_visible = self.tabs_ui.visible();
        if !tabs_ui_visible {
            let engine = self.tabs.get_mut(&self.active_tab).unwrap();
            match (engine.wl_buffer(), &self.fallback_buffer) {
                // Render buffer if one is attached and requires redraw.
                (Some(engine_buffer), _) if engine.dirty() || self.dirty => {
                    // Attach engine buffer to primary surface.
                    surface.attach(Some(engine_buffer), 0, 0);
                    let height = (self.size.height - UI_HEIGHT) as i32;
                    surface.damage(0, 0, self.size.width as i32, height);

                    // Request new engine frame.
                    engine.frame_done();

                    self.stalled = false;
                },
                // Attach fallback buffer while engine is loading.
                //
                // We don't modify `stalled` here since this content never changes.
                (None, Some(buffer)) => {
                    surface.attach(Some(buffer), 0, 0);
                    let height = (self.size.height - UI_HEIGHT) as i32;
                    surface.damage(0, 0, self.size.width as i32, height);
                },
                // Ignore if buffer is attached but not dirty.
                _ => (),
            }
        }

        // Attach new UI buffers.
        if tabs_ui_visible {
            let ui_rendered = self.tabs_ui.draw(self.tabs.values(), self.active_tab);
            self.stalled &= !ui_rendered;
        } else {
            // Draw UI.
            let ui_rendered = self.ui.draw(self.tabs.len(), self.dirty);
            self.stalled &= !ui_rendered;

            // Commit latest IME state.
            if let Some(text_input) = &mut self.text_input {
                self.ui.commit_ime_state(text_input);
            }
        }

        // Request a new frame if this frame was dirty.
        if !self.stalled {
            surface.frame(&self.wayland_queue, surface.clone());
        }

        // Submit the new frame.
        surface.commit();

        // Clear global force-redraw flag.
        self.dirty = false;
    }

    /// Unstall the renderer.
    ///
    /// This will render a new frame if there currently is no frame request
    /// pending.
    pub fn unstall(&mut self) {
        // Ignore if unstalled or request came from background engine.
        if !self.stalled {
            return;
        }

        // Redraw immediately to unstall rendering.
        self.draw();
        let _ = self.connection.flush();
    }

    /// Explicitly mark window as dirty for a forced redraw.
    ///
    /// This will not unstall the renderer automatically, use `[Self::unstall]`
    /// to do so.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Update surface size.
    pub fn set_size(&mut self, size: Size) {
        // Complete initial configure.
        let was_done = mem::replace(&mut self.initial_configure_done, true);

        // Update window dimensions.
        if self.size == size {
            // Still force redraw for the initial configure.
            if !was_done {
                self.unstall();
            }

            return;
        }
        self.size = size;

        // Resize window's browser engines.
        for engine in self.tabs.values_mut() {
            let engine_size = Size::new(self.size.width, self.size.height - UI_HEIGHT);
            engine.set_size(engine_size);

            // Update browser's viewporter logical render size.
            self.viewport.set_destination(engine_size.width as i32, engine_size.height as i32);
        }

        // Resize UI element surface.
        self.tabs_ui.set_size(Size::new(self.size.width, self.size.height));
        let ui_pos = Position::new(0, (self.size.height - UI_HEIGHT) as i32);
        let ui_size = Size::new(self.size.width, UI_HEIGHT);
        self.ui.set_geometry(ui_pos, ui_size);

        // We skip unstall in browser view to wait for an engine update instead.
        if self.tabs_ui.visible() {
            self.unstall();
        }
    }

    /// Update surface scale.
    pub fn set_scale(&mut self, scale: f64) {
        // Update window scale.
        if self.scale == scale {
            return;
        }
        self.scale = scale;

        // Resize window's browser engines.
        for engine in self.tabs.values_mut() {
            engine.set_scale(scale);
        }

        // Resize UI.
        self.tabs_ui.set_scale(scale);
        self.ui.set_scale(scale);

        // We skip unstall in browser view to wait for an engine update instead.
        if self.tabs_ui.visible() {
            self.unstall();
        }
    }

    /// Handle new key press.
    pub fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Ignore keyboard input in tabs UI.
        if self.tabs_ui.visible() {
            return;
        }

        if self.ui.has_keyboard_focus() {
            // Handle keyboard event in UI.
            self.ui.press_key(raw, keysym, modifiers);

            // Unstall if UI changed.
            if self.ui.dirty() {
                self.unstall();
            }
        } else {
            // Forward keyboard event to browser engine.
            let engine = match self.tabs.get_mut(&self.active_tab) {
                Some(engine) => engine,
                None => return,
            };
            engine.press_key(raw, keysym, modifiers);
        }
    }

    /// Handle key release.
    pub fn release_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Ignore keyboard input in tabs UI.
        if self.tabs_ui.visible() {
            return;
        }

        if self.ui.has_keyboard_focus() {
            // Forward event to UI.
            self.ui.release_key(raw, keysym, modifiers);

            // Unstall if UI changed.
            if self.ui.dirty() {
                self.unstall();
            }
        } else {
            // Forward keyboard event to browser engine.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.release_key(raw, keysym, modifiers);
            }
        }
    }

    /// Handle scroll axis events.
    pub fn pointer_axis(
        &mut self,
        surface: &WlSurface,
        time: u32,
        position: Position<f64>,
        horizontal: AxisScroll,
        vertical: AxisScroll,
        modifiers: Modifiers,
    ) {
        if self.xdg.wl_surface() == surface {
            // Forward event to browser engine.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_axis(time, position, horizontal, vertical, modifiers);
            }
        }
    }

    /// Handle pointer button events.
    pub fn pointer_button(
        &mut self,
        surface: &WlSurface,
        time: u32,
        position: Position<f64>,
        button: u32,
        state: u32,
        modifiers: Modifiers,
    ) {
        // Forward emulated touch event to UI.
        if self.xdg.wl_surface() == surface {
            // Clear UI keyboard focus.
            self.ui.clear_keyboard_focus();

            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_button(time, position, button, state, modifiers);
            }
        } else if self.ui.surface() == surface {
            // Forward emulated touch event to UI.
            match state {
                0 if button == BTN_LEFT => self.ui.touch_up(time, -1, modifiers),
                1 if button == BTN_LEFT => self.ui.touch_down(time, -1, position, modifiers),
                _ => (),
            }
        } else if self.tabs_ui.surface() == surface {
            // Ignore button-up when button-down closed tabs UI.
            if self.tabs_ui.visible() {
                // Clear UI keyboard focus.
                self.ui.clear_keyboard_focus();

                // Forward emulated touch event to UI.
                match state {
                    0 if button == BTN_LEFT => self.tabs_ui.touch_up(time, -1, modifiers),
                    1 if button == BTN_LEFT => {
                        self.tabs_ui.touch_down(time, -1, position, modifiers)
                    },
                    _ => (),
                }
            }
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.tabs_ui.dirty() {
            self.unstall();
        }
    }

    /// Handle pointer motion events.
    pub fn pointer_motion(
        &mut self,
        surface: &WlSurface,
        time: u32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        // Forward events to corresponding surface.
        if self.xdg.wl_surface() == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_motion(time, position, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_motion(time, -1, position, modifiers);
        } else if self.tabs_ui.surface() == surface {
            self.tabs_ui.touch_motion(time, -1, position, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.tabs_ui.dirty() {
            self.unstall();
        }
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        surface: &WlSurface,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        self.touch_points.insert(id, position);

        // Forward events to corresponding surface.
        if self.xdg.wl_surface() == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                // Clear UI keyboard focus.
                self.ui.clear_keyboard_focus();

                engine.touch_down(&self.touch_points, time, id, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_down(time, id, position, modifiers);
        } else if self.tabs_ui.surface() == surface {
            // Clear UI keyboard focus.
            self.ui.clear_keyboard_focus();

            self.tabs_ui.touch_down(time, id, position, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.tabs_ui.dirty() {
            self.unstall();
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, surface: &WlSurface, time: u32, id: i32, modifiers: Modifiers) {
        // Forward events to corresponding surface.
        if self.xdg.wl_surface() == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.touch_up(&self.touch_points, time, id, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_up(time, id, modifiers);
        } else if self.tabs_ui.surface() == surface {
            self.tabs_ui.touch_up(time, id, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.tabs_ui.dirty() {
            self.unstall();
        }

        // Remove touch point from all future events.
        self.touch_points.remove(&id);
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        surface: &WlSurface,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        self.touch_points.insert(id, position);

        // Forward events to corresponding surface.
        if self.xdg.wl_surface() == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.touch_motion(&self.touch_points, time, id, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_motion(time, id, position, modifiers);
        } else if self.tabs_ui.surface() == surface {
            self.tabs_ui.touch_motion(time, id, position, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.tabs_ui.dirty() {
            self.unstall();
        }
    }

    /// Handle IME focus.
    pub fn text_input_enter(&mut self, text_input: ZwpTextInputV3) {
        self.text_input = Some(text_input.into());
    }

    /// Handle IME unfocus.
    pub fn text_input_leave(&mut self) {
        self.text_input = None;
    }

    /// Delete text around the current cursor position.
    pub fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        self.ui.delete_surrounding_text(before_length, after_length);
        self.unstall();
    }

    /// Insert text at the current cursor position.
    pub fn commit_string(&mut self, text: String) {
        self.ui.commit_string(text);
    }

    /// Set preedit text at the current cursor position.
    pub fn preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        self.ui.preedit_string(text, cursor_begin, cursor_end);

        // NOTE: Unstall is always called and it's always the last event in the
        // text-input chain, so we can trigger a redraw here without accidentally
        // drawing a partially updated IME state.
        self.unstall();
    }

    /// Update the URI displayed by the UI.
    pub fn set_display_uri(&mut self, engine_id: EngineId, uri: &str) {
        // Ignore URI change for background engines.
        if engine_id != self.active_tab {
            return;
        }

        // Update the URI.
        self.ui.set_uri(uri);

        // Unstall if UI changed.
        if self.ui.dirty() {
            self.unstall();
        }
    }

    /// Open the tabs UI.
    pub fn show_tabs_ui(&mut self) {
        self.tabs_ui.show();

        // Ensure IME is closed.
        self.ui.clear_keyboard_focus();
        if let Some(text_input) = &mut self.text_input {
            self.ui.commit_ime_state(text_input);
        }
    }

    /// Check whether a surface is owned by this window.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        self.xdg.wl_surface() == surface
            || self.ui.surface() == surface
            || self.tabs_ui.surface() == surface
    }

    /// Get window size.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Get underlying XDG shell window.
    pub fn xdg(&self) -> &XdgWindow {
        &self.xdg
    }
}

/// Unique identifier for one window.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(usize);

impl WindowId {
    pub fn new() -> Self {
        static NEXT_WINDOW_ID: AtomicUsize = AtomicUsize::new(0);
        Self(NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for WindowId {
    fn default() -> Self {
        Self::new()
    }
}

/// Text input with enabled-state tracking.
pub struct TextInput {
    text_input: ZwpTextInputV3,
    enabled: bool,
}

impl From<ZwpTextInputV3> for TextInput {
    fn from(text_input: ZwpTextInputV3) -> Self {
        Self { text_input, enabled: false }
    }
}

impl TextInput {
    /// Check if text input is enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enable text input on a surface.
    ///
    /// This is automatically debounced if the text input is already enabled.
    ///
    /// Does not automatically send a commit, to allow synchronized
    /// initialization of all IME state.
    pub fn enable(&mut self) {
        if self.enabled {
            return;
        }

        self.enabled = true;
        self.text_input.enable();
    }

    /// Disable text input on a surface.
    ///
    /// This is automatically debounced if the text input is already disabled.
    ///
    /// Contrary to `[Self::enable]`, this immediately sends a commit after
    /// disabling IME, since there's no need to synchronize with other
    /// events.
    pub fn disable(&mut self) {
        if !self.enabled {
            return;
        }

        self.enabled = false;
        self.text_input.disable();
        self.text_input.commit();
    }

    /// Set the surrounding text.
    pub fn set_surrounding_text(&self, text: String, cursor_index: i32, selection_anchor: i32) {
        self.text_input.set_surrounding_text(text, cursor_index, selection_anchor);
    }

    /// Indicate the cause of surrounding text change.
    pub fn set_text_change_cause(&self, cause: ChangeCause) {
        self.text_input.set_text_change_cause(cause);
    }

    /// Set text field content purpose and hint.
    pub fn set_content_type(&self, hint: ContentHint, purpose: ContentPurpose) {
        self.text_input.set_content_type(hint, purpose);
    }

    /// Set text field cursor position.
    pub fn set_cursor_rectangle(&self, x: i32, y: i32, width: i32, height: i32) {
        self.text_input.set_cursor_rectangle(x, y, width, height);
    }

    /// Commit IME state.
    pub fn commit(&self) {
        self.text_input.commit();
    }
}
