//! IME handler.

use glib::subclass::types::ObjectSubclassIsExt;
use glib::Object;

use crate::window::TextInputChange;

mod imp {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    use _text_input::zwp_text_input_v3::{ContentHint, ContentPurpose};
    use glib::object::Cast;
    use glib::subclass::prelude::*;
    use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
    use wpe_webkit::{InputHints, InputMethodContextExt, InputMethodContextImpl, InputPurpose};

    use crate::window::{TextInputChange, TextInputState};

    #[derive(Default)]
    pub struct InputMethodContext {
        text_input_state: Cell<Option<TextInputState>>,
        dirty: AtomicBool,
    }

    impl InputMethodContext {
        /// Get current IME text_input state.
        pub fn text_input_state(&self) -> TextInputChange {
            // Skip expensive surrounding_text clone without changes.
            if !self.dirty.fetch_and(false, Ordering::Relaxed) {
                return TextInputChange::Unchanged;
            }

            let mut text_input_state = match self.text_input_state.take() {
                Some(text_input_state) => text_input_state,
                None => return TextInputChange::Disabled,
            };
            self.text_input_state.set(Some(text_input_state.clone()));

            // Map WebKit purpose to Wayland purpose.
            let context_obj = self.obj();
            let webkit_context =
                unsafe { context_obj.unsafe_cast_ref::<wpe_webkit::InputMethodContext>() };
            text_input_state.purpose = match webkit_context.input_purpose() {
                InputPurpose::Digits => ContentPurpose::Digits,
                InputPurpose::Number => ContentPurpose::Number,
                InputPurpose::Phone => ContentPurpose::Phone,
                InputPurpose::Url => ContentPurpose::Url,
                InputPurpose::Email => ContentPurpose::Email,
                InputPurpose::Password => ContentPurpose::Password,
                _ => ContentPurpose::Normal,
            };

            // Map WebKit hint to Wayland hint.
            let wpe_hints = webkit_context.input_hints();
            let mut hint = ContentHint::None;
            hint.set(ContentHint::Spellcheck, wpe_hints.contains(InputHints::SPELLCHECK));
            hint.set(ContentHint::Lowercase, wpe_hints.contains(InputHints::LOWERCASE));
            let wpe_uppercase = InputHints::UPPERCASE_CHARS
                | InputHints::UPPERCASE_WORDS
                | InputHints::UPPERCASE_SENTENCES;
            hint.set(ContentHint::Uppercase, wpe_hints.contains(wpe_uppercase));
            text_input_state.hint = hint;

            TextInputChange::Dirty(text_input_state)
        }

        /// Mark text input state as dirty.
        ///
        /// This is useful to force a resubmission when switching between
        /// surfaces.
        pub fn mark_dirty(&self) {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    impl InputMethodContextImpl for InputMethodContext {
        fn notify_focus_in(&self) {
            self.text_input_state.set(Some(TextInputState::default()));

            self.dirty.store(true, Ordering::Relaxed);
        }

        fn notify_focus_out(&self) {
            self.text_input_state.take();

            self.dirty.store(true, Ordering::Relaxed);
        }

        fn notify_cursor_area(&self, x: i32, y: i32, width: i32, height: i32) {
            if let Some(mut text_input_state) = self.text_input_state.take() {
                text_input_state.cursor_rect = (x, y, width, height);
                self.text_input_state.set(Some(text_input_state));

                self.dirty.store(true, Ordering::Relaxed);
            }
        }

        fn notify_surrounding(&self, text: &str, cursor_index: u32, selection_index: u32) {
            let selection_index = selection_index as i32;
            let cursor_index = cursor_index as i32;
            if let Some(mut text_input_state) = self.text_input_state.take() {
                text_input_state.surrounding_text = text.into();
                text_input_state.cursor_index = cursor_index;
                if selection_index != cursor_index {
                    text_input_state.selection = Some(cursor_index..selection_index);
                }
                self.text_input_state.set(Some(text_input_state));

                self.dirty.store(true, Ordering::Relaxed);
            }
        }

        fn reset(&self) {
            if self.text_input_state.take().is_some() {
                self.text_input_state.set(Some(TextInputState::default()));

                self.dirty.store(true, Ordering::Relaxed);
            }
        }
    }

    impl ObjectImpl for InputMethodContext {}

    #[glib::object_subclass]
    impl ObjectSubclass for InputMethodContext {
        type ParentType = wpe_webkit::InputMethodContext;
        type Type = super::InputMethodContext;

        const NAME: &'static str = "WebkitInputMethodContext";
    }
}

glib::wrapper! {
    pub struct InputMethodContext(ObjectSubclass<imp::InputMethodContext>)
        @extends wpe_webkit::InputMethodContext;
}

impl InputMethodContext {
    pub fn new() -> Self {
        Object::new()
    }

    /// Get current IME text_input state.
    pub fn text_input_state(&self) -> TextInputChange {
        self.imp().text_input_state()
    }

    /// Mark text input state as dirty.
    ///
    /// This is useful to force a resubmission when switching between
    /// surfaces.
    pub fn mark_text_input_dirty(&self) {
        self.imp().mark_dirty()
    }
}

impl Default for InputMethodContext {
    fn default() -> Self {
        Self::new()
    }
}
