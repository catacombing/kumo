//! IME subclass implementation.

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
    use wpe_platform::{
        InputHints, InputMethodContextExt, InputMethodContextImpl, InputPurpose, PreeditString,
    };

    use crate::window::{TextInputChange, TextInputState};

    #[derive(Default)]
    pub struct InputMethodContext {
        text_input_state: Cell<Option<TextInputState>>,
        preedit_string: Cell<Option<PreeditString>>,
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
            let webkit_context = context_obj.upcast_ref::<wpe_platform::InputMethodContext>();
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

        /// Update the current preedit text.
        pub fn set_preedit_string(&self, text: String, cursor_begin: i32, cursor_end: i32) {
            let preedit_string = PreeditString { text, cursor_begin, cursor_end };
            self.preedit_string.replace(Some(preedit_string));
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
        fn focus_in(&self) {
            self.text_input_state.set(Some(TextInputState::default()));
            self.preedit_string.replace(None);

            self.dirty.store(true, Ordering::Relaxed);
        }

        fn focus_out(&self) {
            self.text_input_state.take();

            self.dirty.store(true, Ordering::Relaxed);
        }

        fn set_cursor_area(&self, x: i32, y: i32, width: i32, height: i32) {
            if let Some(mut text_input_state) = self.text_input_state.take() {
                text_input_state.cursor_rect = (x, y, width, height);
                self.text_input_state.set(Some(text_input_state));

                self.dirty.store(true, Ordering::Relaxed);
            }
        }

        fn set_surrounding(&self, text: &str, cursor_index: u32, selection_index: u32) {
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

        fn preedit_string(&self) -> Option<PreeditString> {
            let preedit_string = self.preedit_string.take();
            self.preedit_string.set(preedit_string.clone());
            preedit_string
        }

        fn reset(&self) {
            if self.text_input_state.take().is_some() {
                self.focus_in();
            }
        }
    }

    impl ObjectImpl for InputMethodContext {}

    #[glib::object_subclass]
    impl ObjectSubclass for InputMethodContext {
        type ParentType = wpe_platform::InputMethodContext;
        type Type = super::InputMethodContext;

        const NAME: &'static str = "KumoWebKitInputMethodContext";
    }
}

glib::wrapper! {
    pub struct InputMethodContext(ObjectSubclass<imp::InputMethodContext>)
        @extends wpe_platform::InputMethodContext;
}

impl InputMethodContext {
    pub fn new() -> Self {
        Object::new()
    }

    /// Get current IME text_input state.
    pub fn text_input_state(&self) -> TextInputChange {
        self.imp().text_input_state()
    }

    /// Update the current preedit text.
    pub fn set_preedit_string(&self, text: String, cursor_begin: i32, cursor_end: i32) {
        self.imp().set_preedit_string(text, cursor_begin, cursor_end)
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
