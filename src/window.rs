//! Browser window handling.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use _text_input::zwp_text_input_v3::{ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3};
use funq::StQueueHandle;
use glutin::display::Display;
use indexmap::IndexMap;
use smallvec::SmallVec;
use smithay_client_toolkit::dmabuf::DmabufFeedback;
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::csd_frame::WindowState;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{AxisScroll, BTN_LEFT};
use smithay_client_toolkit::shell::xdg::window::{
    Window as XdgWindow, WindowConfigure, WindowDecorations,
};
use smithay_client_toolkit::shell::WaylandSurface;

use crate::engine::webkit::{WebKitEngine, WebKitError};
use crate::engine::{Engine, EngineId};
use crate::history::{History, HistoryMatch, SessionRecord, MAX_MATCHES};
use crate::ui::engine_backdrop::EngineBackdrop;
use crate::ui::overlay::option_menu::{
    Borders, OptionMenuId, OptionMenuItem, OptionMenuPosition, ScrollTarget,
};
use crate::ui::overlay::Overlay;
use crate::ui::Ui;
use crate::uri::{SCHEMES, TLDS};
use crate::wayland::protocols::ProtocolStates;
use crate::{Position, Size, State};

/// Search engine base URI.
const SEARCH_URI: &str = "https://duckduckgo.com/?q=";

// Default window size.
const DEFAULT_WIDTH: u32 = 360;
const DEFAULT_HEIGHT: u32 = 640;

#[funq::callbacks(State)]
pub trait WindowHandler {
    /// Close a browser window.
    fn close_window(&mut self, window_id: WindowId);
}

impl WindowHandler for State {
    fn close_window(&mut self, window_id: WindowId) {
        // Remove the window and mark it as closed.
        let mut removed = match self.windows.remove(&window_id) {
            Some(removed) => removed,
            None => return,
        };
        removed.closed = true;

        if self.windows.is_empty() {
            // Quit if all windows were closed.
            self.main_loop.quit();
        } else {
            // Delete session if this wasn't the last window.
            self.history.set_session(removed.id, Vec::new());
        }
    }
}

/// Wayland window.
pub struct Window {
    id: WindowId,

    tabs: IndexMap<EngineId, Box<dyn Engine>>,
    active_tab: EngineId,
    overlay: Overlay,

    dmabuf_feedback: Rc<RefCell<Option<DmabufFeedback>>>,
    wayland_queue: QueueHandle<State>,
    text_input: Option<TextInput>,
    initial_configure_done: bool,
    engine_viewport: WpViewport,
    engine_surface: WlSurface,
    connection: Connection,
    egl_display: Display,
    xdg: XdgWindow,
    scale: f64,
    size: Size,

    queue: StQueueHandle<State>,

    engine_backdrop: EngineBackdrop,

    ui: Ui,
    history_menu_matches: SmallVec<[HistoryMatch; MAX_MATCHES]>,
    history_menu: Option<OptionMenuId>,

    // Touch point position tracking.
    touch_points: HashMap<i32, Position<f64>>,
    keyboard_focus: KeyboardFocus,

    fullscreen_request: Option<EngineId>,
    fullscreened: bool,

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
        history: History,
        dmabuf_feedback: Rc<RefCell<Option<DmabufFeedback>>>,
    ) -> Result<Self, WebKitError> {
        let id = WindowId::new();

        // Create all surfaces and subsurfaces.
        let surface = protocol_states.compositor.create_surface(&wayland_queue);
        let (engine_subsurface, engine_surface) =
            protocol_states.subcompositor.create_subsurface(surface.clone(), &wayland_queue);
        let (overlay_subsurface, overlay_surface) =
            protocol_states.subcompositor.create_subsurface(surface.clone(), &wayland_queue);
        let (ui_subsurface, ui_surface) =
            protocol_states.subcompositor.create_subsurface(surface.clone(), &wayland_queue);

        // Ensure correct surface ordering.
        engine_subsurface.place_above(&surface);
        overlay_subsurface.place_above(&engine_surface);
        overlay_subsurface.place_above(&ui_surface);

        // Create a viewport for each surface.
        let backdrop_viewport = protocol_states.viewporter.viewport(&wayland_queue, &surface);
        let engine_viewport = protocol_states.viewporter.viewport(&wayland_queue, &engine_surface);
        let overlay_viewport =
            protocol_states.viewporter.viewport(&wayland_queue, &overlay_surface);
        let ui_viewport = protocol_states.viewporter.viewport(&wayland_queue, &ui_surface);

        // Create overlay UI renderer.
        let mut overlay = Overlay::new(
            id,
            queue.handle(),
            egl_display.clone(),
            overlay_surface,
            overlay_viewport,
            protocol_states.compositor.clone(),
        );

        // Create UI renderer.
        let mut ui = Ui::new(
            id,
            queue.handle(),
            egl_display.clone(),
            ui_surface,
            ui_subsurface,
            ui_viewport,
            protocol_states.compositor.clone(),
            history,
        );

        // Create engine backdrop.
        let mut engine_backdrop = EngineBackdrop::new(
            egl_display.clone(),
            surface.clone(),
            backdrop_viewport,
            protocol_states,
            wayland_queue.clone(),
        );

        // Enable fractional scaling.
        protocol_states.fractional_scale.fractional_scaling(&wayland_queue, &surface);

        // Create XDG window.
        let decorations = WindowDecorations::RequestServer;
        let xdg = protocol_states.xdg_shell.create_window(surface, decorations, &wayland_queue);
        xdg.set_title("Kumo");
        xdg.set_app_id("Kumo");
        xdg.commit();

        let size = Size::new(DEFAULT_WIDTH, DEFAULT_HEIGHT);
        let active_tab = EngineId::new(id);

        // Resize UI elements to the initial window size.
        engine_backdrop.set_size(size);
        overlay.set_size(size);
        ui.set_size(size);

        let mut window = Self {
            engine_backdrop,
            dmabuf_feedback,
            engine_viewport,
            engine_surface,
            wayland_queue,
            egl_display,
            connection,
            active_tab,
            overlay,
            queue,
            size,
            xdg,
            ui,
            id,
            stalled: true,
            scale: 1.,
            initial_configure_done: Default::default(),
            history_menu_matches: Default::default(),
            fullscreen_request: Default::default(),
            keyboard_focus: Default::default(),
            history_menu: Default::default(),
            touch_points: Default::default(),
            text_input: Default::default(),
            fullscreened: Default::default(),
            closed: Default::default(),
            dirty: Default::default(),
            tabs: Default::default(),
        };

        // Create initial browser tab.
        window.add_tab(true, true)?;

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
    pub fn add_tab(
        &mut self,
        focus_uribar: bool,
        switch_focus: bool,
    ) -> Result<EngineId, WebKitError> {
        // Create a new browser engine.
        let engine_id = EngineId::new(self.id);
        let engine = WebKitEngine::new(
            &self.egl_display,
            self.wayland_queue.clone(),
            self.queue.clone(),
            engine_id,
            self.engine_size(),
            self.scale,
            self.dmabuf_feedback.borrow().as_ref(),
        )?;

        self.tabs.insert(engine_id, Box::new(engine));

        // Switch the active tab.
        if switch_focus {
            self.active_tab = engine_id;
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        if focus_uribar {
            // Focus URI bar to allow text input.
            self.set_keyboard_focus(KeyboardFocus::Ui);
            self.ui.keyboard_focus_uribar();
        }
        if switch_focus {
            self.ui.set_uri("");
        }

        self.unstall();

        Ok(engine_id)
    }

    /// Close a tabs.
    pub fn close_tab(&mut self, history: &History, engine_id: EngineId) {
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

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        // Update browser sessions.
        self.persist_session(history);

        // Force tabs UI redraw.
        self.dirty = true;
        self.unstall();
    }

    /// Get this window's active tab.
    pub fn active_tab(&self) -> EngineId {
        self.active_tab
    }

    /// Switch between tabs.
    pub fn set_active_tab(&mut self, engine_id: EngineId) {
        self.active_tab = engine_id;

        // Update URI bar.
        let uri = self.tabs.get_mut(&self.active_tab).unwrap().uri();
        self.ui.set_uri(&uri);

        // Update tabs popup.
        self.overlay.tabs_mut().set_active_tab(self.active_tab);

        self.unstall();
    }

    /// Load a URI with the active tab.
    pub fn load_uri(&mut self, uri: String, allow_relative_paths: bool) {
        // Perform search if URI is not a recognized URI.
        let uri = match build_uri(uri.trim(), allow_relative_paths) {
            Some(uri) => uri,
            None => Cow::Owned(format!("{SEARCH_URI}{uri}")),
        };

        if let Some(engine) = self.tabs.get(&self.active_tab) {
            engine.load_uri(&uri);
        }

        // Close open option menus.
        self.close_history_menu();

        // Clear URI bar focus.
        self.set_keyboard_focus(KeyboardFocus::None);
    }

    /// Redraw the window.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&mut self) {
        // Ignore rendering before initial configure or after shutdown.
        if self.closed || !self.initial_configure_done {
            return;
        }

        // Notify profiler about frame start.
        #[cfg(feature = "profiling")]
        profiling::finish_frame!();

        // Mark window as stalled if no rendering is performed.
        self.stalled = true;

        let mut text_input_state = TextInputChange::Disabled;
        let overlay_opaque = self.overlay.opaque();

        // Redraw the active browser engine.
        if !overlay_opaque {
            let max_engine_size = Size::<f64>::from(self.engine_size()) * self.scale;
            let engine = self.tabs.get_mut(&self.active_tab).unwrap();

            match engine.wl_buffer() {
                // Render the engine's buffer.
                Some(engine_buffer) => {
                    let buffer_size: Size<f64> = engine.buffer_size().into();

                    // Update browser's viewporter render size.
                    let src_width = buffer_size.width.min(max_engine_size.width);
                    let src_height = buffer_size.height.min(max_engine_size.height);
                    self.engine_viewport.set_source(0., 0., src_width, src_height);
                    let dst_width = (src_width / self.scale).round() as i32;
                    let dst_height = (src_height / self.scale).round() as i32;
                    self.engine_viewport.set_destination(dst_width, dst_height);

                    // Render buffer if it requires a redraw.
                    if engine.dirty() || self.dirty {
                        // Update opaque region.
                        self.engine_surface.set_opaque_region(engine.opaque_region());

                        // Attach engine buffer to primary surface.
                        self.engine_surface.attach(Some(engine_buffer), 0, 0);
                        match engine.take_buffer_damage() {
                            Some(damage_rects) => {
                                for (x, y, width, height) in damage_rects {
                                    self.engine_surface.damage_buffer(x, y, width, height);
                                }
                            },
                            None => self.engine_surface.damage(0, 0, dst_width, dst_height),
                        }
                        self.engine_surface.commit();

                        // Request new engine frame.
                        engine.frame_done();

                        self.stalled = false;
                    }
                },
                // Clear attached surface if we've switched to an engine that
                // doesn't have a buffer yet.
                None => {
                    self.engine_surface.attach(None, 0, 0);
                    self.engine_surface.commit();
                },
            }

            // Get engine's IME text_input state.
            if self.text_input.is_some() && self.keyboard_focus == KeyboardFocus::Browser {
                text_input_state = engine.text_input_state();
            }
        }

        // Draw engine backdrop.
        let backdrop_rendered = self.engine_backdrop.draw();
        self.stalled &= !backdrop_rendered;

        // Draw UI.
        if !overlay_opaque && !self.fullscreened {
            let ui_rendered = self.ui.draw(self.tabs.len(), self.dirty);
            self.stalled &= !ui_rendered;
        }

        // Get UI's IME text_input state.
        if self.text_input.is_some() && self.keyboard_focus == KeyboardFocus::Ui {
            text_input_state = self.ui.text_input_state();
        }

        // Draw overlay surface.
        let overlay_rendered = self.overlay.draw();
        self.stalled &= !overlay_rendered;

        // Update IME text_input state.
        match (text_input_state, &mut self.text_input) {
            (TextInputChange::Dirty(text_input_state), Some(text_input)) => {
                text_input_state.commit(text_input);
            },
            (TextInputChange::Disabled, Some(text_input)) => text_input.disable(),
            _ => (),
        }

        // Request a new frame if this frame was dirty.
        let surface = self.xdg.wl_surface();
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

    /// Handle Wayland configure events.
    pub fn configure(&mut self, configure: WindowConfigure) {
        // Get new configured size.
        let width = configure.new_size.0.map(|w| w.get()).unwrap_or(self.size.width);
        let height = configure.new_size.1.map(|h| h.get()).unwrap_or(self.size.height);
        let size = Size { width, height };

        // Get fullscreen state.
        let is_fullscreen = configure.state.contains(WindowState::FULLSCREEN);

        // Complete initial configure.
        let was_done = mem::replace(&mut self.initial_configure_done, true);

        // Short-circuit if nothing changed.
        let size_unchanged = self.size == size;
        if size_unchanged && self.fullscreened == is_fullscreen {
            // Still force redraw for the initial configure.
            if !was_done {
                self.unstall();
            }

            return;
        }
        self.fullscreened = is_fullscreen;
        self.size = size;

        // Resize window's browser engines.
        let engine_size = self.engine_size();
        for engine in self.tabs.values_mut() {
            engine.set_size(engine_size);
        }

        // Resize UI element surface.
        if !size_unchanged {
            self.engine_backdrop.set_size(self.size);
            self.overlay.set_size(self.size);
            self.ui.set_size(self.size);
        }

        // Acknowledge pending engine fullscreen requests.
        if Some(self.active_tab) == self.fullscreen_request.take() {
            let active_tab = self.tabs.get_mut(&self.active_tab).unwrap();
            active_tab.set_fullscreen(self.fullscreened);
        }

        // Destroy history popup, so we don't need to resize it.
        if let Some(id) = self.history_menu {
            self.overlay.close_option_menu(id);
        }

        self.unstall();
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
        self.engine_backdrop.set_scale(scale);
        self.overlay.set_scale(scale);
        self.ui.set_scale(scale);

        self.unstall();
    }

    /// Handle new key press.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn press_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        match self.keyboard_focus {
            KeyboardFocus::Ui => self.ui.press_key(raw, keysym, modifiers),
            KeyboardFocus::Browser => {
                let engine = match self.tabs.get_mut(&self.active_tab) {
                    Some(engine) => engine,
                    None => return,
                };
                engine.press_key(time, raw, keysym, modifiers);
            },
            KeyboardFocus::None => (),
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
            self.unstall();
        }
    }

    /// Handle key release.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn release_key(&mut self, time: u32, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        match self.keyboard_focus {
            KeyboardFocus::Browser => {
                if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                    engine.release_key(time, raw, keysym, modifiers);
                }
            },
            // Ui has no release handling need (yet).
            KeyboardFocus::Ui | KeyboardFocus::None => (),
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
            self.unstall();
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
        if &self.engine_surface == surface {
            // Forward event to browser engine.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                // Ensure popups are closed when scrolling.
                engine.close_option_menu(None);

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
        down: bool,
        modifiers: Modifiers,
    ) {
        if &self.engine_surface == surface {
            self.update_keyboard_focus_surface(surface);

            // Use real pointer events for the browser engine.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_button(time, position, button, down, modifiers);
            }
        } else {
            // Emulate touch for non-engine purposes.
            if button == BTN_LEFT {
                if down {
                    self.touch_down(surface, time, -1, position, modifiers);
                } else {
                    self.touch_up(surface, time, -1, modifiers);
                }
            }
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
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
        if &self.engine_surface == surface {
            // Use real pointer events for the browser engine.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_motion(time, position, modifiers);
            }
        } else {
            // Emulate touch for non-engine purposes.
            self.touch_motion(surface, time, -1, position, modifiers);
        }
    }

    /// Handle pointer enter events.
    pub fn pointer_enter(
        &mut self,
        surface: &WlSurface,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        if &self.engine_surface == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_enter(position, modifiers);
            }
        }
    }

    /// Handle pointer leave events.
    pub fn pointer_leave(
        &mut self,
        surface: &WlSurface,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        if &self.engine_surface == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.pointer_leave(position, modifiers);
            }
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

        // Update the surface receiving keyboard focus.
        self.update_keyboard_focus_surface(surface);

        // Forward events to corresponding surface.
        if &self.engine_surface == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                // Close all dropdowns when interacting with the page.
                engine.close_option_menu(None);

                engine.touch_down(time, id, position, modifiers);
            }
        } else if self.ui.surface() == surface {
            // Close all dropdowns when clicking on the UI.
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.close_option_menu(None);
            }

            self.ui.touch_down(time, id, position, modifiers);
        } else if self.overlay.surface() == surface {
            self.overlay.touch_down(time, id, position, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
            self.unstall();
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, surface: &WlSurface, time: u32, id: i32, modifiers: Modifiers) {
        // Forward events to corresponding surface.
        if &self.engine_surface == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                if let Some(position) = self.touch_points.get(&id) {
                    engine.touch_up(time, id, *position, modifiers);
                }
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_up(time, id, modifiers);
        } else if self.overlay.surface() == surface {
            self.overlay.touch_up(time, id, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
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
        if &self.engine_surface == surface {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.touch_motion(time, id, position, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.ui.touch_motion(time, id, position, modifiers);
        } else if self.overlay.surface() == surface {
            self.overlay.touch_motion(time, id, position, modifiers);
        }

        // Unstall if UI changed.
        if self.ui.dirty() || self.overlay.dirty() {
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
        match self.keyboard_focus {
            KeyboardFocus::Ui => self.ui.delete_surrounding_text(before_length, after_length),
            KeyboardFocus::Browser => {
                let engine = match self.tabs.get_mut(&self.active_tab) {
                    Some(engine) => engine,
                    None => return,
                };
                engine.delete_surrounding_text(before_length, after_length);
            },
            KeyboardFocus::None => (),
        }
    }

    /// Insert text at the current cursor position.
    pub fn commit_string(&mut self, text: String) {
        match self.keyboard_focus {
            KeyboardFocus::Ui => self.ui.commit_string(text),
            KeyboardFocus::Browser => {
                let engine = match self.tabs.get_mut(&self.active_tab) {
                    Some(engine) => engine,
                    None => return,
                };
                engine.commit_string(text);
            },
            KeyboardFocus::None => (),
        }
    }

    /// Set preedit text at the current cursor position.
    pub fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        match self.keyboard_focus {
            KeyboardFocus::Ui => self.ui.set_preedit_string(text, cursor_begin, cursor_end),
            KeyboardFocus::Browser => {
                let engine = match self.tabs.get_mut(&self.active_tab) {
                    Some(engine) => engine,
                    None => return,
                };
                engine.set_preedit_string(text, cursor_begin, cursor_end);
            },
            KeyboardFocus::None => (),
        }

        // NOTE: `preedit_string` is always called and it's always the last event in the
        // text-input chain, so we can trigger a redraw here without accidentally
        // drawing a partially updated IME state.
        if self.keyboard_focus != KeyboardFocus::Browser {
            self.unstall();
        }
    }

    /// Update an engine's URI.
    pub fn set_engine_uri(&mut self, history: &History, engine_id: EngineId, uri: String) {
        // Update UI if the URI change is for the active tab.
        if engine_id == self.active_tab {
            self.ui.set_uri(&uri);

            // Unstall if UI changed.
            if self.ui.dirty() {
                self.unstall();
            }
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        // Increment URI visit count for history.
        history.visit(uri);

        // Update browser session.
        self.persist_session(history);
    }

    /// Update the window's browser session.
    ///
    /// This is used to recover the browser session when restarting Kumo.
    pub fn persist_session(&self, history: &History) {
        let session: Vec<_> = self.tabs.values().map(SessionRecord::from).collect();
        history.set_session(self.id, session);
    }

    /// Update an engine's title.
    pub fn set_engine_title(&mut self, history: &History, engine_id: EngineId, title: String) {
        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        // Update title of current URI for history.
        if let Some(engine) = self.tabs.get(&engine_id) {
            let uri = engine.uri();
            history.set_title(&uri, title);
        }
    }

    /// Open the tabs UI.
    pub fn show_tabs_ui(&mut self) {
        self.overlay.tabs_mut().set_visible(true);
        self.set_keyboard_focus(KeyboardFocus::None);
    }

    /// Create a new dropdown popup.
    pub fn open_option_menu<I>(
        &mut self,
        menu_id: OptionMenuId,
        position: impl Into<OptionMenuPosition>,
        item_width: Option<u32>,
        items: I,
    ) where
        I: Iterator<Item = OptionMenuItem>,
    {
        self.overlay.open_option_menu(menu_id, position, item_width, self.scale, items);
        self.unstall();
    }

    /// Remove a dropdown popup.
    pub fn close_option_menu(&mut self, menu_id: OptionMenuId) {
        self.overlay.close_option_menu(menu_id);
        self.unstall();
    }

    /// Handle submission for option menu spawned by the window.
    pub fn submit_option_menu(&mut self, menu_id: OptionMenuId, index: usize) {
        // Ignore unknown menu IDs.
        if Some(menu_id) != self.history_menu {
            return;
        }

        // Load the selected URI.
        let uri = self.history_menu_matches.swap_remove(index).uri;
        self.ui.set_uri(&uri);
        self.load_uri(uri, false);
    }

    /// Show history options menu.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn open_history_menu(&mut self, matches: SmallVec<[HistoryMatch; MAX_MATCHES]>) {
        // Skip new menu creation without matches.
        if matches.is_empty() {
            // Close old menu.
            self.close_history_menu();

            return;
        }

        // Convert matches to option menu entries.
        self.history_menu_matches = matches;
        let items = self.history_menu_matches.iter().map(|m| {
            let (label, description) = if m.title.is_empty() {
                (m.uri.clone(), m.uri.clone())
            } else {
                (m.title.clone(), m.uri.clone())
            };
            OptionMenuItem { label, description, disabled: false, selected: false }
        });

        match self.history_menu.and_then(|id| self.overlay.option_menu(id)) {
            // Update contents of existing menu.
            Some(menu) => {
                menu.set_visible(true);
                menu.set_items(items);
                menu.scroll(ScrollTarget::End);
            },
            // Open new menu.
            None => {
                let menu_id = OptionMenuId::new(self.id);
                let menu = self.overlay.open_option_menu(
                    menu_id,
                    Position::new(0, self.size.height as i32),
                    self.size.width,
                    self.scale,
                    items,
                );
                menu.set_borders(Borders::TOP);
                menu.scroll(ScrollTarget::End);
                self.history_menu = Some(menu_id);
            },
        }
    }

    /// Hide history options menu.
    pub fn close_history_menu(&mut self) {
        if let Some(menu) = self.history_menu.and_then(|id| self.overlay.option_menu(id)) {
            menu.set_visible(false);
        }
    }

    /// Handle engine fullscreen requests.
    pub fn request_fullscreen(&mut self, engine_id: EngineId, enable: bool) {
        // Ignore fullscreen requests for background engines.
        if engine_id != self.active_tab {
            return;
        }

        // Request fullscreen mode from compositor.
        if enable {
            self.xdg().set_fullscreen(None);
        } else {
            self.xdg().unset_fullscreen();
        }

        // Store engine's fullscreen request.
        self.fullscreen_request = Some(engine_id);
    }

    /// Check whether a surface is owned by this window.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        &self.engine_surface == surface
            || self.xdg.wl_surface() == surface
            || self.ui.surface() == surface
            || self.overlay.surface() == surface
    }

    /// Get underlying XDG shell window.
    pub fn xdg(&self) -> &XdgWindow {
        &self.xdg
    }

    /// Size allocated to the browser engine's buffer.
    pub fn engine_size(&self) -> Size {
        if self.fullscreened {
            Size::new(self.size.width, self.size.height)
        } else {
            Size::new(self.size.width, self.size.height - Ui::toolbar_height())
        }
    }

    /// Handle keyboard focus surface changes.
    pub fn update_keyboard_focus_surface(&mut self, surface: &WlSurface) {
        // Assign keyboard focus to the element owning the surface.
        //
        // The overlay does not have any text input elements and thus does not take
        // keyboard focus.
        if self.ui.surface() == surface {
            self.set_keyboard_focus(KeyboardFocus::Ui);
        } else if &self.engine_surface == surface {
            self.set_keyboard_focus(KeyboardFocus::Browser);
        }
    }

    /// Update the keyboard focus.
    pub fn set_keyboard_focus(&mut self, focus: KeyboardFocus) {
        self.keyboard_focus = focus;

        // Clear UI focus.
        if focus != KeyboardFocus::Ui {
            self.ui.clear_keyboard_focus();
        }

        // Clear engine focus.
        if focus != KeyboardFocus::Browser {
            if let Some(engine) = self.tabs.get_mut(&self.active_tab) {
                engine.clear_focus();
            }
        }
    }

    /// Handle DMA buffer release.
    pub fn buffer_released(&mut self, buffer: &WlBuffer) {
        for (_, engine) in &mut self.tabs {
            engine.buffer_released(buffer);
        }
    }

    /// Notify window about DMA buffer feedback change.
    pub fn dmabuf_feedback_changed(&mut self) {
        if let Some(feedback) = &*self.dmabuf_feedback.borrow() {
            for (_, engine) in &mut self.tabs {
                engine.dmabuf_feedback(feedback);
            }
        }
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

    /// Get the raw window ID value.
    pub fn as_raw(&self) -> usize {
        self.0
    }
}

impl Default for WindowId {
    fn default() -> Self {
        Self::new()
    }
}

/// Keyboard focus surfaces.
#[derive(PartialEq, Eq, Copy, Clone, Default)]
pub enum KeyboardFocus {
    None,
    #[default]
    Ui,
    Browser,
}

/// Text input with enabled-state tracking.
#[derive(Debug)]
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

/// IME text_input state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextInputState {
    pub cursor_index: i32,
    pub cursor_rect: (i32, i32, i32, i32),
    pub selection: Option<Range<i32>>,
    pub surrounding_text: String,
    pub change_cause: ChangeCause,
    pub purpose: ContentPurpose,
    pub hint: ContentHint,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            change_cause: ChangeCause::Other,
            purpose: ContentPurpose::Normal,
            hint: ContentHint::None,
            surrounding_text: Default::default(),
            cursor_index: Default::default(),
            cursor_rect: Default::default(),
            selection: Default::default(),
        }
    }
}

impl TextInputState {
    fn commit(self, text_input: &mut TextInput) {
        // Enable IME if necessary.
        text_input.enable();

        // Offer the entire input text as surrounding text hint.
        //
        // NOTE: This request is technically limited to 4000 bytes, but that is unlikely
        // to be an issue for our purposes.
        let (cursor_index, selection_anchor) = match &self.selection {
            Some(selection) => (selection.end, selection.start),
            None => (self.cursor_index, self.cursor_index),
        };
        text_input.set_surrounding_text(self.surrounding_text, cursor_index, selection_anchor);

        // Set reason for this update.
        text_input.set_text_change_cause(self.change_cause);

        // Set text input field type.
        text_input.set_content_type(self.hint, self.purpose);

        // Set cursor position.
        let (cursor_x, cursor_y, cursor_width, cursor_height) = self.cursor_rect;
        text_input.set_cursor_rectangle(cursor_x, cursor_y, cursor_width, cursor_height);

        text_input.commit();
    }
}

/// Text input state change.
#[derive(Debug)]
pub enum TextInputChange {
    /// Text input is disabled.
    Disabled,
    /// Text input is unchanged.
    Unchanged,
    /// Text input requires update.
    Dirty(TextInputState),
}

#[allow(rustdoc::bare_urls)]
/// Extract HTTP URI from uri bar input.
///
/// # Examples
///
/// | input                         | output                                      |
/// | ----------------------------- | ------------------------------------------- |
/// | `"https://example.org"`       | `Some("https://example.org")`               |
/// | `"example.org"`               | `Some("https://example.org")`               |
/// | `"example.org/space in path"` | `Some("https://example.org/space in path")` |
/// | `"/home"`                     | `Some("file:///home")`                      |
/// | `"example org"`               | `None`                                      |
/// | `"ftp://example.org"`         | `None`                                      |
fn build_uri(mut input: &str, allow_relative_paths: bool) -> Option<Cow<'_, str>> {
    let uri = Cow::Borrowed(input);

    // If input starts with `/`, we assume it's a path.
    if uri.starts_with('/') {
        return Some(Cow::Owned(format!("file://{uri}")));
    }

    // Allow relative paths at startup by checking against the filesystem.
    if allow_relative_paths {
        let path = Path::new(&*uri);
        if path.exists() {
            if let Some(absolute) = path.canonicalize().ok().as_deref().and_then(Path::to_str) {
                return Some(Cow::Owned(format!("file://{absolute}")));
            }
        }
    }

    // Parse scheme, short-circuiting if an unknown scheme was found.
    let mut has_scheme = false;
    let mut has_port = false;
    if let Some(index) = input.find(|c: char| !c.is_alphabetic()) {
        if input[index..].starts_with(':') {
            has_scheme = SCHEMES.contains(&&input[..index]);
            if has_scheme {
                // Allow arbitrary number of slashes after the scheme.
                input = input[index + 1..].trim_start_matches('/');
            } else {
                // Check if we're dealing with a local address + port, instead of scheme.
                // Example: "localhost:80/index"
                has_port = index + 1 < input.len()
                    && &input[index + 1..index + 2] != "/"
                    && input[index + 1..].chars().take_while(|c| *c != '/').all(|c| c.is_numeric());

                if has_port {
                    input = &input[..index];
                } else {
                    return None;
                }
            }
        }
    }

    if !has_port {
        // Allow all characters after a slash.
        if let Some(index) = input.find('/') {
            input = &input[..index];
        }

        // Parse port.
        if let Some(index) = input.rfind(':') {
            has_port =
                index + 1 < input.len() && input[index + 1..].chars().all(|c| c.is_numeric());
            if has_port {
                input = &input[..index];
            }
        }
    }

    // Abort if the domain contains any illegal characters.
    if input.starts_with('[') && input.ends_with(']') {
        // Handle IPv6 validation.
        if input[1..input.len() - 1]
            .find(|c| !matches!(c, '0'..='9' | 'a'..='f' | 'A'..='F' | ':'))
            .is_some()
        {
            return None;
        }
    } else if input.find(|c: char| !c.is_alphanumeric() && c != '-' && c != '.').is_some() {
        return None;
    }

    // Skip TLD check if scheme was explicitly specified.
    if has_scheme {
        return Some(uri);
    }

    // Skip TLD check for IPv4/6.
    let ipv4_segments =
        input.split('.').take_while(|s| s.chars().all(|c| c.is_ascii_digit())).take(5).count();
    let is_ip = input.starts_with('[') || ipv4_segments == 4;
    if is_ip {
        return Some(Cow::Owned(format!("https://{uri}")));
    }

    // Check for valid TLD.
    match input.rfind('.') {
        // Accept missing TLD with explicitly specified ports.
        None if has_port => Some(Cow::Owned(format!("https://{uri}"))),
        Some(tld_index) if TLDS.contains(&input[tld_index + 1..].to_uppercase().as_str()) => {
            Some(Cow::Owned(format!("https://{uri}")))
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn extract_uri() {
        assert_eq!(build_uri("https://example.org", false).as_deref(), Some("https://example.org"));
        assert_eq!(build_uri("example.org", false).as_deref(), Some("https://example.org"));
        assert_eq!(
            build_uri("x.org/space path", false).as_deref(),
            Some("https://x.org/space path")
        );
        assert_eq!(build_uri("/home/user", false).as_deref(), Some("file:///home/user"));
        assert_eq!(build_uri("https://x.org:666", false).as_deref(), Some("https://x.org:666"));
        assert_eq!(build_uri("example.org:666", false).as_deref(), Some("https://example.org:666"));
        assert_eq!(build_uri("https://example:666", false).as_deref(), Some("https://example:666"));
        assert_eq!(build_uri("example:666", false).as_deref(), Some("https://example:666"));
        assert_eq!(build_uri("example:666/x", false).as_deref(), Some("https://example:666/x"));
        assert_eq!(
            build_uri("https://exa-mple.org", false).as_deref(),
            Some("https://exa-mple.org")
        );
        assert_eq!(build_uri("exa-mple.org", false).as_deref(), Some("https://exa-mple.org"));
        assert_eq!(build_uri("https:123", false).as_deref(), Some("https:123"));
        assert_eq!(build_uri("https:123:456", false).as_deref(), Some("https:123:456"));
        assert_eq!(build_uri("/test:123", false).as_deref(), Some("file:///test:123"));
        assert_eq!(
            build_uri("data:text/HTML,<input>", false).as_deref(),
            Some("data:text/HTML,<input>")
        );

        assert_eq!(build_uri("example org", false).as_deref(), None);
        assert_eq!(build_uri("ftp://example.org", false).as_deref(), None);
        assert_eq!(build_uri("space in scheme:example.org", false).as_deref(), None);
        assert_eq!(build_uri("example.invalidtld", false).as_deref(), None);
        assert_eq!(build_uri("example.org:/", false).as_deref(), None);
        assert_eq!(build_uri("example:/", false).as_deref(), None);
        assert_eq!(build_uri("xxx:123:456", false).as_deref(), None);

        assert_eq!(
            build_uri("http://[fe80::a]/index", false).as_deref(),
            Some("http://[fe80::a]/index")
        );
        assert_eq!(
            build_uri("http://127.0.0.1:80/x", false).as_deref(),
            Some("http://127.0.0.1:80/x")
        );
        assert_eq!(build_uri("[fe80::a]:80", false).as_deref(), Some("https://[fe80::a]:80"));
        assert_eq!(build_uri("127.0.0.1:80", false).as_deref(), Some("https://127.0.0.1:80"));
        assert_eq!(build_uri("[fe80::a]", false).as_deref(), Some("https://[fe80::a]"));
        assert_eq!(build_uri("127.0.0.1", false).as_deref(), Some("https://127.0.0.1"));

        let cwd = env::current_dir().unwrap().to_string_lossy().into_owned();
        let expected = format!("file://{cwd}/src/main.rs");
        assert_eq!(build_uri("./src/main.rs", true).as_deref(), Some(expected.as_str()));
        assert_eq!(build_uri("src/main.rs", true).as_deref(), Some(expected.as_str()));
    }
}
