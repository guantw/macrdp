use anyhow::Result;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::collections::BTreeMap;

use crate::keymap::scancode_to_keycode;
use crate::KeyboardMappings;

#[derive(Clone, Copy)]
struct HeldKey {
    keycode: u16,
    modifiers: u8,
    consumed: u8,
}

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

fn source_modifier(scancode: u16) -> u8 {
    match scancode {
        0x2A | 0x36 => 0x01,
        0x1D | 0x11D => 0x02,
        0x38 | 0x138 => 0x04,
        0x15B | 0x15C => 0x08,
        _ => 0,
    }
}

#[derive(Default)]
pub struct KeyboardInjector {
    modifiers: u8,
    caps_lock: bool,
    caps_lock_down: bool,
    held: BTreeMap<u16, HeldKey>,
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
        self.inject_mapped_key(scancode, extended, pressed, &KeyboardMappings::default())
    }

    pub fn inject_mapped_key(
        &mut self,
        scancode: u8,
        extended: bool,
        pressed: bool,
        mappings: &KeyboardMappings,
    ) -> Result<()> {
        for event in self.mapped_events(scancode, extended, pressed, mappings)? {
            event.post(CGEventTapLocation::HID);
        }
        tracing::trace!(scancode, extended, pressed, "Keyboard event injected");
        Ok(())
    }

    fn mapped_events(
        &mut self,
        scancode: u8,
        extended: bool,
        pressed: bool,
        mappings: &KeyboardMappings,
    ) -> Result<Vec<CGEvent>> {
        let Some(default_keycode) = scancode_to_keycode(scancode, extended) else {
            tracing::warn!(scancode, extended, "Unknown scancode, ignoring");
            return Ok(Vec::new());
        };
        let source = u16::from(scancode) | if extended { 0x100 } else { 0 };
        let repeated = self.held.contains_key(&source);
        let mut events = Vec::new();

        if pressed {
            // Resolve only on the initial press. Repeats and releases use that
            // same result even after a config change or an early modifier release.
            if !repeated {
                let source_modifiers = self
                    .held
                    .keys()
                    .fold(0, |mask, code| mask | source_modifier(*code));
                let rule = mappings
                    .compiled
                    .iter()
                    .find(|rule| {
                        rule.source == source
                            && rule.modifiers != 0
                            && rule.modifiers == source_modifiers
                    })
                    .or_else(|| {
                        mappings
                            .compiled
                            .iter()
                            .find(|rule| rule.source == source && rule.modifiers == 0)
                    });
                self.held.insert(
                    source,
                    rule.map_or(
                        HeldKey {
                            keycode: default_keycode,
                            modifiers: 0,
                            consumed: 0,
                        },
                        |rule| HeldKey {
                            keycode: rule.target,
                            modifiers: rule.target_modifiers,
                            consumed: rule.modifiers,
                        },
                    ),
                );
            }
            self.sync_modifiers(&mut events)?;
            let key = self.held[&source];
            if modifier_bit(key.keycode) == 0 {
                let event = self.key_event(key.keycode, true)?;
                event.set_integer_value_field(
                    EventField::KEYBOARD_EVENT_AUTOREPEAT,
                    i64::from(repeated),
                );
                events.push(event);
            }
        } else if let Some(key) = self.held.get(&source).copied() {
            // Keep another physical key mapped to the same output held down.
            if modifier_bit(key.keycode) == 0
                && !self
                    .held
                    .iter()
                    .any(|(code, held)| *code != source && held.keycode == key.keycode)
            {
                events.push(self.key_event(key.keycode, false)?);
            }
            self.held.remove(&source);
            self.sync_modifiers(&mut events)?;
        }
        Ok(events)
    }

    fn sync_modifiers(&mut self, events: &mut Vec<CGEvent>) -> Result<()> {
        let consumed = self.held.values().fold(0, |mask, key| mask | key.consumed);
        let wanted = self.held.iter().fold(0, |mask, (source, key)| {
            if source_modifier(*source) & consumed != 0 {
                mask
            } else {
                mask | key.modifiers | modifier_bit(key.keycode)
            }
        });
        // Release consumed source modifiers before pressing target modifiers.
        for pressed in [false, true] {
            for (keycode, bit) in MODIFIER_KEYS {
                if (wanted & bit != 0) == pressed && (self.modifiers & bit != 0) != pressed {
                    events.push(self.key_event(keycode, pressed)?);
                }
            }
        }
        Ok(())
    }

    fn release_events(&mut self) -> Result<Vec<CGEvent>> {
        let mut events = Vec::new();
        let held = std::mem::take(&mut self.held);
        let keys: std::collections::BTreeSet<_> = held.values().map(|key| key.keycode).collect();
        for keycode in keys {
            if modifier_bit(keycode) == 0 {
                events.push(self.key_event(keycode, false)?);
            }
        }
        self.sync_modifiers(&mut events)?;
        self.caps_lock = false;
        self.caps_lock_down = false;
        Ok(events)
    }

    /// Stop held output when entering/leaving the recorder without changing lock state.
    pub fn release_for_recording(&mut self) -> Result<()> {
        let caps_lock = self.caps_lock;
        let result = self.release_all();
        self.caps_lock = caps_lock;
        result
    }

    /// Release injected keys when a connection ends, including mapped modifiers.
    pub fn release_all(&mut self) -> Result<()> {
        for event in self.release_events()? {
            event.post(CGEventTapLocation::HID);
        }
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
    use crate::{KeyChord, KeyMapping, Modifier};

    fn mapping(
        from: &str,
        from_modifiers: &[Modifier],
        to: &str,
        to_modifiers: &[Modifier],
    ) -> KeyMapping {
        KeyMapping {
            from: KeyChord {
                key: from.into(),
                modifiers: from_modifiers.to_vec(),
            },
            to: KeyChord {
                key: to.into(),
                modifiers: to_modifiers.to_vec(),
            },
        }
    }

    fn codes(events: &[CGEvent]) -> Vec<i64> {
        events
            .iter()
            .map(|event| event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
            .collect()
    }

    fn modifiers(event: &CGEvent) -> CGEventFlags {
        event.get_flags()
            & (CGEventFlags::CGEventFlagShift
                | CGEventFlags::CGEventFlagControl
                | CGEventFlags::CGEventFlagAlternate
                | CGEventFlags::CGEventFlagCommand)
    }

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

    #[test]
    fn empty_rules_keep_the_default_alt_and_win_mapping() {
        let mut keyboard = KeyboardInjector::default();
        let mappings = KeyboardMappings::default();
        for (scan, extended, keycode, flag) in [
            (0x38, false, 0x37, CGEventFlags::CGEventFlagCommand),
            (0x38, true, 0x36, CGEventFlags::CGEventFlagCommand),
            (0x5B, true, 0x3A, CGEventFlags::CGEventFlagAlternate),
            (0x5C, true, 0x3D, CGEventFlags::CGEventFlagAlternate),
        ] {
            let down = keyboard
                .mapped_events(scan, extended, true, &mappings)
                .unwrap();
            assert_eq!(codes(&down), [keycode]);
            assert_eq!(modifiers(&down[0]), flag);
            let up = keyboard
                .mapped_events(scan, extended, false, &mappings)
                .unwrap();
            assert_eq!(codes(&up), [keycode]);
            assert_eq!(modifiers(&up[0]), CGEventFlags::empty());
        }
    }

    #[test]
    fn modifier_remaps_are_directional_and_do_not_chain() {
        let mappings = KeyboardMappings::try_from(vec![
            mapping("left_control", &[], "left_meta", &[]),
            mapping("left_meta", &[], "left_control", &[]),
            mapping("left_alt", &[], "left_alt", &[]),
        ])
        .unwrap();
        let mut keyboard = KeyboardInjector::default();
        for (scan, extended, keycode, flag) in [
            (0x1D, false, 0x37, CGEventFlags::CGEventFlagCommand),
            (0x5B, true, 0x3B, CGEventFlags::CGEventFlagControl),
            (0x38, false, 0x3A, CGEventFlags::CGEventFlagAlternate),
            (0x1D, true, 0x3E, CGEventFlags::CGEventFlagControl),
        ] {
            assert_eq!(
                codes(
                    &keyboard
                        .mapped_events(scan, extended, true, &mappings)
                        .unwrap()
                ),
                [keycode]
            );
            for (scan, keycode) in [(0x2E, 0x08), (0x2F, 0x09)] {
                let down = keyboard.mapped_events(scan, false, true, &mappings).unwrap();
                assert_eq!(codes(&down), [keycode]);
                assert_eq!(modifiers(&down[0]), flag);
                let up = keyboard.mapped_events(scan, false, false, &mappings).unwrap();
                assert_eq!(codes(&up), [keycode]);
                assert_eq!(modifiers(&up[0]), flag);
            }
            keyboard
                .mapped_events(scan, extended, false, &mappings)
                .unwrap();
        }

        // Remapping Alt alone must not change Ctrl+V into Command+V.
        let mappings = KeyboardMappings::try_from(vec![
            mapping("left_alt", &[], "left_control", &[]),
        ]).unwrap();
        for scan in [0x1D, 0x38] {
            keyboard.mapped_events(scan, false, true, &mappings).unwrap();
            let down = keyboard.mapped_events(0x2F, false, true, &mappings).unwrap();
            assert_eq!(codes(&down), [0x09]);
            assert_eq!(modifiers(&down[0]), CGEventFlags::CGEventFlagControl);
            keyboard.mapped_events(0x2F, false, false, &mappings).unwrap();
            keyboard.mapped_events(scan, false, false, &mappings).unwrap();
        }
    }

    #[test]
    fn exact_chords_override_single_keys_and_match_original_windows_modifiers() {
        let mappings = KeyboardMappings::try_from(vec![
            mapping("left_control", &[], "left_meta", &[]),
            mapping("home", &[], "left", &[Modifier::Meta]),
            mapping(
                "home",
                &[Modifier::Control],
                "right",
                &[Modifier::Shift, Modifier::Meta],
            ),
        ])
        .unwrap();
        let mut keyboard = KeyboardInjector::default();
        keyboard
            .mapped_events(0x1D, false, true, &mappings)
            .unwrap();
        let home = keyboard.mapped_events(0x47, true, true, &mappings).unwrap();
        assert_eq!(*codes(&home).last().unwrap(), 0x7C); // right, not the plain Home rule
        assert_eq!(
            modifiers(home.last().unwrap()),
            CGEventFlags::CGEventFlagShift | CGEventFlags::CGEventFlagCommand
        );
        keyboard
            .mapped_events(0x47, true, false, &mappings)
            .unwrap();
        keyboard
            .mapped_events(0x1D, false, false, &mappings)
            .unwrap();

        keyboard
            .mapped_events(0x2A, false, true, &mappings)
            .unwrap();
        let home = keyboard.mapped_events(0x47, true, true, &mappings).unwrap();
        assert_eq!(*codes(&home).last().unwrap(), 0x7B); // fallback retains Shift for selection
        assert_eq!(
            modifiers(home.last().unwrap()),
            CGEventFlags::CGEventFlagShift | CGEventFlags::CGEventFlagCommand
        );
        keyboard
            .mapped_events(0x47, true, false, &mappings)
            .unwrap();
        let letter = keyboard
            .mapped_events(0x12, false, true, &mappings)
            .unwrap();
        assert_eq!(modifiers(&letter[0]), CGEventFlags::CGEventFlagShift);
    }

    #[test]
    fn shortcuts_consume_modifiers_repeat_and_release_in_either_order() {
        let mappings = KeyboardMappings::try_from(vec![mapping(
            "insert",
            &[Modifier::Shift],
            "v",
            &[Modifier::Meta],
        )])
        .unwrap();
        for release_shift_first in [false, true] {
            let mut keyboard = KeyboardInjector::default();
            keyboard
                .mapped_events(0x2A, false, true, &mappings)
                .unwrap();
            let down = keyboard.mapped_events(0x52, true, true, &mappings).unwrap();
            assert_eq!(codes(&down), [0x38, 0x37, 0x09]); // Shift up, Command down, V down
            assert_eq!(modifiers(&down[2]), CGEventFlags::CGEventFlagCommand);
            let repeated = keyboard.mapped_events(0x52, true, true, &mappings).unwrap();
            assert_eq!(codes(&repeated), [0x09]);
            assert_eq!(
                repeated[0].get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
                1
            );
            if release_shift_first {
                assert!(keyboard
                    .mapped_events(0x2A, false, false, &mappings)
                    .unwrap()
                    .is_empty());
            }
            let up = keyboard
                .mapped_events(0x52, true, false, &mappings)
                .unwrap();
            assert_eq!(codes(&up)[0], 0x09);
            assert_eq!(modifiers(&up[0]), CGEventFlags::CGEventFlagCommand);
            if !release_shift_first {
                assert_eq!(
                    modifiers(up.last().unwrap()),
                    CGEventFlags::CGEventFlagShift
                );
                keyboard
                    .mapped_events(0x2A, false, false, &mappings)
                    .unwrap();
            }
            let letter = keyboard
                .mapped_events(0x12, false, true, &mappings)
                .unwrap();
            assert_eq!(modifiers(&letter[0]), CGEventFlags::empty());
        }
    }

    #[test]
    fn shared_output_stays_pressed_until_all_source_keys_are_released() {
        let mappings =
            KeyboardMappings::try_from(vec![mapping("left_control", &[], "left_meta", &[])])
                .unwrap();
        let mut keyboard = KeyboardInjector::default();
        keyboard
            .mapped_events(0x1D, false, true, &mappings)
            .unwrap();
        assert!(keyboard
            .mapped_events(0x38, false, true, &mappings)
            .unwrap()
            .is_empty());
        assert!(keyboard
            .mapped_events(0x1D, false, false, &mappings)
            .unwrap()
            .is_empty());
        let c = keyboard
            .mapped_events(0x2E, false, true, &mappings)
            .unwrap();
        assert_eq!(modifiers(&c[0]), CGEventFlags::CGEventFlagCommand);
        let up = keyboard
            .mapped_events(0x38, false, false, &mappings)
            .unwrap();
        assert_eq!(codes(&up), [0x37]);
        assert_eq!(modifiers(&up[0]), CGEventFlags::empty());
    }

    #[test]
    fn hot_updates_preserve_held_outputs_and_disconnect_releases_them() {
        let first =
            KeyboardMappings::try_from(vec![mapping("home", &[], "left", &[Modifier::Meta])])
                .unwrap();
        let second =
            KeyboardMappings::try_from(vec![mapping("home", &[], "right", &[Modifier::Alt])])
                .unwrap();
        let mut keyboard = KeyboardInjector::default();
        keyboard.mapped_events(0x47, true, true, &first).unwrap();
        let repeat = keyboard.mapped_events(0x47, true, true, &second).unwrap();
        assert_eq!(codes(&repeat), [0x7B]);
        assert_eq!(modifiers(&repeat[0]), CGEventFlags::CGEventFlagCommand);
        let released = keyboard.mapped_events(0x47, true, false, &second).unwrap();
        assert_eq!(codes(&released), [0x7B, 0x37]);
        let next = keyboard.mapped_events(0x47, true, true, &second).unwrap();
        assert_eq!(codes(&next), [0x3A, 0x7C]);
        // Deleting the mapping also takes effect in this same input session.
        let defaults = KeyboardMappings::default();
        let release_after_delete = keyboard.mapped_events(0x47, true, false, &defaults).unwrap();
        assert_eq!(codes(&release_after_delete), [0x7C, 0x3A]);
        let unmapped = keyboard.mapped_events(0x47, true, true, &defaults).unwrap();
        assert_eq!(codes(&unmapped), [0x73]);
        keyboard.mapped_events(0x47, true, false, &defaults).unwrap();
        keyboard.mapped_events(0x47, true, true, &second).unwrap();
        let disconnected = keyboard.release_events().unwrap();
        assert_eq!(codes(&disconnected), [0x7C, 0x3A]);
        assert_eq!(
            modifiers(disconnected.last().unwrap()),
            CGEventFlags::empty()
        );
        assert!(keyboard.release_events().unwrap().is_empty());
        let next_session = keyboard.mapped_events(0x12, false, true, &second).unwrap();
        assert_eq!(modifiers(&next_session[0]), CGEventFlags::empty());
    }
}
