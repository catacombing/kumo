//! Downloads overlay.

use std::collections::HashMap;
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use funq::MtQueueHandle;
use indexmap::IndexMap;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::config::{CONFIG, Colors, Config};
use crate::engine::EngineId;
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{ScrollVelocity, SvgButton};
use crate::window::WindowId;
use crate::{Position, Size, State, gl, rect_contains};

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Padding around buttons.
const BUTTON_PADDING: f64 = 10.;

/// Logical height of each download entry.
const ENTRY_HEIGHT: u32 = 65;

/// Horizontal tabbing around download entries.
const ENTRY_X_PADDING: f64 = 10.;

/// Vertical padding between download entries.
const ENTRY_Y_PADDING: f64 = 1.;

/// Padding around the download entry "X" button.
const ENTRY_CLOSE_PADDING: f64 = 40.;

#[funq::callbacks(State)]
trait DownloadsHandler {
    /// Close the downloads UI.
    fn close_downloads(&mut self, window_id: WindowId);

    /// Cancel a file download.
    fn cancel_download(&mut self, download_id: DownloadId);
}

impl DownloadsHandler for State {
    fn close_downloads(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_downloads_ui_visible(false);
    }

    fn cancel_download(&mut self, download_id: DownloadId) {
        let window = match self.windows.get_mut(&download_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        window.cancel_download(download_id);
    }
}

/// Downloads UI.
pub struct Downloads {
    texture_cache: TextureCache,
    delete_button: SvgButton,
    close_button: SvgButton,
    scroll_offset: f64,

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

impl Downloads {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        Self {
            window_id,
            queue,
            close_button: SvgButton::new(Svg::Close),
            delete_button: SvgButton::new(Svg::Bin),
            scale: 1.,
            texture_cache: Default::default(),
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            last_config: Default::default(),
            velocity: Default::default(),
            visible: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Add a new download.
    pub fn add_download(&mut self, download: Download) {
        self.texture_cache.entries.insert(download.id, download);
        self.dirty = true;
    }

    /// Update a download's progress.
    ///
    /// A progress value of `None` indicates that the download has failed and
    /// will not make any further progress.
    pub fn set_download_progress(&mut self, download_id: DownloadId, progress: Option<u8>) {
        if let Some(download) = self.texture_cache.entries.get_mut(&download_id) {
            match progress {
                Some(progress) => download.progress = progress,
                None => {
                    download.progress = 100;
                    download.failed = true;
                },
            }
            self.dirty = true;
        }
    }

    /// Get number of tracked downloads.
    pub fn download_count(&mut self) -> usize {
        self.texture_cache.entries.len()
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

    /// Physical position of the bulk delete button.
    ///
    /// This includes all padding since that is included in the texture.
    fn delete_button_position(&self) -> Position<f64> {
        let size: Size<f64> = self.size.into();
        let x = (size.width - 2. * BUTTON_HEIGHT as f64 - 3. * BUTTON_PADDING).round();
        let y = (size.height - BUTTON_HEIGHT as f64 - 2. * BUTTON_PADDING).round();
        Position::new(x, y) * self.scale
    }

    /// Get physical size of the download entry close button.
    fn close_entry_button_size(entry_size: Size, scale: f64) -> Size<f64> {
        let size = entry_size.height as f64 - ENTRY_CLOSE_PADDING * scale;
        Size::new(size, size)
    }

    /// Get physical position of the close button within a download entry.
    fn close_entry_button_position(entry_size: Size, scale: f64) -> Position<f64> {
        let icon_size = Self::close_entry_button_size(entry_size, scale);
        let button_padding = (entry_size.height as f64 - icon_size.height) / 2.;
        let x = entry_size.width as f64 - button_padding - icon_size.width;
        Position::new(x, button_padding)
    }

    /// Physical size of each download entry.
    fn entry_size(&self) -> Size {
        let width = self.size.width - (2. * ENTRY_X_PADDING).round() as u32;
        Size::new(width, ENTRY_HEIGHT) * self.scale
    }

    /// Get entry at the specified location.
    ///
    /// The tuple's second element will be `true` when the position matches the
    /// close button of the download entry.
    fn entry_at(&mut self, mut position: Position<f64>) -> Option<(&mut Download, bool)> {
        let y_padding = ENTRY_Y_PADDING * self.scale;
        let x_padding = ENTRY_X_PADDING * self.scale;
        let entry_end_y = self.close_button_position().y;

        let entry_size_int = self.entry_size();
        let entry_size: Size<f64> = entry_size_int.into();

        // Check whether position is within downloads list boundaries.
        if position.x < x_padding
            || position.x >= x_padding + entry_size.width
            || position.y < 0.
            || position.y >= entry_end_y
        {
            return None;
        }

        // Apply current scroll offset.
        position.y -= self.scroll_offset;

        // Check if position is in the entry separator.
        let bottom_relative = (entry_end_y - position.y).round();
        let bottom_relative_y =
            entry_size.height - 1. - (bottom_relative % (entry_size.height + y_padding));
        if bottom_relative_y < 0. {
            return None;
        }

        // Find download entry at the specified offset.
        let rindex = (bottom_relative / (entry_size.height + y_padding).round()) as usize;
        let index = self.texture_cache.entries.len() - 1 - rindex;
        let (_, entry) = self.texture_cache.entries.get_index_mut(index)?;

        // Check if click is within close button bounds.
        //
        // We include padding for the close button since it can be really hard to hit
        // otherwise.
        let close_position = Self::close_entry_button_position(entry_size_int, self.scale);
        let entry_relative_x = position.x - x_padding;
        let close = entry_relative_x >= close_position.x - close_position.y;

        Some((entry, close))
    }

    /// Clamp downloads list viewport offset.
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

    /// Get maximum downloads list scroll offset.
    fn max_scroll_offset(&self) -> usize {
        let entry_padding = (ENTRY_Y_PADDING * self.scale).round() as usize;
        let entry_height = self.entry_size().height;

        // Calculate height available for download entries.
        let ui_height = (self.size.height as f64 * self.scale).round() as usize;
        let close_button_height = self.button_size().height as usize;
        let available_height = ui_height - close_button_height;

        // Calculate height of all download entries.
        let num_entries = self.texture_cache.len();
        let mut entries_height =
            (num_entries * (entry_height as usize + entry_padding)).saturating_sub(entry_padding);

        // Allow a bit of padding at the top.
        let top_padding = (BUTTON_PADDING * self.scale).round();
        entries_height += top_padding as usize;

        // Calculate downloads content outside the viewport.
        entries_height.saturating_sub(available_height)
    }
}

impl Popup for Downloads {
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

            // Force SVG button redraw.
            self.delete_button.dirty = true;
            self.close_button.dirty = true;
        }

        // Get geometry required for rendering.
        let x_padding = (ENTRY_X_PADDING * self.scale) as f32;
        let delete_button_position: Position<f32> = self.delete_button_position().into();
        let close_button_position: Position<f32> = self.close_button_position().into();
        let ui_height = (self.size.height as f64 * self.scale).round() as f32;
        let button_height = self.button_size().height as i32;
        let entry_size = self.entry_size();

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        unsafe { self.texture_cache.free_unused_textures() };
        let delete_button = self.delete_button.texture();
        let close_button = self.close_button.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the downloads
        // popup always fills the entire surface.
        let [r, g, b] = config.colors.background.as_f32();
        unsafe {
            gl::ClearColor(r, g, b, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Scissor crop bottom entry, to not overlap the buttons.
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(0, button_height, i32::MAX, ui_height as i32);
        }

        // Draw downloads list.
        let mut texture_pos =
            Position::new(x_padding, close_button_position.y + self.scroll_offset as f32);
        for i in (0..self.texture_cache.len()).rev() {
            // Render only entries within the viewport.
            texture_pos.y -= entry_size.height as f32;
            if texture_pos.y <= -(entry_size.height as f32) {
                break;
            } else if texture_pos.y < close_button_position.y {
                let texture = self.texture_cache.texture(&config, i, entry_size, self.scale);
                renderer.draw_texture_at(texture, texture_pos, None);
            }

            // Add padding after the downloads entry.
            texture_pos.y -= (ENTRY_Y_PADDING * self.scale) as f32
        }

        unsafe { gl::Disable(gl::SCISSOR_TEST) };

        // Draw buttons.
        renderer.draw_texture_at(delete_button, delete_button_position, None);
        renderer.draw_texture_at(close_button, close_button_position, None);
    }

    fn position(&self) -> Position {
        Position::new(0, 0)
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update UI element sizes.
        self.delete_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
    }

    fn size(&self) -> Size {
        self.size
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        self.delete_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
    }

    fn opaque_region(&self) -> Size {
        self.size
    }

    fn touch_down(
        &mut self,
        _time: u32,
        id: i32,
        logical_position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Convert position to physical space.
        let position = logical_position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Get button geometries.
        let delete_button_position = self.delete_button_position();
        let close_button_position = self.close_button_position();
        let button_size = self.button_size().into();

        if rect_contains(delete_button_position, button_size, position) {
            self.touch_state.action = TouchAction::DeleteTap;
        } else if rect_contains(close_button_position, button_size, position) {
            self.touch_state.action = TouchAction::CloseTap;
        } else {
            self.touch_state.action = TouchAction::EntryTap;
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
            TouchAction::EntryTap | TouchAction::EntryDrag => {
                // Ignore dragging until tap distance limit is exceeded.
                let max_tap_distance = CONFIG.read().unwrap().input.max_tap_distance;
                let delta = self.touch_state.position - self.touch_state.start;
                if delta.x.powi(2) + delta.y.powi(2) <= max_tap_distance {
                    return;
                }
                self.touch_state.action = TouchAction::EntryDrag;

                // Calculate current scroll velocity.
                let delta = self.touch_state.position.y - old_position.y;
                self.velocity.set(delta);

                // Immediately start moving the downloads list.
                let old_offset = self.scroll_offset;
                self.scroll_offset += delta;
                self.clamp_scroll_offset();
                self.dirty |= self.scroll_offset != old_offset;
            },
            // Ignore drag when tap started on a UI element.
            _ => (),
        }
    }

    fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        match self.touch_state.action {
            // Cancel active and remove inactive downloads on `X` button press.
            TouchAction::EntryTap => {
                if let Some((download, true)) = self.entry_at(self.touch_state.start) {
                    let download_id = download.id;
                    if download.progress < 100 {
                        // Mark download as failed.
                        download.progress = 100;
                        download.failed = true;
                        self.dirty = true;

                        // Request cancellation from the engine.
                        self.queue.cancel_download(download_id);
                    } else {
                        self.texture_cache.entries.shift_remove(&download_id);
                        self.dirty = true;
                    }
                }
            },
            // Close the downloads UI.
            TouchAction::CloseTap => self.queue.close_downloads(self.window_id),
            // Remove all completed downloads.
            TouchAction::DeleteTap => {
                self.dirty |= !self.texture_cache.entries.is_empty();
                self.texture_cache.entries.retain(|_, download| download.progress < 100);
            },
            TouchAction::EntryDrag => (),
        }
    }
}

/// Download texture cache by URI.
#[derive(Default)]
struct TextureCache {
    textures: HashMap<TextureCacheKey, Texture>,
    entries: IndexMap<DownloadId, Download>,
}

impl TextureCache {
    /// Cleanup unused textures.
    ///
    /// # Safety
    ///
    /// The correct OpenGL context **must** be current or this will attempt to
    /// delete invalid OpenGL textures.
    #[cfg_attr(feature = "profiling", profiling::function)]
    unsafe fn free_unused_textures(&mut self) {
        self.textures.retain(|key, texture| {
            let retain =
                self.entries.get(&key.id).is_some_and(|download| download.progress == key.progress);

            // Release OpenGL texture.
            if !retain {
                texture.delete();
            }

            retain
        });
    }

    /// Get the texture for a download entry.
    ///
    /// This will automatically take care of caching rendered textures.
    ///
    /// ### Panics
    ///
    /// Panics if `index >= self.len()`.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn texture(&mut self, config: &Config, index: usize, entry_size: Size, scale: f64) -> &Texture {
        let (_, download) = &self.entries.get_index(index).unwrap();
        let key = TextureCacheKey {
            progress: download.progress,
            failed: download.failed,
            colors: config.colors,
            id: download.id,
        };

        // Create and cache texture if necessary.
        self.textures.entry(key).or_insert_with(|| {
            // Extract filename from destination path.
            let destination_path = PathBuf::from(&download.destination);
            let filename = destination_path
                .file_name()
                .and_then(|filename| filename.to_str())
                .unwrap_or("unknown");

            // Replace home prefix with `~`.
            let homed_destination = match glib::home_dir().to_str() {
                Some(home) => &download.destination.replace(home, "~"),
                None => &download.destination,
            };

            // Create filename text layout.
            let filename_layout = TextLayout::new(config.font.size(1.13), scale);
            let filename_height = filename_layout.line_height();
            filename_layout.set_text(filename);

            // Create path text layout.
            let path_layout = TextLayout::new(config.font.size(0.63), scale);
            let path_height = path_layout.line_height();
            path_layout.set_text(homed_destination);

            // Create uri text layout.
            let uri_layout = TextLayout::new(config.font.size(0.63), scale);
            let uri_height = uri_layout.line_height();
            uri_layout.set_text(&download.uri);

            // Get Y text padding above filename.
            let y_padding =
                ((entry_size.height as i32 - filename_height - path_height - uri_height) / 2)
                    as f64;

            // Configure text rendering options.
            let mut text_options = TextOptions::new();

            // Calculate available area for font rendering.
            let close_position = Downloads::close_entry_button_position(entry_size, scale);
            let text_width = (close_position.x - close_position.y * 2.).round() as i32;
            let filename_size = Size::new(text_width, filename_height);
            text_options.position(Position::new(close_position.y, y_padding));
            text_options.size(filename_size);

            // Create texture with uniform background.
            let builder = TextureBuilder::new(entry_size.into());
            builder.clear(config.colors.alt_background.as_f64());

            // Render load progress indication.
            if download.progress < 100 {
                let width = entry_size.width as f64 / 100. * download.progress.max(5) as f64;
                let hl = config.colors.highlight.as_f64();

                let context = builder.context();
                context.rectangle(0., 0., width, entry_size.height as f64);
                context.set_source_rgba(hl[0], hl[1], hl[2], 0.5);
                context.fill().unwrap();
            }

            // Render filename text to the texture.
            if download.failed {
                text_options.text_color(config.colors.error.as_f64());
            }
            builder.rasterize(&filename_layout, &text_options);

            // Render path text to the texture.
            let path_size = Size::new(text_width, path_height);
            let path_y = y_padding + filename_height as f64;
            text_options.position(Position::new(close_position.y, path_y));
            text_options.size(path_size);
            text_options.text_color(config.colors.alt_foreground.as_f64());
            builder.rasterize(&path_layout, &text_options);

            // Render uri text to the texture.
            let uri_size = Size::new(text_width, uri_height);
            let uri_y = path_y + path_height as f64;
            text_options.position(Position::new(close_position.y, uri_y));
            text_options.size(uri_size);
            text_options.text_color(config.colors.alt_foreground.as_f64());
            builder.rasterize(&uri_layout, &text_options);

            // Render close `X`.
            let fg = config.colors.foreground.as_f64();
            let size = Downloads::close_entry_button_size(entry_size, scale);
            let context = builder.context();
            context.move_to(close_position.x, close_position.y);
            context.line_to(close_position.x + size.width, close_position.y + size.height);
            context.move_to(close_position.x + size.width, close_position.y);
            context.line_to(close_position.x, close_position.y + size.height);
            context.set_source_rgb(fg[0], fg[1], fg[2]);
            context.set_line_width(scale);
            context.stroke().unwrap();

            builder.build()
        })
    }

    /// Get the number of entries.
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Hash key for the downloads texture cache.
#[derive(Hash, PartialEq, Eq, Copy, Clone)]
struct TextureCacheKey {
    colors: Colors,
    id: DownloadId,
    progress: u8,
    failed: bool,
}

/// Touch event tracking.
#[derive(Default)]
struct TouchState {
    slot: Option<i32>,
    action: TouchAction,
    start: Position<f64>,
    position: Position<f64>,
}

/// Intention of a touch sequence.
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug)]
enum TouchAction {
    #[default]
    EntryTap,
    EntryDrag,
    CloseTap,
    DeleteTap,
}

/// Browser file download.
pub struct Download {
    pub id: DownloadId,
    pub destination: String,
    pub progress: u8,
    pub failed: bool,
    pub uri: String,
}

/// Unique identifier for a file download.
#[derive(Hash, PartialEq, Eq, Copy, Clone, Debug)]
pub struct DownloadId {
    engine_id: EngineId,
    id: usize,
}

impl DownloadId {
    pub fn new(engine_id: EngineId) -> Self {
        static NEXT_DOWNLOAD_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed);
        Self { engine_id, id }
    }

    /// Get the download's origin engine ID.
    pub fn engine_id(&self) -> EngineId {
        self.engine_id
    }

    /// Get the download's window ID.
    pub fn window_id(&self) -> WindowId {
        self.engine_id.window_id()
    }
}
