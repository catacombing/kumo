//! Browser configuration UI.

use std::mem;
use std::str::FromStr;

use funq::MtQueueHandle;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use toml::Value;
use tracing::error;

use crate::config::{self, CONFIG};
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{ScrollVelocity, SvgButton, TextField};
use crate::window::{TextInputChange, WindowId};
use crate::{Error, Position, Size, State, gl, rect_contains};

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Padding around buttons.
const BUTTON_PADDING: f64 = 10.;

/// Height of each individual settings entry.
const ENTRY_HEIGHT: u32 = 30;

/// Padding between settings.
const ENTRY_Y_PADDING: f64 = 10.;

/// Padding around all setting entries.
const OUTSIDE_PADDING: f64 = 25.;

/// Padding around the setting's text field text.
const TEXT_FIELD_PADDING: f64 = 10.;

#[funq::callbacks(State)]
trait SettingsHandler {
    /// Close the settings UI.
    fn close_settings(&mut self, window_id: WindowId);

    /// Persist settings to config file.
    fn save_settings(&mut self, window_id: WindowId);

    /// Reset all settings to the config value.
    fn reset_settings(&mut self, window_id: WindowId);

    /// Handle setting changes.
    fn set_setting(
        &mut self,
        window_id: WindowId,
        path: String,
        format: InputFormat,
        value: String,
    );

    /// Clear settings value.
    fn reset_setting(&mut self, path: String);
}

impl SettingsHandler for State {
    fn close_settings(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_settings_ui_visible(false);
    }

    fn save_settings(&mut self, window_id: WindowId) {
        match self.config_manager.persist() {
            Ok(_) => {
                let window = match self.windows.get_mut(&window_id) {
                    Some(window) => window,
                    None => return,
                };

                window.set_settings_savable(false);
            },
            Err(err) => error!("Failed to save config: {err}"),
        }
    }

    fn reset_settings(&mut self, window_id: WindowId) {
        self.config_manager.reset::<&str>(&[]);

        config::reload_config(&self.config_manager);

        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.reload_settings();

        window.set_settings_savable(false);
    }

    fn set_setting(
        &mut self,
        window_id: WindowId,
        path: String,
        format: InputFormat,
        value: String,
    ) {
        let value = match format.toml_value(&value) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to parse setting value: {err}");
                return;
            },
        };

        let path: Vec<_> = path.split('.').collect();
        self.config_manager.set(&path, value);

        config::reload_config(&self.config_manager);

        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.set_settings_savable(true);
    }

    fn reset_setting(&mut self, path: String) {
        let path: Vec<_> = path.split('.').collect();
        self.config_manager.reset(&path);

        config::reload_config(&self.config_manager);
    }
}

/// Settings UI.
pub struct Settings {
    entries: Vec<SettingsEntry>,
    close_button: SvgButton,
    reset_button: SvgButton,
    save_button: SvgButton,
    scroll_offset: f64,
    savable: bool,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_state: TouchState,
    velocity: ScrollVelocity,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    last_config: u32,
    visible: bool,
    dirty: bool,
}

impl Settings {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        let mut entries = Vec::new();
        load_entries(&mut entries, LoadOperation::Populate(window_id, queue.clone()));

        Self {
            window_id,
            entries,
            queue,
            close_button: SvgButton::new(Svg::ArrowLeft),
            reset_button: SvgButton::new(Svg::Reset),
            save_button: SvgButton::new(Svg::Save),
            scale: 1.,
            keyboard_focus: Default::default(),
            scroll_offset: Default::default(),
            last_config: Default::default(),
            touch_state: Default::default(),
            velocity: Default::default(),
            savable: Default::default(),
            visible: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Check whether the popup is active.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide a popup.
    pub fn set_visible(&mut self, visible: bool) {
        self.dirty |= self.visible != visible;
        self.visible = visible;
    }

    /// Reload setting values from current config.
    pub fn reload_settings(&mut self) {
        // Ensure references to old entries are cleared.
        self.keyboard_focus = None;
        self.touch_state.reset();

        load_entries(&mut self.entries, LoadOperation::Reload);
    }

    /// Set whether there are changes available for saving to disk.
    pub fn set_savable(&mut self, savable: bool) {
        self.dirty |= self.savable != savable;
        self.savable = savable;
    }

    /// Get default physical UI button size.
    ///
    /// This includes all padding, since that is part of the texture.
    fn button_size(&self) -> Size {
        let height = BUTTON_HEIGHT + (2. * BUTTON_PADDING).round() as u32;
        let width = BUTTON_HEIGHT + (2. * BUTTON_PADDING).round() as u32;
        Size::new(width, height) * self.scale
    }

    /// Physical position of the close button.
    ///
    /// This includes all padding since that is included in the texture.
    fn close_button_position(&self) -> Position<f64> {
        let button_size = self.button_size();
        let x = (self.size.width as f64 * self.scale).round() - button_size.width as f64;
        let y = (self.size.height as f64 * self.scale).round() - button_size.height as f64;
        Position::new(x, y)
    }

    /// Physical position of the save button.
    ///
    /// This includes all padding since that is included in the texture.
    fn save_button_position(&self) -> Position<f64> {
        Position::new(0., self.close_button_position().y)
    }

    /// Physical position of the Reset button.
    ///
    /// This includes all padding since that is included in the texture.
    fn reset_button_position(&self) -> Position<f64> {
        let mut position = self.save_button_position();
        position.x += self.button_size().width as f64;
        position
    }

    /// Physical size of each settings item row.
    fn entry_size(&self) -> Size {
        let width = self.size.width - (2. * OUTSIDE_PADDING).round() as u32;
        Size::new(width, ENTRY_HEIGHT) * self.scale
    }

    /// Get setting's text field at the specified location.
    fn input_at(
        &mut self,
        mut position: Position<f64>,
    ) -> Option<(usize, &mut SettingsEntry, Position<f64>)> {
        let entry_size_int = self.entry_size();
        let entry_size: Size<f64> = entry_size_int.into();

        let outside_padding = (OUTSIDE_PADDING * self.scale).round();
        let y_padding = (ENTRY_Y_PADDING * self.scale).round();
        let entries_end_y = self.close_button_position().y;
        let (text_position, text_size) = SettingsEntry::text_geometry(entry_size_int, self.scale);

        // Check whether position is within list boundaries and inside the setting's
        // text field.
        if position.x < text_position.x + outside_padding
            || position.x >= outside_padding + text_position.x + text_size.width as f64
            || position.y < 0.
            || position.y >= entries_end_y
        {
            return None;
        }

        // Apply current scroll offset.
        position.y -= self.scroll_offset;

        // Check if position is in the setting separator.
        let bottom_relative = (entries_end_y - position.y).round();
        let bottom_relative_y =
            entry_size.height - 1. - (bottom_relative % (entry_size.height + y_padding));
        if bottom_relative_y < 0. {
            return None;
        }

        // Find history entry at the specified offset.
        let index = (bottom_relative / (entry_size.height + y_padding).round()) as usize;
        let entry = self.entries.get_mut(index)?;

        // Filter out headings.
        entry.input.as_ref()?;

        // Get touch position relative to the input's origin.
        let y = entries_end_y - (entry_size.height + y_padding) * (index + 1) as f64
            + y_padding
            + self.scroll_offset;
        let (text_position, _) = SettingsEntry::text_geometry(entry_size_int, self.scale);
        let position = Position::new(outside_padding + text_position.x, y - text_position.y);

        Some((index, entry, position))
    }

    /// Clamp list's viewport offset.
    fn clamp_scroll_offset(&mut self) {
        let old_offset = self.scroll_offset;
        let max_offset = self.max_scroll_offset() as f64;
        self.scroll_offset = self.scroll_offset.clamp(0., max_offset);

        // Cancel velocity after reaching the scroll limit.
        if old_offset != self.scroll_offset {
            self.velocity.set(0.);
            self.dirty = true;
        }
    }

    /// Get maximum scroll offset.
    fn max_scroll_offset(&self) -> usize {
        let entry_padding = (ENTRY_Y_PADDING * self.scale).round() as usize;
        let entry_height = self.entry_size().height;

        // Calculate height available for history entries.
        let ui_height = (self.size.height as f64 * self.scale).round() as usize;
        let close_button_height = self.button_size().height as usize;
        let available_height = ui_height - close_button_height;

        // Calculate height of all history entries.
        let num_entries = self.entries.len();
        let mut entries_height =
            (num_entries * (entry_height as usize + entry_padding)).saturating_sub(entry_padding);

        // Allow a bit of padding at the top.
        let top_padding = (OUTSIDE_PADDING * self.scale).round();
        entries_height += top_padding as usize;

        // Calculate history content outside the viewport.
        entries_height.saturating_sub(available_height)
    }
}

impl Popup for Settings {
    fn dirty(&self) -> bool {
        self.dirty
            || self.velocity.is_moving()
            || CONFIG.read().unwrap().generation != self.last_config
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self, renderer: &Renderer) {
        self.dirty = false;

        // Don't render anything when hidden.
        if !self.visible {
            return;
        }

        // Animate scroll velocity.
        self.velocity.apply(&mut self.scroll_offset);

        // Ensure offset is correct in case entries or window size changed.
        self.clamp_scroll_offset();

        // Update config version ID.
        let config = CONFIG.read().unwrap();
        if self.last_config != config.generation {
            self.last_config = config.generation;

            // Clear cached textures to handle color changes.
            for entry in &mut self.entries {
                entry.dirty = true;
            }
            self.reset_button.dirty = true;
            self.close_button.dirty = true;
            self.save_button.dirty = true;
        }

        // Get geometry required for rendering.
        let ui_height = (self.size.height as f64 * self.scale).round() as f32;
        let outside_padding = (OUTSIDE_PADDING * self.scale) as f32;
        let reset_button_position: Position<f32> = self.reset_button_position().into();
        let close_button_position: Position<f32> = self.close_button_position().into();
        let save_button_position: Position<f32> = self.save_button_position().into();
        let entry_height = self.entry_size().height as f32;
        let button_height = self.button_size().height as i32;

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        let reset_button = self.reset_button.texture();
        let close_button = self.close_button.texture();
        let save_button = self.save_button.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the popup always
        // fills the entire surface.
        let [r, g, b] = config.colors.background.as_f32();
        unsafe {
            gl::ClearColor(r, g, b, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Scissor crop the bottom, to not overlap the buttons.
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(0, button_height, i32::MAX, ui_height as i32);
        }

        // Draw visible textures.
        let mut texture_pos =
            Position::new(outside_padding, close_button_position.y + self.scroll_offset as f32);
        for entry in &mut self.entries {
            // Render only entries within the viewport.
            texture_pos.y -= entry_height;
            if texture_pos.y <= -entry_height {
                break;
            } else if texture_pos.y < close_button_position.y {
                let texture = unsafe { entry.texture() };
                renderer.draw_texture_at(texture, texture_pos, None);
            }

            // Add padding after the entry.
            texture_pos.y -= (ENTRY_Y_PADDING * self.scale) as f32;
        }

        unsafe { gl::Disable(gl::SCISSOR_TEST) };

        // Draw buttons.
        if self.savable {
            renderer.draw_texture_at(save_button, save_button_position, None);
            renderer.draw_texture_at(reset_button, reset_button_position, None);
        }
        renderer.draw_texture_at(close_button, close_button_position, None);
    }

    fn position(&self) -> Position {
        Position::new(0, 0)
    }

    fn set_size(&mut self, size: Size) {
        // Anchor scroll offset to top if focused input is in the screen's top half.
        if let Some(KeyboardInputElement::Setting(_, position)) = self.keyboard_focus {
            if position.y <= self.size.height as f64 / 2. * self.scale {
                let height_delta = size.height as f64 - self.size.height as f64;
                self.scroll_offset -= height_delta * self.scale;
            }
        }

        self.size = size;
        self.dirty = true;

        // Update UI element sizes.
        let entry_size = self.entry_size();
        for entry in &mut self.entries {
            entry.set_geometry(entry_size, self.scale);
        }
        self.reset_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
        self.save_button.set_geometry(self.button_size(), self.scale);
    }

    fn size(&self) -> Size {
        self.size
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        let entry_size = self.entry_size();
        for entry in &mut self.entries {
            entry.set_geometry(entry_size, self.scale);
        }
        self.reset_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
        self.save_button.set_geometry(self.button_size(), self.scale);
    }

    fn opaque_region(&self) -> Size {
        self.size
    }

    fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        logical_position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Cancel velocity when a new touch sequence starts.
        self.velocity.set(0.);

        // Convert position to physical space.
        let position = logical_position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Get button geometries.
        let reset_button_position = self.reset_button_position();
        let close_button_position = self.close_button_position();
        let save_button_position = self.save_button_position();
        let button_size = self.button_size().into();

        if rect_contains(close_button_position, button_size, position) {
            self.touch_state.action = TouchAction::CloseTap;
        } else if rect_contains(save_button_position, button_size, position) {
            self.touch_state.action = TouchAction::SaveTap;
            self.clear_keyboard_focus();
        } else if rect_contains(reset_button_position, button_size, position) {
            self.touch_state.action = TouchAction::ResetTap;
            self.clear_keyboard_focus();
        } else if let Some((index, entry, input_position)) = self.input_at(position) {
            entry.touch_down(time, logical_position, position - input_position);
            self.dirty |= entry.input.as_ref().is_some_and(|i| i.dirty);

            self.keyboard_focus = Some(KeyboardInputElement::Setting(index, input_position));
            self.touch_state.action = TouchAction::SettingsInput(index, input_position);

            // Ensure focus is disabled for all other text fields.
            for input in self
                .entries
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .filter_map(|(_, e)| e.input.as_mut())
            {
                input.set_focus(false);
            }
        } else {
            self.touch_state.action = TouchAction::None;
            self.clear_keyboard_focus();
        }
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }

        // Update touch position.
        let position = position * self.scale;
        let old_position = mem::replace(&mut self.touch_state.position, position);

        match self.touch_state.action {
            // Handle transition from tap to drag.
            TouchAction::None | TouchAction::Scrolling => {
                // Ignore dragging until tap distance limit is exceeded.
                let max_tap_distance = CONFIG.read().unwrap().input.max_tap_distance;
                let delta = self.touch_state.position - self.touch_state.start;
                if delta.x.powi(2) + delta.y.powi(2) <= max_tap_distance {
                    return;
                }
                self.touch_state.action = TouchAction::Scrolling;

                // Calculate current scroll velocity.
                let delta = self.touch_state.position.y - old_position.y;
                self.velocity.set(delta);

                // Immediately start moving the entries.
                let old_offset = self.scroll_offset;
                self.scroll_offset += delta;
                self.clamp_scroll_offset();
                self.dirty |= self.scroll_offset != old_offset;
            },
            // Forward input to touched settings text field.
            TouchAction::SettingsInput(index, input_position) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.touch_motion(position - input_position);
                    self.dirty |= entry.input.as_ref().is_some_and(|i| i.dirty);
                }
            },
            _ => (),
        }
    }

    fn touch_up(&mut self, time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        match self.touch_state.action {
            TouchAction::CloseTap => self.queue.close_settings(self.window_id),
            TouchAction::ResetTap => {
                self.queue.reset_settings(self.window_id);
                self.dirty = true;
            },
            TouchAction::SaveTap => self.queue.save_settings(self.window_id),
            // Forward input to touched settings text field.
            TouchAction::SettingsInput(index, _) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.touch_up(time);
                    self.dirty |= entry.input.as_ref().is_some_and(|i| i.dirty);
                }
            },
            _ => (),
        }
    }

    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                input.press_key(raw, keysym, modifiers);
                self.dirty |= input.dirty;
            }
        }
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                input.delete_surrounding_text(before_length, after_length);
                self.dirty |= input.dirty;
            }
        }
    }

    fn commit_string(&mut self, text: &str) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                input.commit_string(text);
                self.dirty |= input.dirty;
            }
        }
    }

    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                input.set_preedit_string(text, cursor_begin, cursor_end);
                self.dirty |= input.dirty;
            }
        }
    }

    fn text_input_state(&mut self) -> TextInputChange {
        if let Some(KeyboardInputElement::Setting(index, position)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                return input.text_input_state(position);
            }
        }
        TextInputChange::Disabled
    }

    fn paste(&mut self, text: &str) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus {
            if let Some(input) = self.entries.get_mut(index).and_then(|e| e.input.as_mut()) {
                input.paste(text);
                self.dirty |= input.dirty;
            }
        }
    }

    fn has_keyboard_focus(&self) -> bool {
        self.keyboard_focus.is_some()
    }

    fn clear_keyboard_focus(&mut self) {
        if let Some(KeyboardInputElement::Setting(index, _)) = self.keyboard_focus.take()
            && let Some(input) = &mut self.entries[index].input
        {
            input.set_focus(false);
        }
        self.dirty = true;
    }
}

/// Renderable entry in the settings UI.
struct SettingsEntry {
    texture: Option<Texture>,

    input: Option<TextField>,
    label: &'static str,

    size: Size,
    scale: f64,

    dirty: bool,
}

impl SettingsEntry {
    fn new(
        window_id: WindowId,
        mut queue: MtQueueHandle<State>,
        path: &'static str,
        format: InputFormat,
        label: &'static str,
        value: &str,
    ) -> Self {
        let font_family = match format {
            InputFormat::None => CONFIG.read().unwrap().font.family.clone(),
            _ => CONFIG.read().unwrap().font.monospace_family.clone(),
        };
        let font_size = CONFIG.read().unwrap().font.size(1.);

        let mut input = TextField::with_family(window_id, queue.clone(), font_family, font_size);
        input.set_text(value);
        let _ = input.set_text_change_handler(Box::new(move |text_field| {
            // Remove illegal characters until validation is successful.
            let mut text = text_field.text();
            let text_len = text.len();
            let mut validation_result = format.validate(&text);
            while validation_result == ValidationResult::Failed {
                text.pop();
                validation_result = format.validate(&text)
            }

            // Update text field if we popped any characters.
            if text.len() != text_len {
                text_field.set_text(&text);
            }

            // Update setting for completed input, or reset if the field is empty.
            if text.is_empty() {
                queue.reset_setting(path.to_string());
            } else if validation_result == ValidationResult::Success {
                queue.set_setting(window_id, path.to_string(), format, text);
            }
        }));

        Self {
            label,
            input: Some(input),
            dirty: true,
            scale: 1.,
            texture: Default::default(),
            size: Default::default(),
        }
    }

    fn new_heading(label: &'static str) -> Self {
        Self {
            label,
            dirty: true,
            scale: 1.,
            texture: Default::default(),
            input: Default::default(),
            size: Default::default(),
        }
    }

    /// Get the rendered texture.
    ///
    /// # Safety
    ///
    /// This is only safe to call while the OpenGL context for the settings UI's
    /// renderer is bound.
    unsafe fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) || self.input.as_ref().is_some_and(|i| i.dirty) {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = match &self.input {
                Some(_) => Some(self.draw_setting()),
                None => Some(self.draw_heading()),
            };

            if let Some(input) = &mut self.input {
                input.dirty = false;
            }
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw a setting into an OpenGL texture.
    ///
    /// # Panics
    ///
    /// Panics if the underlying entry is a heading and `input` is `None`.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw_setting(&mut self) -> Texture {
        let input = self.input.as_mut().unwrap();
        let config = CONFIG.read().unwrap();

        // Clear with input area's background color.
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(config.colors.alt_background.as_f64());

        let (mut input_position, input_size) = Self::text_geometry(self.size, self.scale);
        input_position.x += input.scroll_offset;

        // Configure text field's rendering options.
        let mut text_options = TextOptions::new();
        text_options.cursor_position(input.cursor_index());
        text_options.autocomplete(input.autocomplete().into());
        text_options.preedit(input.preedit.clone());
        text_options.position(input_position);
        text_options.size(input_size.into());
        text_options.set_ellipsize(false);

        if input.focused {
            // While focused, show selection or input cursor.
            if input.selection.is_some() {
                text_options.selection(input.selection.clone());
            } else {
                text_options.show_cursor();
            }
        }

        // Rasterize the input field.
        let layout = input.layout();
        layout.set_scale(self.scale);
        builder.rasterize(layout, &text_options);

        // Draw background over input, hiding scrolled content.
        let [bgr, bgg, bgb] = config.colors.background.as_f64();
        let context = builder.context();
        context.rectangle(0., 0., (self.size.width as f64 / 2.).ceil(), self.size.height as f64);
        context.set_source_rgb(bgr, bgg, bgb);
        context.fill().unwrap();

        // Draw setting's label.
        let text_padding = (TEXT_FIELD_PADDING * self.scale).round() as u32;
        let width = self.size.width - input_size.width - 2 * text_padding;
        let size = Size::new(width, self.size.height);
        let layout = TextLayout::new(config.font.size(1.), self.scale);
        layout.set_text(self.label);
        let mut text_options = TextOptions::new();
        text_options.size(size.into());
        builder.rasterize(&layout, &text_options);

        builder.build()
    }

    /// Draw a heading into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw_heading(&mut self) -> Texture {
        let config = CONFIG.read().unwrap();

        // Clear with input area's background color.
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(config.colors.background.as_f64());

        // Draw heading's text.
        let layout = TextLayout::new(config.font.size(1.25), self.scale);
        layout.set_text(self.label);
        builder.rasterize(&layout, &TextOptions::new());

        builder.build()
    }

    /// Set the physical size and scale of the element.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Update text input width.
        if let Some(input) = &mut self.input {
            let (_, text_size) = Self::text_geometry(self.size, self.scale);
            input.set_width(text_size.width as f64);
        }

        // Force redraw.
        self.dirty = true;
    }

    /// Get physical geometry of the text input area.
    fn text_geometry(size: Size, scale: f64) -> (Position<f64>, Size) {
        let text_padding = (TEXT_FIELD_PADDING * scale).round() as u32;
        let width = size.width / 2 - text_padding * 2;
        let x = (size.width - width - text_padding) as f64;

        let position = Position::new(x, 0.);
        let size = Size::new(width, size.height);

        (position, size)
    }

    /// Handle touch press events.
    fn touch_down(
        &mut self,
        time: u32,
        absolute_position: Position<f64>,
        relative_position: Position<f64>,
    ) {
        let input = match &mut self.input {
            Some(input) => input,
            None => return,
        };

        input.touch_down(time, absolute_position, relative_position);
        input.set_focus(true);
    }

    /// Handle touch motion events.
    fn touch_motion(&mut self, relative_position: Position<f64>) {
        let input = match &mut self.input {
            Some(input) => input,
            None => return,
        };

        input.touch_motion(relative_position);
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32) {
        let input = match &mut self.input {
            Some(input) => input,
            None => return,
        };

        input.touch_up(time);
    }
}

/// Format for a setting's input element.
#[derive(Copy, Clone)]
enum InputFormat {
    /// Any text input.
    None,
    /// Hexadecimal color.
    Color,
    /// Floating point number.
    Float,
    /// Integer number.
    Integer,
}

impl InputFormat {
    /// Return [`true`] if text is acceptable for this format.
    fn validate(&self, text: &str) -> ValidationResult {
        if text.is_empty() {
            return ValidationResult::Incomplete;
        }

        match self {
            Self::None => ValidationResult::Success,

            Self::Float if f64::from_str(text).is_ok() => ValidationResult::Success,
            Self::Float => ValidationResult::Failed,

            Self::Integer if i64::from_str(text).is_ok() => ValidationResult::Success,
            Self::Integer => ValidationResult::Failed,

            Self::Color if !text.starts_with('#') => ValidationResult::Failed,
            Self::Color if text[1..].chars().any(|c| !c.is_ascii_hexdigit()) => {
                ValidationResult::Failed
            },
            Self::Color if text.len() > 7 => ValidationResult::Failed,
            Self::Color if text.len() < 7 => ValidationResult::Incomplete,
            Self::Color => ValidationResult::Success,
        }
    }

    /// Parse text as toml value for this format.
    fn toml_value(&self, text: &str) -> Result<Value, Error> {
        match self {
            Self::None | Self::Color => Ok(Value::String(text.into())),
            Self::Float => {
                let float = f64::from_str(text)?;
                Ok(Value::Float(float))
            },
            Self::Integer => {
                let float = i64::from_str(text)?;
                Ok(Value::Integer(float))
            },
        }
    }
}

/// Result of [`InputFormat::validate`]
#[derive(Copy, Clone, PartialEq, Eq)]
enum ValidationResult {
    /// Text is valid in its entirety.
    Success,
    /// Text contains illegal characters.
    Failed,
    /// Text could be part of a valid input, but is incomplete.
    Incomplete,
}

/// Touch event tracking.
#[derive(Default)]
struct TouchState {
    slot: Option<i32>,
    action: TouchAction,
    start: Position<f64>,
    position: Position<f64>,
}

impl TouchState {
    fn reset(&mut self) {
        self.action = TouchAction::None;
        self.slot = None;
    }
}

/// Intention of a touch sequence.
#[derive(Default, Copy, Clone, PartialEq, Debug)]
enum TouchAction {
    #[default]
    None,
    Scrolling,
    CloseTap,
    ResetTap,
    SaveTap,
    SettingsInput(usize, Position<f64>),
}

/// Elements accepting keyboard focus.
#[derive(Debug)]
enum KeyboardInputElement {
    Setting(usize, Position<f64>),
}

/// Populate or reload the settings entries.
fn load_entries(entries: &mut Vec<SettingsEntry>, op: LoadOperation) {
    let config = CONFIG.read().unwrap();

    let mut index = 0;
    let index_mut = &mut index;
    macro_rules! add_setting {
        ($label:literal, $($path:ident).+, $fmt:expr $(,)?) => {{
            let default = config. $($path).+ .to_string();
            match op {
                LoadOperation::Populate(window_id, ref queue) => {
                    let path = stringify!($($path).+);
                    let entry = SettingsEntry::new(window_id, queue.clone(), path, $fmt, $label, &default);
                    entries.push(entry);
                }
                LoadOperation::Reload => {
                    // Update setting's text field, without triggering text change handler.
                    if let Some(input) = &mut entries[*index_mut].input {
                        let old_handler = input.set_text_change_handler(Box::new(|_| {}));
                        input.set_text(&default);
                        let _ = input.set_text_change_handler(old_handler);
                    }
                    *index_mut += 1;
                },
            }
        }};
    }
    macro_rules! add_heading {
        ($label:literal $(,)?) => {{
            if let LoadOperation::Populate(..) = op {
                let entry = SettingsEntry::new_heading($label);
                entries.push(entry);
            }
            *index_mut += 1;
        }};
    }

    add_setting!("Monospace Family", font.monospace_family, InputFormat::None);
    add_setting!("Size", font.size, InputFormat::Float);
    add_setting!("Family", font.family, InputFormat::None);
    add_heading!("Font");

    add_setting!("Disabled", colors.disabled, InputFormat::Color);
    add_setting!("Error", colors.error, InputFormat::Color);
    add_setting!("Highlight", colors.highlight, InputFormat::Color);
    add_setting!("Alt Background", colors.alt_background, InputFormat::Color);
    add_setting!("Alt Foreground", colors.alt_foreground, InputFormat::Color);
    add_setting!("Background", colors.background, InputFormat::Color);
    add_setting!("Foreground", colors.foreground, InputFormat::Color);
    add_heading!("Colors");

    add_setting!("Uri", search.uri, InputFormat::None);
    add_heading!("Search");

    add_setting!("Velocity Friction", input.velocity_friction, InputFormat::Float);
    add_setting!("Velocity Interval", input.velocity_interval, InputFormat::Integer);
    add_setting!("Max Tap Distance", input.max_tap_distance, InputFormat::Integer);
    add_setting!("Long Press", input.long_press, InputFormat::Integer);
    add_heading!("Input");
}

/// Settings entry load operation.
enum LoadOperation {
    Populate(WindowId, MtQueueHandle<State>),
    Reload,
}
