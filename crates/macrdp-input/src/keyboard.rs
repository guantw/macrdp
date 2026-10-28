use anyhow::Result;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use crate::keymap::scancode_to_keycode;

const MODIFIER_KEYS: [(u16, u8); 8] = [
    (0x38, 0x01),
    (0x3C, 0x02), // Shift
    (0x3B, 0x04),
    (0x3E, 0x08), // Control
    (0x3A, 0x10),
    (0x3D, 0x20), // Option
    (0x37, 0x40),
    (0x36, 0x80), // Command
];

fn modifier_bit(keycode: u16) -> u8 {
    MODIFIER_KEYS
        .iter()
        .find(|(key, _)| *key == keycode)
        .map_or(0, |(_, bit)| *bit)
}

#[derive(Default)]
pub struct KeyboardInjector {
    modifiers: u8,
    caps_lock: bool,
    caps_lock_down: bool,
}

impl KeyboardInjector {
    pub fn new() -> Result<Self> {
        // Verify we can create an event source (permission check)
        let _ = Self::source().map_err(|_| {
            anyhow::anyhow!("Failed to create CGEventSource — check Accessibility permission")
        })?;
        Ok(Self::default())
    }

    fn source() -> Result<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| anyhow::anyhow!("Failed to create CGEventSource"))
    }

    // Build without posting so modifier handling can be checked without OS input.
    fn key_event(&mut self, keycode: CGKeyCode, pressed: bool) -> Result<CGEvent> {
        let source = Self::source()?;
        let event = CGEvent::new_keyboard_event(source, keycode, pressed)
            .map_err(|_| anyhow::anyhow!("Failed to create keyboard event"))?;

        let modifier = modifier_bit(keycode);
        if pressed {
            self.modifiers |= modifier;
        } else {
            self.modifiers &= !modifier;
        }
        if keycode == 0x39 {
            if pressed && !self.caps_lock_down {
                self.caps_lock = !self.caps_lock;
            }
            self.caps_lock_down = pressed;
        }

        // Never inherit HID state (including Fn) from a local or earlier event.
        // Track each side separately so releasing one Shift preserves the other.
        // Private sources start with only this key's native function/keypad tags.
        let mut flags = event.get_flags()
            & (CGEventFlags::CGEventFlagSecondaryFn
                | CGEventFlags::CGEventFlagNumericPad
                | CGEventFlags::CGEventFlagHelp);
        flags.set(CGEventFlags::CGEventFlagShift, self.modifiers & 0x03 != 0);
        flags.set(CGEventFlags::CGEventFlagControl, self.modifiers & 0x0C != 0);
        flags.set(
            CGEventFlags::CGEventFlagAlternate,
            self.modifiers & 0x30 != 0,
        );
        flags.set(CGEventFlags::CGEventFlagCommand, self.modifiers & 0xC0 != 0);
        flags.set(CGEventFlags::CGEventFlagAlphaShift, self.caps_lock);
        event.set_flags(flags);
        Ok(event)
    }

    pub fn synchronize(&mut self, caps_lock: bool) {
        self.caps_lock = caps_lock;
    }

    /// Inject a key press or release event
    pub fn inject_key(&mut self, scancode: u8, extended: bool, pressed: bool) -> Result<()> {
        let keycode = match scancode_to_keycode(scancode, extended) {
            Some(kc) => kc,
            None => {
                tracing::warn!(scancode, extended, "Unknown scancode, ignoring");
                return Ok(());
            }
        };

        let event = self.key_event(keycode as CGKeyCode, pressed)?;

        event.post(CGEventTapLocation::HID);
        tracing::trace!(scancode, keycode, pressed, "Keyboard event injected");
        Ok(())
    }

    /// Inject a unicode character press/release
    pub fn inject_unicode(&mut self, ch: u16, pressed: bool) -> Result<()> {
        let event = self.key_event(0, pressed)?;

        if pressed {
            event.set_string_from_utf16_unchecked(&[ch]);
        }

        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only construct and inspect CGEvents. None of these tests post input.
    #[test]
    fn ordinary_letters_do_not_inherit_fn_or_other_modifiers() {
        let mut keyboard = KeyboardInjector::default();
        for (scan, keycode) in [(0x12, 0x0E), (0x20, 0x02), (0x21, 0x03)] {
            assert_eq!(scancode_to_keycode(scan, false), Some(keycode));
            for pressed in [true, false] {
                let event = keyboard.key_event(keycode, pressed).unwrap();
                assert_eq!(event.get_flags(), CGEventFlags::empty());
                assert_eq!(
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                    i64::from(keycode)
                );
            }
        }
        // Function/navigation events must not leave Fn attached to the next letter.
        for (keycode, tags) in [
            (0x7A, CGEventFlags::CGEventFlagSecondaryFn),
            (
                0x7E,
                CGEventFlags::CGEventFlagSecondaryFn | CGEventFlags::CGEventFlagNumericPad,
            ),
            (0x73, CGEventFlags::CGEventFlagSecondaryFn),
            (0x52, CGEventFlags::CGEventFlagNumericPad),
        ] {
            assert_eq!(keyboard.key_event(keycode, true).unwrap().get_flags(), tags);
            keyboard.key_event(keycode, false).unwrap();
            assert_eq!(
                keyboard.key_event(0x0E, true).unwrap().get_flags(),
                CGEventFlags::empty()
            );
        }
    }

    #[test]
    fn modifiers_follow_remote_press_release_and_preserve_either_side() {
        let mut keyboard = KeyboardInjector::default();
        for (left, right, flag) in [
            (0x38, 0x3C, CGEventFlags::CGEventFlagShift),
            (0x3B, 0x3E, CGEventFlags::CGEventFlagControl),
            (0x3A, 0x3D, CGEventFlags::CGEventFlagAlternate),
            (0x37, 0x36, CGEventFlags::CGEventFlagCommand),
        ] {
            assert_eq!(keyboard.key_event(left, true).unwrap().get_flags(), flag);
            keyboard.key_event(right, true).unwrap();
            keyboard.key_event(left, false).unwrap();
            assert_eq!(keyboard.key_event(0x0E, true).unwrap().get_flags(), flag);
            assert_eq!(keyboard.key_event(0x0E, false).unwrap().get_flags(), flag);
            assert_eq!(
                keyboard.key_event(right, false).unwrap().get_flags(),
                CGEventFlags::empty()
            );
        }
        keyboard.key_event(0x3B, true).unwrap();
        keyboard.key_event(0x37, true).unwrap();
        assert_eq!(
            keyboard.key_event(0x03, true).unwrap().get_flags(),
            CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagCommand
        );
    }

    #[test]
    fn caps_lock_toggles_and_accepts_client_synchronization() {
        let mut keyboard = KeyboardInjector::default();
        keyboard.key_event(0x39, true).unwrap();
        keyboard.key_event(0x39, true).unwrap(); // key repeat
        keyboard.key_event(0x39, false).unwrap();
        assert_eq!(
            keyboard.key_event(0x0E, true).unwrap().get_flags(),
            CGEventFlags::CGEventFlagAlphaShift
        );
        keyboard.key_event(0x39, true).unwrap();
        keyboard.key_event(0x39, false).unwrap();
        assert_eq!(
            keyboard.key_event(0x0E, true).unwrap().get_flags(),
            CGEventFlags::empty()
        );
        keyboard.synchronize(true);
        assert_eq!(
            keyboard.key_event(0x0E, true).unwrap().get_flags(),
            CGEventFlags::CGEventFlagAlphaShift
        );
        keyboard.synchronize(false);
        assert_eq!(
            keyboard.key_event(0x0E, true).unwrap().get_flags(),
            CGEventFlags::empty()
        );
    }

}
