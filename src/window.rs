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
use funq::{MtQueueHandle, StQueueHandle};
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

use crate::engine::{Engine, EngineId, Group, GroupId, NO_GROUP_ID, NO_GROUP_REF};
use crate::storage::groups::Groups;
use crate::storage::history::{History, HistoryMatch, MAX_MATCHES};
use crate::storage::session::{Session, SessionRecord};
use crate::storage::Storage;
use crate::ui::engine_backdrop::EngineBackdrop;
use crate::ui::overlay::option_menu::{
    Borders, OptionMenuId, OptionMenuItem, OptionMenuPosition, ScrollTarget,
};
use crate::ui::overlay::Overlay;
use crate::ui::Ui;
use crate::uri::{SCHEMES, TLDS};
use crate::wayland::protocols::ProtocolStates;
use crate::{Position, Size, State, WebKitState};

/// Search engine base URI.
const SEARCH_URI: &str = "https://duckduckgo.com/?q=";

// Default window size.
const DEFAULT_WIDTH: u32 = 360;
const DEFAULT_HEIGHT: u32 = 640;

#[funq::callbacks(State)]
pub trait WindowHandler {
    /// Close a browser window.
    fn close_window(&mut self, window_id: WindowId);

    /// Write text to the system clipboard.
    fn set_clipboard(&mut self, text: String);

    /// Request clipboard pasting.
    fn request_paste(&mut self, target: PasteTarget);

    /// Paste text into the window.
    fn paste(&mut self, target: PasteTarget, text: String);
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
            self.storage.session.persist(removed.id, []);

            // Cleanup unused groups.
            self.storage.groups.delete_orphans();
        }
    }

    fn set_clipboard(&mut self, text: String) {
        self.set_clipboard(text);
    }

    fn request_paste(&mut self, target: PasteTarget) {
        self.request_paste(target);
    }

    fn paste(&mut self, target: PasteTarget, text: String) {
        let window = match self.windows.get_mut(&target.window_id()) {
            Some(window) => window,
            None => return,
        };

        match (window.keyboard_focus, target) {
            (KeyboardFocus::Overlay, PasteTarget::Ui(_)) => {
                window.overlay.paste(text);
                window.unstall();
            },
            (KeyboardFocus::Ui, PasteTarget::Ui(_)) => {
                window.ui.paste(text);
                window.unstall();
            },
            (KeyboardFocus::Browser, PasteTarget::Browser(engine_id))
                if window.active_tab == Some(engine_id) =>
            {
                if let Some(engine) = window.active_tab_mut() {
                    engine.paste(text);
                }
            },
            // Ignore paste requests if input focus has changed.
            _ => (),
        }
    }
}

/// Wayland window.
pub struct Window {
    id: WindowId,

    tabs: IndexMap<EngineId, Box<dyn Engine>>,
    groups: IndexMap<GroupId, Group>,
    engine_state: Rc<RefCell<WebKitState>>,
    active_tab: Option<EngineId>,

    wayland_queue: QueueHandle<State>,
    text_input: Option<TextInput>,
    initial_configure_done: bool,
    engine_viewport: WpViewport,
    engine_surface: WlSurface,
    connection: Connection,
    xdg: XdgWindow,
    scale: f64,
    size: Size,

    ui: Ui,
    overlay: Overlay,
    engine_backdrop: EngineBackdrop,

    history_menu_matches: SmallVec<[HistoryMatch; MAX_MATCHES]>,
    history_menu: Option<OptionMenuId>,
    history: History,

    // Touch point position tracking.
    touch_points: HashMap<i32, Position<f64>>,
    keyboard_focus: KeyboardFocus,

    fullscreen_request: Option<EngineId>,
    fullscreened: bool,

    session_storage: Session,
    group_storage: Groups,

    text_menu: Option<(OptionMenuId, Option<String>)>,
    queue: MtQueueHandle<State>,

    last_rendered_engine: Option<EngineId>,
    stalled: bool,
    closed: bool,
}

impl Window {
    pub fn new(
        protocol_states: &ProtocolStates,
        connection: Connection,
        display: Display,
        queue: StQueueHandle<State>,
        wayland_queue: QueueHandle<State>,
        storage: &Storage,
        engine_state: Rc<RefCell<WebKitState>>,
    ) -> Self {
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
            display.clone(),
            overlay_surface,
            overlay_viewport,
            protocol_states.compositor.clone(),
            storage.history.clone(),
        );

        // Create UI renderer.
        let mut ui = Ui::new(
            id,
            queue.handle(),
            display.clone(),
            ui_surface,
            ui_subsurface,
            ui_viewport,
            protocol_states.compositor.clone(),
            storage.history.clone(),
        );

        // Create engine backdrop.
        let mut engine_backdrop = EngineBackdrop::new(
            display,
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

        // Resize UI elements to the initial window size.
        let size = Size::new(DEFAULT_WIDTH, DEFAULT_HEIGHT);
        engine_backdrop.set_size(size);
        overlay.set_size(size);
        ui.set_size(size);

        let mut window = Self {
            engine_backdrop,
            engine_viewport,
            engine_surface,
            wayland_queue,
            engine_state,
            connection,
            overlay,
            size,
            xdg,
            ui,
            id,
            active_tab: Some(EngineId::new(id, NO_GROUP_ID)),
            session_storage: storage.session.clone(),
            group_storage: storage.groups.clone(),
            history: storage.history.clone(),
            queue: queue.handle(),
            stalled: true,
            scale: 1.,
            initial_configure_done: Default::default(),
            history_menu_matches: Default::default(),
            last_rendered_engine: Default::default(),
            fullscreen_request: Default::default(),
            keyboard_focus: Default::default(),
            fullscreened: Default::default(),
            history_menu: Default::default(),
            touch_points: Default::default(),
            text_input: Default::default(),
            text_menu: Default::default(),
            closed: Default::default(),
            groups: Default::default(),
            tabs: Default::default(),
        };

        // Create initial browser tab.
        window.add_tab(true, true, NO_GROUP_ID);

        window
    }

    /// Get the ID of this window.
    pub fn id(&self) -> WindowId {
        self.id
    }

    /// Get a reference to a tab using its ID.
    #[allow(clippy::borrowed_box)]
    pub fn tab(&self, engine_id: EngineId) -> Option<&Box<dyn Engine>> {
        self.tabs.get(&engine_id)
    }

    /// Get a mutable reference to a tab using its ID.
    pub fn tab_mut(&mut self, engine_id: EngineId) -> Option<&mut Box<dyn Engine>> {
        self.tabs.get_mut(&engine_id)
    }

    /// Add a tab to the window.
    pub fn add_tab(
        &mut self,
        focus_uribar: bool,
        switch_focus: bool,
        group_id: GroupId,
    ) -> EngineId {
        // Get the tab group for the new engine.
        let group = self.groups.get(&group_id).unwrap_or(NO_GROUP_REF);

        // Create a new browser engine.
        let engine_id = EngineId::new(self.id, group.id());
        let engine = Box::new(self.engine_state.borrow_mut().create_engine(
            group,
            engine_id,
            self.engine_size(),
            self.scale,
        ));

        self.tabs.insert(engine_id, engine);

        // Switch the active tab.
        if switch_focus {
            self.active_tab = Some(engine_id);
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        if focus_uribar {
            // Focus URI bar to allow text input.
            self.set_keyboard_focus(KeyboardFocus::Ui);
            self.ui.keyboard_focus_uribar();
        }

        if switch_focus {
            self.ui.set_load_progress(1.);
            self.ui.set_uri("");
        }

        self.unstall();

        engine_id
    }

    /// Close a tab.
    pub fn close_tab(&mut self, engine_id: EngineId) {
        // Remove engine and get the position it was in.
        let (index, group_id) = match self.tabs.shift_remove_full(&engine_id) {
            Some((index, engine_id, _)) => (index, engine_id.group_id()),
            None => return,
        };

        if Some(engine_id) == self.active_tab {
            // First search for previous and following tabs with matching tab group,
            // otherwise fall back to the first tab.
            let len = self.tabs.len();
            let mut prev_tabs = self.tabs.iter().rev().skip(len - index);
            let new_focus = prev_tabs
                .find(|(engine_id, _)| engine_id.group_id() == group_id)
                .or_else(|| {
                    let mut next_tabs = self.tabs.iter().skip(index);
                    next_tabs.find(|(engine_id, _)| engine_id.group_id() == group_id)
                })
                .or_else(|| self.tabs.first());

            self.set_active_tab(new_focus.map(|(engine_id, _)| *engine_id));
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        // Update browser sessions.
        self.persist_session();

        // Force tabs UI redraw.
        self.unstall();
    }

    /// Get a reference to this window's active tab.
    #[allow(clippy::borrowed_box)]
    pub fn active_tab(&self) -> Option<&Box<dyn Engine>> {
        self.tab(self.active_tab?)
    }

    /// Get a mutable reference to this window's active tab.
    pub fn active_tab_mut(&mut self) -> Option<&mut Box<dyn Engine>> {
        self.tab_mut(self.active_tab?)
    }

    /// Switch between tabs.
    pub fn set_active_tab(&mut self, engine_id: impl Into<Option<EngineId>>) {
        self.active_tab = engine_id.into();

        // Update URI and load progress.
        if let Some(engine) = self.active_tab.and_then(|id| self.tabs.get(&id)) {
            self.ui.set_uri(&engine.uri());
            self.ui.set_load_progress(1.);
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_active_tab(self.active_tab);

        // Update session's focused tab.
        self.persist_session();

        self.unstall();
    }

    /// Load a URI with the active tab.
    pub fn load_uri(&mut self, uri: String, allow_relative_paths: bool) {
        // Perform search if URI is not a recognized URI.
        let uri = match build_uri(uri.trim(), allow_relative_paths) {
            Some(uri) => uri,
            None => Cow::Owned(format!("{SEARCH_URI}{uri}")),
        };

        if let Some(engine) = self.active_tab_mut() {
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
            self.draw_engine(&mut text_input_state);
        }

        // Draw engine backdrop.
        let backdrop_rendered = self.engine_backdrop.draw();
        self.stalled &= !backdrop_rendered;

        // Draw UI.
        if !overlay_opaque && !self.fullscreened {
            let has_history = self
                .active_tab
                .and_then(|id| self.tabs.get(&id))
                .is_some_and(|engine| engine.has_prev());
            let tab_group = self.overlay.tabs_mut().active_tab_group();
            let tab_count = self.tabs.values().filter(|t| t.id().group_id() == tab_group).count();
            let ui_rendered = self.ui.draw(tab_count, has_history);
            self.stalled &= !ui_rendered;
        }

        // Get UI's IME text_input state.
        if self.text_input.is_some() {
            match self.keyboard_focus {
                KeyboardFocus::Ui => text_input_state = self.ui.text_input_state(),
                KeyboardFocus::Overlay => text_input_state = self.overlay.text_input_state(),
                _ => (),
            }
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
    }

    /// Redraw the active tab's engine.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw_engine(&mut self, text_input_state: &mut TextInputChange) {
        let max_engine_size = Size::<f64>::from(self.engine_size()) * self.scale;
        let engine = match self.active_tab.and_then(|id| self.tabs.get_mut(&id)) {
            Some(engine) => engine,
            None => return,
        };

        // Get engine's IME text_input state.
        if self.text_input.is_some() && self.keyboard_focus == KeyboardFocus::Browser {
            *text_input_state = engine.text_input_state();
        }

        // Avoid rendering engine without any changes.
        if !engine.dirty() && self.last_rendered_engine == Some(engine.id()) {
            return;
        }

        // Attach the engine's buffer.
        if !engine.attach_buffer(&self.engine_surface) {
            self.engine_surface.attach(None, 0, 0);
            self.engine_surface.commit();
            return;
        }

        // Update viewporter buffer transform.

        let buffer_size: Size<f64> = engine.buffer_size().into();
        let src_width = buffer_size.width.min(max_engine_size.width);
        let src_height = buffer_size.height.min(max_engine_size.height);
        self.engine_viewport.set_source(0., 0., src_width, src_height);

        let dst_width = (src_width / self.scale).round() as i32;
        let dst_height = (src_height / self.scale).round() as i32;
        self.engine_viewport.set_destination(dst_width, dst_height);

        // Update opaque region.
        self.engine_surface.set_opaque_region(engine.opaque_region());

        // Attach buffer with its damage since the last frame.
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

        self.last_rendered_engine = self.active_tab;
        self.stalled = false;
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
        let fullscreen_request = self.fullscreen_request.take();
        let fullscreened = self.fullscreened;
        if let Some(engine) = self.active_tab_mut() {
            if Some(engine.id()) == fullscreen_request {
                engine.set_fullscreen(fullscreened);
            }
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
            KeyboardFocus::Overlay => self.overlay.press_key(raw, keysym, modifiers),
            KeyboardFocus::Browser => {
                let engine = match self.active_tab_mut() {
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
                if let Some(engine) = self.active_tab_mut() {
                    engine.release_key(time, raw, keysym, modifiers);
                }
            },
            // Ui has no release handling need (yet).
            KeyboardFocus::Ui | KeyboardFocus::Overlay | KeyboardFocus::None => (),
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
            if let Some(engine) = self.active_tab_mut() {
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
            self.set_keyboard_focus(KeyboardFocus::Browser);

            // Use real pointer events for the browser engine.
            if let Some(engine) = self.active_tab_mut() {
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
            if let Some(engine) = self.active_tab_mut() {
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
            if let Some(engine) = self.active_tab_mut() {
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
            if let Some(engine) = self.active_tab_mut() {
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

        // Forward events to corresponding surface.
        if &self.engine_surface == surface {
            self.set_keyboard_focus(KeyboardFocus::Browser);

            // Close active text input popups.
            if let Some((id, _)) = self.text_menu.take() {
                self.close_option_menu(id);
            }

            if let Some(engine) = self.active_tab_mut() {
                // Close all dropdowns when interacting with the page.
                engine.close_option_menu(None);

                engine.touch_down(time, id, position, modifiers);
            }
        } else if self.ui.surface() == surface {
            self.set_keyboard_focus(KeyboardFocus::Ui);

            // Close all dropdowns when clicking on the UI.
            if let Some(engine) = self.active_tab_mut() {
                engine.close_option_menu(None);
            }
            if let Some((id, _)) = self.text_menu.take() {
                self.close_option_menu(id);
            }

            self.ui.touch_down(time, id, position, modifiers);
        } else if self.overlay.surface() == surface {
            self.overlay.touch_down(time, id, position, modifiers);

            // Set keyboard focus, if the overlay has an input element focused.
            if self.overlay.has_keyboard_focus() {
                self.set_keyboard_focus(KeyboardFocus::Overlay);
            }
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
            if let Some(&position) = self.touch_points.get(&id) {
                if let Some(engine) = self.active_tab_mut() {
                    engine.touch_up(time, id, position, modifiers);
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
            if let Some(engine) = self.active_tab_mut() {
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
            KeyboardFocus::Overlay => {
                self.overlay.delete_surrounding_text(before_length, after_length)
            },
            KeyboardFocus::Browser => {
                let engine = match self.active_tab_mut() {
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
            KeyboardFocus::Overlay => self.overlay.commit_string(text),
            KeyboardFocus::Browser => {
                let engine = match self.active_tab_mut() {
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
            KeyboardFocus::Overlay => {
                self.overlay.set_preedit_string(text, cursor_begin, cursor_end)
            },
            KeyboardFocus::Browser => {
                let engine = match self.active_tab_mut() {
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
    pub fn set_engine_uri(&mut self, engine_id: EngineId, uri: String) {
        // Update UI if the URI change is for the active tab.
        if Some(engine_id) == self.active_tab {
            self.ui.set_uri(&uri);

            // Unstall if UI changed.
            if self.ui.dirty() {
                self.unstall();
            }
        }

        // Update tabs popup.
        self.overlay.tabs_mut().set_tabs(self.tabs.values(), self.active_tab);

        let group = self.groups.get(&engine_id.group_id()).unwrap_or(NO_GROUP_REF);
        if !group.ephemeral {
            // Increment URI visit count for history.
            self.history.visit(uri);

            // Update browser session.
            self.persist_session();
        }
    }

    /// Update the window's browser session.
    ///
    /// This is used to recover the browser session when restarting Kumo.
    pub fn persist_session(&self) {
        // Persist latest session state.
        let session = self.tabs.iter().filter_map(|(engine_id, engine)| {
            let group = self.groups.get(&engine_id.group_id()).unwrap_or(NO_GROUP_REF);
            SessionRecord::new(engine, group, Some(*engine_id) == self.active_tab)
        });
        self.session_storage.persist(self.id, session);

        // Ensure sessions' group labels are persisted.
        self.group_storage.persist(self.groups.values());
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

    /// Update an engine's load progress.
    pub fn set_load_progress(&mut self, engine_id: EngineId, progress: f64) {
        if self.active_tab == Some(engine_id) {
            self.ui.set_load_progress(progress);
            self.unstall();
        }
    }

    /// Open or close the tabs UI.
    pub fn set_tabs_ui_visible(&mut self, visible: bool) {
        self.overlay.tabs_mut().set_visible(visible);

        if visible {
            self.set_keyboard_focus(KeyboardFocus::None);
        }

        self.unstall();
    }

    /// Open or close the history UI.
    pub fn set_history_ui_visibile(&mut self, visible: bool) {
        self.overlay.set_history_visible(visible);

        if visible {
            self.set_keyboard_focus(KeyboardFocus::None);
        }

        self.unstall();
    }

    /// Set the history UI filter.
    pub fn set_history_filter(&mut self, filter: String) {
        self.overlay.set_history_filter(filter);
        self.unstall();
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
        if self.history_menu == Some(menu_id) {
            // Load the selected URI.
            let uri = self.history_menu_matches.swap_remove(index).uri;
            self.ui.set_uri(&uri);
            self.load_uri(uri, false);
        } else if self.text_menu.as_ref().is_some_and(|(id, _)| *id == menu_id) {
            let (_, selection) = self.text_menu.take().unwrap();
            match TextMenuItem::from_index(index as u8) {
                Some(TextMenuItem::Paste) => self.queue.request_paste(PasteTarget::Ui(self.id)),
                Some(TextMenuItem::Copy) => self.queue.set_clipboard(selection.unwrap()),
                Some(TextMenuItem::_Invalid) | None => unreachable!(),
            }
            self.close_option_menu(menu_id);
        }
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
            OptionMenuItem { label, description, ..Default::default() }
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

    /// Show text input long-press options menu.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn open_text_menu(&mut self, position: Position, selection: Option<String>) {
        if let Some((id, _)) = self.text_menu.take() {
            self.close_option_menu(id);
        }

        let menu_id = OptionMenuId::new(self.id);
        let items: &[_] = match selection {
            Some(_) => &[TextMenuItem::Paste, TextMenuItem::Copy],
            None => &[TextMenuItem::Paste],
        };
        let items = items.iter().copied().map(|item| item.into());
        self.open_option_menu(menu_id, position, None, items);
        self.text_menu = Some((menu_id, selection));
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
        if Some(engine_id) != self.active_tab {
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

    /// Update the keyboard focus.
    pub fn set_keyboard_focus(&mut self, focus: KeyboardFocus) {
        self.keyboard_focus = focus;

        // Clear UI focus.
        if focus != KeyboardFocus::Ui {
            self.ui.clear_keyboard_focus();
        }
        if focus != KeyboardFocus::Overlay {
            self.overlay.clear_keyboard_focus();
        }

        // Clear engine focus.
        if focus != KeyboardFocus::Browser {
            if let Some(engine) = self.active_tab_mut() {
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
    pub fn dmabuf_feedback_changed(&mut self, new_feedback: &DmabufFeedback) {
        for (_, engine) in &mut self.tabs {
            engine.dmabuf_feedback(new_feedback);
        }
    }

    /// Cycle overview to the next tab group.
    pub fn cycle_tab_group(&mut self, group_id: GroupId) {
        // Get next group.
        let mut groups = self.groups.iter();
        let new_group = match groups.find(|(id, _)| **id == group_id) {
            Some(_) => groups.next().map_or(NO_GROUP_REF, |(_, group)| group),
            None => self.groups.first().map_or(NO_GROUP_REF, |(_, group)| group),
        };

        // Update group ID and refresh tabs list.
        let tabs = self.overlay.tabs_mut();
        tabs.set_active_tab_group(new_group);

        self.unstall();
    }

    /// Set ephemeral mode of the active tab group.
    pub fn set_ephemeral_mode(&mut self, group_id: GroupId, ephemeral: bool) {
        let group = match self.groups.get_mut(&group_id) {
            Some(group) => group,
            None => return,
        };

        // Toggle ephemeral mode and update tabs view.
        group.ephemeral = ephemeral;

        // Update the tab overview if it is currently showing this group.
        let tabs_ui = self.overlay.tabs_mut();
        if tabs_ui.active_tab_group() == group_id {
            tabs_ui.set_active_tab_group(group);
            self.unstall();
        }

        // Ensure toggled group's session is immediately updated.
        self.persist_session();
    }

    /// Create a new tab group.
    ///
    /// The group will not be recreated if the supplied UUID already exists.
    pub fn create_tab_group(&mut self, template: Option<Group>, focus: bool) -> GroupId {
        // Create a new persistent group.
        let group = template.unwrap_or_else(|| Group::new(false));

        // Switch the tabs view to the created group.
        if focus {
            self.overlay.tabs_mut().set_active_tab_group(&group);
        }

        // Store new group if it doesn't exist yet.
        let group_id = group.id();
        if !self.groups.contains_key(&group_id) {
            self.groups.insert(group_id, group);
        }

        self.unstall();

        group_id
    }

    /// Delete a tab group.
    pub fn delete_tab_group(&mut self, group_id: GroupId) {
        // Close all tabs belonging to the group.
        self.tabs.retain(|engine_id, _| engine_id.group_id() != group_id);

        // Switch overview to the next available group.
        self.cycle_tab_group(group_id);

        self.groups.shift_remove(&group_id);

        // Remove deleted tabs from the session storage.
        self.persist_session();
    }

    /// Update the label of the active tab group.
    pub fn update_group_label(&mut self, label: String) {
        let tabs_ui = self.overlay.tabs_mut();
        if let Some(group) = self.groups.get_mut(&tabs_ui.active_tab_group()) {
            // Update group label and tabs UI.
            group.label = label.into();
            tabs_ui.set_active_tab_group(group);

            // Persist new label to database.
            self.group_storage.persist(self.groups.values());
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
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub enum KeyboardFocus {
    None,
    #[default]
    Ui,
    Overlay,
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

/// Target for a clipboard paste action.
#[derive(Copy, Clone, Debug)]
pub enum PasteTarget {
    Browser(EngineId),
    Ui(WindowId),
}

impl PasteTarget {
    /// Get the target window ID.
    pub fn window_id(&self) -> WindowId {
        match self {
            Self::Browser(engine_id) => engine_id.window_id(),
            Self::Ui(window_id) => *window_id,
        }
    }
}

/// Entries for the text input option menu.
#[repr(u8)]
#[derive(Copy, Clone)]
enum TextMenuItem {
    Paste,
    Copy,
    // SAFETY: Must be last value, since it's used for "safe" transmute.
    _Invalid,
}

impl TextMenuItem {
    /// Get item variant from its index.
    fn from_index(index: u8) -> Option<Self> {
        if index >= Self::_Invalid as u8 {
            return None;
        }

        Some(unsafe { mem::transmute::<u8, Self>(index) })
    }

    /// Get the text label for this item.
    fn label(&self) -> &'static str {
        match self {
            Self::Paste => "Paste",
            Self::Copy => "Copy",
            Self::_Invalid => unreachable!(),
        }
    }
}

impl From<TextMenuItem> for OptionMenuItem {
    fn from(item: TextMenuItem) -> Self {
        OptionMenuItem { label: item.label().into(), ..Default::default() }
    }
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
