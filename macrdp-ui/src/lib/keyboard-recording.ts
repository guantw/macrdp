import type { KeyChord, KeyModifier } from "./types";

export const modifiers: KeyModifier[] = ["control", "alt", "shift", "meta"];

export function keyModifier(key: string): KeyModifier | undefined {
  return modifiers.find((modifier) => key === `left_${modifier}` || key === `right_${modifier}`);
}

export function keyIdFromCode(code: string): string | undefined {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-2])$/.test(code)) return code.toLowerCase();
  if (/^Numpad[0-9]$/.test(code)) return `numpad_${code.slice(6)}`;
  const keys: Record<string, string> = {
    ControlLeft: "left_control", ControlRight: "right_control",
    AltLeft: "left_alt", AltRight: "right_alt",
    ShiftLeft: "left_shift", ShiftRight: "right_shift",
    MetaLeft: "left_meta", MetaRight: "right_meta",
    Escape: "escape", Tab: "tab", CapsLock: "caps_lock", Space: "space",
    Enter: "enter", Backspace: "backspace", Delete: "delete", Insert: "insert",
    Home: "home", End: "end", PageUp: "page_up", PageDown: "page_down",
    ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down",
    ScrollLock: "scroll_lock", Backquote: "grave", Minus: "minus", Equal: "equal",
    BracketLeft: "left_bracket", BracketRight: "right_bracket", Backslash: "backslash",
    Semicolon: "semicolon", Quote: "quote", Comma: "comma", Period: "period", Slash: "slash",
    NumpadDecimal: "numpad_decimal", NumpadAdd: "numpad_add", NumpadSubtract: "numpad_subtract",
    NumpadMultiply: "numpad_multiply", NumpadDivide: "numpad_divide", NumpadEnter: "numpad_enter", NumLock: "num_lock",
  };
  return keys[code];
}

/** Shared by original RDP input and local DOM input. No mappings are applied here. */
export class ChordRecorder {
  private held = new Set<string>();
  private chord: KeyChord | null = null;

  keyDown(key: string, activeModifiers: KeyModifier[] = []): KeyChord {
    this.held.add(key);
    const modifier = keyModifier(key);
    const main = modifier && this.chord && !keyModifier(this.chord.key) ? this.chord.key : key;
    const pressedModifiers = [...this.held].map(keyModifier);
    this.chord = {
      key: main,
      modifiers: modifiers.filter((value) => value !== keyModifier(main)
        && (pressedModifiers.includes(value) || activeModifiers.includes(value))),
    };
    return this.chord;
  }

  keyUp(key: string): KeyChord | null {
    // Finish on release, including modifier-only input and Command combinations
    // whose main-key release is not delivered by the webview.
    return this.held.has(key) ? this.chord : null;
  }
}
