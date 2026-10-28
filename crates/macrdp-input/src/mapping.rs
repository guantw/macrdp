//! User mappings are matched against RDP keys and produce native macOS keys.
//! The default scancode map remains separate and is used for unmatched input.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Shift,
    Control,
    Alt,
    Meta,
}

impl Modifier {
    pub(crate) fn mask(self) -> u8 {
        match self {
            Self::Shift => 0x01,
            Self::Control => 0x02,
            Self::Alt => 0x04,
            Self::Meta => 0x08,
        }
    }

    fn mac_mask(self) -> u8 {
        match self {
            Self::Shift => 0x01,
            Self::Control => 0x04,
            Self::Alt => 0x10,
            Self::Meta => 0x40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMapping {
    pub from: KeyChord,
    pub to: KeyChord,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledMapping {
    pub source: u16,
    pub modifiers: u8,
    pub target: u16,
    pub target_modifiers: u8,
}

/// A validated mapping list. JSON and TOML both enter through the same boundary.
#[derive(Debug, Clone, Default)]
pub struct KeyboardMappings {
    rules: Vec<KeyMapping>,
    pub(crate) compiled: Vec<CompiledMapping>,
}

impl TryFrom<Vec<KeyMapping>> for KeyboardMappings {
    type Error = String;

    fn try_from(rules: Vec<KeyMapping>) -> Result<Self, Self::Error> {
        let mut sources = HashSet::new();
        let mut compiled = Vec::with_capacity(rules.len());
        for (index, rule) in rules.iter().enumerate() {
            let source = KEY_OPTIONS
                .iter()
                .find(|key| key.id == rule.from.key)
                .ok_or_else(|| {
                    format!("Mapping {}: unknown input key {}", index + 1, rule.from.key)
                })?;
            let target = KEY_OPTIONS
                .iter()
                .find(|key| key.id == rule.to.key)
                .ok_or_else(|| {
                    format!("Mapping {}: unknown output key {}", index + 1, rule.to.key)
                })?;
            let modifiers = rule
                .from
                .modifiers
                .iter()
                .fold(0, |mask, modifier| mask | modifier.mask());
            if !sources.insert((source.scancode, modifiers)) {
                return Err(format!(
                    "Mapping {}: this input already has a mapping",
                    index + 1
                ));
            }
            compiled.push(CompiledMapping {
                source: source.scancode,
                modifiers,
                target: target.keycode,
                target_modifiers: rule
                    .to
                    .modifiers
                    .iter()
                    .fold(0, |mask, modifier| mask | modifier.mac_mask()),
            });
        }
        Ok(Self { rules, compiled })
    }
}

impl Serialize for KeyboardMappings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.rules.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for KeyboardMappings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<KeyMapping>::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A single catalog supplies both the configuration UI and mapping validation.
/// Scancodes include the extended bit; output keycodes are native Mac identities,
/// so selecting Option explicitly does not invoke the default Alt -> Command map.
#[derive(Debug, Serialize)]
pub struct KeyOption {
    pub id: &'static str,
    pub windows: &'static str,
    pub mac: &'static str,
    pub group: &'static str,
    #[serde(skip)]
    pub scancode: u16,
    #[serde(skip)]
    pub(crate) keycode: u16,
}

const fn key(
    id: &'static str,
    windows: &'static str,
    mac: &'static str,
    group: &'static str,
    scancode: u16,
    keycode: u16,
) -> KeyOption {
    KeyOption {
        id,
        windows,
        mac,
        group,
        scancode,
        keycode,
    }
}

pub const KEY_OPTIONS: &[KeyOption] = &[
    key(
        "left_control",
        "左 Ctrl",
        "左 Control",
        "修饰键",
        0x1D,
        0x3B,
    ),
    key(
        "right_control",
        "右 Ctrl",
        "右 Control",
        "修饰键",
        0x11D,
        0x3E,
    ),
    key("left_alt", "左 Alt", "左 Option", "修饰键", 0x38, 0x3A),
    key("right_alt", "右 Alt", "右 Option", "修饰键", 0x138, 0x3D),
    key("left_meta", "左 Win", "左 Command", "修饰键", 0x15B, 0x37),
    key("right_meta", "右 Win", "右 Command", "修饰键", 0x15C, 0x36),
    key("left_shift", "左 Shift", "左 Shift", "修饰键", 0x2A, 0x38),
    key("right_shift", "右 Shift", "右 Shift", "修饰键", 0x36, 0x3C),
    key("a", "A", "A", "字母", 0x1E, 0x00),
    key("b", "B", "B", "字母", 0x30, 0x0B),
    key("c", "C", "C", "字母", 0x2E, 0x08),
    key("d", "D", "D", "字母", 0x20, 0x02),
    key("e", "E", "E", "字母", 0x12, 0x0E),
    key("f", "F", "F", "字母", 0x21, 0x03),
    key("g", "G", "G", "字母", 0x22, 0x05),
    key("h", "H", "H", "字母", 0x23, 0x04),
    key("i", "I", "I", "字母", 0x17, 0x22),
    key("j", "J", "J", "字母", 0x24, 0x26),
    key("k", "K", "K", "字母", 0x25, 0x28),
    key("l", "L", "L", "字母", 0x26, 0x25),
    key("m", "M", "M", "字母", 0x32, 0x2E),
    key("n", "N", "N", "字母", 0x31, 0x2D),
    key("o", "O", "O", "字母", 0x18, 0x1F),
    key("p", "P", "P", "字母", 0x19, 0x23),
    key("q", "Q", "Q", "字母", 0x10, 0x0C),
    key("r", "R", "R", "字母", 0x13, 0x0F),
    key("s", "S", "S", "字母", 0x1F, 0x01),
    key("t", "T", "T", "字母", 0x14, 0x11),
    key("u", "U", "U", "字母", 0x16, 0x20),
    key("v", "V", "V", "字母", 0x2F, 0x09),
    key("w", "W", "W", "字母", 0x11, 0x0D),
    key("x", "X", "X", "字母", 0x2D, 0x07),
    key("y", "Y", "Y", "字母", 0x15, 0x10),
    key("z", "Z", "Z", "字母", 0x2C, 0x06),
    key("0", "0", "0", "数字", 0x0B, 0x1D),
    key("1", "1", "1", "数字", 0x02, 0x12),
    key("2", "2", "2", "数字", 0x03, 0x13),
    key("3", "3", "3", "数字", 0x04, 0x14),
    key("4", "4", "4", "数字", 0x05, 0x15),
    key("5", "5", "5", "数字", 0x06, 0x17),
    key("6", "6", "6", "数字", 0x07, 0x16),
    key("7", "7", "7", "数字", 0x08, 0x1A),
    key("8", "8", "8", "数字", 0x09, 0x1C),
    key("9", "9", "9", "数字", 0x0A, 0x19),
    key("escape", "Esc", "Esc", "编辑与导航", 0x01, 0x35),
    key("tab", "Tab", "Tab", "编辑与导航", 0x0F, 0x30),
    key(
        "caps_lock",
        "Caps Lock",
        "Caps Lock",
        "编辑与导航",
        0x3A,
        0x39,
    ),
    key("space", "Space", "Space", "编辑与导航", 0x39, 0x31),
    key("enter", "Enter", "Return", "编辑与导航", 0x1C, 0x24),
    key(
        "backspace",
        "Backspace",
        "Delete ⌫",
        "编辑与导航",
        0x0E,
        0x33,
    ),
    key("delete", "Delete", "向前删除 ⌦", "编辑与导航", 0x153, 0x75),
    key(
        "insert",
        "Insert",
        "Help / Insert",
        "编辑与导航",
        0x152,
        0x72,
    ),
    key("home", "Home", "Home", "编辑与导航", 0x147, 0x73),
    key("end", "End", "End", "编辑与导航", 0x14F, 0x77),
    key("page_up", "Page Up", "Page Up", "编辑与导航", 0x149, 0x74),
    key(
        "page_down",
        "Page Down",
        "Page Down",
        "编辑与导航",
        0x151,
        0x79,
    ),
    key("left", "←", "←", "编辑与导航", 0x14B, 0x7B),
    key("right", "→", "→", "编辑与导航", 0x14D, 0x7C),
    key("up", "↑", "↑", "编辑与导航", 0x148, 0x7E),
    key("down", "↓", "↓", "编辑与导航", 0x150, 0x7D),
    key("f1", "F1", "F1", "功能键", 0x3B, 0x7A),
    key("f2", "F2", "F2", "功能键", 0x3C, 0x78),
    key("f3", "F3", "F3", "功能键", 0x3D, 0x63),
    key("f4", "F4", "F4", "功能键", 0x3E, 0x76),
    key("f5", "F5", "F5", "功能键", 0x3F, 0x60),
    key("f6", "F6", "F6", "功能键", 0x40, 0x61),
    key("f7", "F7", "F7", "功能键", 0x41, 0x62),
    key("f8", "F8", "F8", "功能键", 0x42, 0x64),
    key("f9", "F9", "F9", "功能键", 0x43, 0x65),
    key("f10", "F10", "F10", "功能键", 0x44, 0x6D),
    key("f11", "F11", "F11", "功能键", 0x57, 0x67),
    key("f12", "F12", "F12", "功能键", 0x58, 0x6F),
    key("scroll_lock", "Scroll Lock", "F14", "功能键", 0x46, 0x6B),
    key("grave", "`", "`", "符号", 0x29, 0x32),
    key("minus", "-", "-", "符号", 0x0C, 0x1B),
    key("equal", "=", "=", "符号", 0x0D, 0x18),
    key("left_bracket", "[", "[", "符号", 0x1A, 0x21),
    key("right_bracket", "]", "]", "符号", 0x1B, 0x1E),
    key("backslash", "\\", "\\", "符号", 0x2B, 0x2A),
    key("semicolon", ";", ";", "符号", 0x27, 0x29),
    key("quote", "'", "'", "符号", 0x28, 0x27),
    key("comma", ",", ",", "符号", 0x33, 0x2B),
    key("period", ".", ".", "符号", 0x34, 0x2F),
    key("slash", "/", "/", "符号", 0x35, 0x2C),
    key("numpad_0", "小键盘 0", "小键盘 0", "数字键盘", 0x52, 0x52),
    key("numpad_1", "小键盘 1", "小键盘 1", "数字键盘", 0x4F, 0x53),
    key("numpad_2", "小键盘 2", "小键盘 2", "数字键盘", 0x50, 0x54),
    key("numpad_3", "小键盘 3", "小键盘 3", "数字键盘", 0x51, 0x55),
    key("numpad_4", "小键盘 4", "小键盘 4", "数字键盘", 0x4B, 0x56),
    key("numpad_5", "小键盘 5", "小键盘 5", "数字键盘", 0x4C, 0x57),
    key("numpad_6", "小键盘 6", "小键盘 6", "数字键盘", 0x4D, 0x58),
    key("numpad_7", "小键盘 7", "小键盘 7", "数字键盘", 0x47, 0x59),
    key("numpad_8", "小键盘 8", "小键盘 8", "数字键盘", 0x48, 0x5B),
    key("numpad_9", "小键盘 9", "小键盘 9", "数字键盘", 0x49, 0x5C),
    key(
        "numpad_decimal",
        "小键盘 .",
        "小键盘 .",
        "数字键盘",
        0x53,
        0x41,
    ),
    key("numpad_add", "小键盘 +", "小键盘 +", "数字键盘", 0x4E, 0x45),
    key(
        "numpad_subtract",
        "小键盘 -",
        "小键盘 -",
        "数字键盘",
        0x4A,
        0x4E,
    ),
    key(
        "numpad_multiply",
        "小键盘 *",
        "小键盘 *",
        "数字键盘",
        0x37,
        0x43,
    ),
    key(
        "numpad_divide",
        "小键盘 /",
        "小键盘 /",
        "数字键盘",
        0x135,
        0x4B,
    ),
    key(
        "numpad_enter",
        "小键盘 Enter",
        "小键盘 Enter",
        "数字键盘",
        0x11C,
        0x4C,
    ),
    key("num_lock", "Num Lock", "Clear", "数字键盘", 0x45, 0x47),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keys_have_unique_ids_and_supported_input_codes() {
        let mut ids = HashSet::new();
        let mut codes = HashSet::new();
        for key in KEY_OPTIONS {
            assert!(ids.insert(key.id));
            assert!(codes.insert(key.scancode));
            assert!(
                crate::scancode_to_keycode(key.scancode as u8, key.scancode & 0x100 != 0).is_some()
            );
        }
    }

    #[test]
    fn unknown_keys_and_ambiguous_sources_are_rejected() {
        let rule = KeyMapping {
            from: KeyChord {
                key: "c".into(),
                modifiers: vec![Modifier::Control, Modifier::Shift],
            },
            to: KeyChord {
                key: "c".into(),
                modifiers: vec![Modifier::Meta],
            },
        };
        let mut reordered = rule.clone();
        reordered.from.modifiers.reverse();
        assert!(KeyboardMappings::try_from(vec![rule.clone(), reordered]).is_err());
        let mut unknown = rule;
        unknown.to.key = "not-a-key".into();
        assert!(KeyboardMappings::try_from(vec![unknown]).is_err());
    }
}
