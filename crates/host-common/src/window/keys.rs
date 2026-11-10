//! The keys our plugin windows pass on, as plain functions of the window systems' numbers (tested
//! without a window system): a Win32 keyboard message's flags, and what a key types, by an X11
//! keysym or a Win32 virtual key, for a plugin format that is handed keys by what they type.

/// What a key types with no modifier held, as the user's keyboard layout has it: a character, or
/// the name of a key that types none. For a plugin format that is handed keys by what they type
/// (VST3's `IPlugView::onKeyDown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed {
    Char(char),
    Named(NamedKey),
}

/// A key that types no character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    Backspace,
    Tab,
    Enter,
    Escape,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Up,
    Right,
    Down,
    /// A function key, F1 to F12.
    F(u8),
}

/// What a Win32 keyboard message's `lParam` says of its key (WM_KEYDOWN's layout, which the
/// keyboard hook's flags share): the scan code, `pressed` (the transition bit 31 clear),
/// `repeat` (a press of a key already down: the previous-state bit 30 differs from the transition
/// bit, as winit reads it) and `alt` (the context bit 29: Alt held).
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Win32Key {
    /// Bits 16 to 23, with 0xE000 added when the extended bit 24 is set: the number winit's
    /// `PhysicalKeyExtScancode::from_scancode` takes on Windows. 0 when the device gave none.
    pub scancode: u32,
    pub pressed: bool,
    pub repeat: bool,
    pub alt: bool,
}

#[cfg(any(windows, test))]
pub(super) fn win32_key(lparam: isize) -> Win32Key {
    let bit = |n: u32| (lparam >> n) & 1 == 1;
    let code = ((lparam >> 16) & 0xFF) as u32;
    let released = bit(31);
    Win32Key {
        scancode: if bit(24) { 0xE000 | code } else { code },
        pressed: !released,
        repeat: bit(30) != released,
        alt: bit(29),
    }
}

/// What the key of Win32 virtual key `vk` types: a key named by its virtual key (winuser.h's
/// VK_*), else the character `character` (MapVirtualKeyW's MAPVK_VK_TO_CHAR answer: a capital
/// for a letter, made small here; 0 for none; bit 31 set for a dead key, which types nothing
/// alone).
#[cfg(any(windows, test))]
pub(super) fn typed_of_vk(vk: u16, character: u32) -> Option<Typed> {
    let named = match vk {
        0x08 => Some(NamedKey::Backspace),
        0x09 => Some(NamedKey::Tab),
        0x0D => Some(NamedKey::Enter),
        0x1B => Some(NamedKey::Escape),
        0x21 => Some(NamedKey::PageUp),
        0x22 => Some(NamedKey::PageDown),
        0x23 => Some(NamedKey::End),
        0x24 => Some(NamedKey::Home),
        0x25 => Some(NamedKey::Left),
        0x26 => Some(NamedKey::Up),
        0x27 => Some(NamedKey::Right),
        0x28 => Some(NamedKey::Down),
        0x2D => Some(NamedKey::Insert),
        0x2E => Some(NamedKey::Delete),
        // VK_F1 to VK_F12.
        0x70..=0x7B => Some(NamedKey::F((vk - 0x6F) as u8)),
        _ => None,
    };
    if let Some(named) = named {
        return Some(Typed::Named(named));
    }
    if character == 0 || character & 0x8000_0000 != 0 {
        return None;
    }
    let typed = char::from_u32(character)?;
    typed.to_lowercase().next().map(Typed::Char)
}

/// What an X11 keysym (X11's keysymdef.h) types: Latin-1's printable keysyms are their own
/// characters, a Unicode keysym (0x0100_0000 plus the code point) its code point, and the keys
/// named here by theirs; anything else (a modifier, a dead key, a keysym this list lacks) none.
#[cfg(any(unix, test))]
pub(super) fn typed_of_keysym(keysym: u32) -> Option<Typed> {
    let named = match keysym {
        0xFF08 => NamedKey::Backspace,
        0xFF09 => NamedKey::Tab,
        // Return, and the keypad's Enter.
        0xFF0D | 0xFF8D => NamedKey::Enter,
        0xFF1B => NamedKey::Escape,
        0xFF50 => NamedKey::Home,
        0xFF51 => NamedKey::Left,
        0xFF52 => NamedKey::Up,
        0xFF53 => NamedKey::Right,
        0xFF54 => NamedKey::Down,
        0xFF55 => NamedKey::PageUp,
        0xFF56 => NamedKey::PageDown,
        0xFF57 => NamedKey::End,
        0xFF63 => NamedKey::Insert,
        0xFFFF => NamedKey::Delete,
        // XK_F1 to XK_F12.
        0xFFBE..=0xFFC9 => NamedKey::F((keysym - 0xFFBD) as u8),
        0x20..=0x7E | 0xA0..=0xFF => return char::from_u32(keysym).map(Typed::Char),
        0x0100_0100..=0x0110_FFFF => return char::from_u32(keysym - 0x0100_0000).map(Typed::Char),
        _ => return None,
    };
    Some(Typed::Named(named))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The spec's hand values, from WM_KEYDOWN's lParam layout (bits 0-15 the repeat count, 16-23
    // the scan code, 24 extended, 29 the context code, 30 the previous state, 31 the transition):
    // 0x002C0001 is Z's scan code (0x2C) pressed once; 0x402C0001 the same key repeated (it was
    // down); 0xC02C0001 its release; the extended bit makes it 0xE02C, as winit's from_scancode
    // numbers the extended keys; the context bit says Alt is held.
    #[test]
    fn a_win32_keys_flags_give_its_scancode_press_and_repeat() {
        let key = |scancode, pressed, repeat, alt| Win32Key {
            scancode,
            pressed,
            repeat,
            alt,
        };
        assert_eq!(win32_key(0x002C_0001), key(0x2C, true, false, false));
        assert_eq!(win32_key(0x402C_0001), key(0x2C, true, true, false));
        assert_eq!(win32_key(0xC02C_0001), key(0x2C, false, false, false));
        assert_eq!(win32_key(0x012C_0001), key(0xE02C, true, false, false));
        assert_eq!(win32_key(0x202C_0001), key(0x2C, true, false, true));
        // Right Ctrl let go of: 0x1D with the extended bit, previous state down, transition up.
        assert_eq!(win32_key(0xC11D_0001), key(0xE01D, false, false, false));
    }

    // winuser.h's virtual keys: VK_BACK 0x08, VK_TAB 0x09, VK_RETURN 0x0D, VK_ESCAPE 0x1B,
    // VK_LEFT 0x25, VK_DELETE 0x2E, VK_F1 0x70, VK_F12 0x7B; 'A' (0x41), whose MAPVK_VK_TO_CHAR
    // is the capital, types a small a; VK_OEM_1 (0xBA) types ';' on a US layout; a dead key's
    // answer has bit 31 set (here the circumflex, 0x5E) and a key with no character answers 0.
    #[test]
    fn a_virtual_key_types_its_name_or_its_small_character() {
        let named = |key| Some(Typed::Named(key));
        assert_eq!(typed_of_vk(0x08, 0x08), named(NamedKey::Backspace));
        assert_eq!(typed_of_vk(0x09, 0x09), named(NamedKey::Tab));
        assert_eq!(typed_of_vk(0x0D, 0x0D), named(NamedKey::Enter));
        assert_eq!(typed_of_vk(0x1B, 0x1B), named(NamedKey::Escape));
        assert_eq!(typed_of_vk(0x25, 0), named(NamedKey::Left));
        assert_eq!(typed_of_vk(0x2E, 0), named(NamedKey::Delete));
        assert_eq!(typed_of_vk(0x70, 0), named(NamedKey::F(1)));
        assert_eq!(typed_of_vk(0x7B, 0), named(NamedKey::F(12)));
        assert_eq!(typed_of_vk(0x41, 0x41), Some(Typed::Char('a')));
        assert_eq!(typed_of_vk(0xBA, 0x3B), Some(Typed::Char(';')));
        assert_eq!(typed_of_vk(0x20, 0x20), Some(Typed::Char(' ')));
        assert_eq!(typed_of_vk(0xDC, 0x8000_005E), None, "a dead key");
        assert_eq!(typed_of_vk(0x10, 0), None, "Shift types nothing");
    }

    // keysymdef.h: XK_a 0x61, XK_semicolon 0x3B, XK_adiaeresis 0xE4, XK_space 0x20,
    // XK_BackSpace 0xFF08, XK_Tab 0xFF09, XK_Return 0xFF0D, XK_KP_Enter 0xFF8D, XK_Escape 0xFF1B,
    // XK_Left 0xFF51, XK_Delete 0xFFFF, XK_F1 0xFFBE, XK_F12 0xFFC9, XK_Shift_L 0xFFE1,
    // XK_dead_acute 0xFE51; the Unicode keysym of GREEK SMALL LETTER ALPHA, 0x10003B1.
    #[test]
    fn a_keysym_types_its_character_or_its_name() {
        let named = |key| Some(Typed::Named(key));
        assert_eq!(typed_of_keysym(0x61), Some(Typed::Char('a')));
        assert_eq!(typed_of_keysym(0x3B), Some(Typed::Char(';')));
        assert_eq!(typed_of_keysym(0xE4), Some(Typed::Char('\u{E4}')));
        assert_eq!(typed_of_keysym(0x20), Some(Typed::Char(' ')));
        assert_eq!(typed_of_keysym(0x0100_03B1), Some(Typed::Char('\u{3B1}')));
        assert_eq!(typed_of_keysym(0xFF08), named(NamedKey::Backspace));
        assert_eq!(typed_of_keysym(0xFF09), named(NamedKey::Tab));
        assert_eq!(typed_of_keysym(0xFF0D), named(NamedKey::Enter));
        assert_eq!(typed_of_keysym(0xFF8D), named(NamedKey::Enter));
        assert_eq!(typed_of_keysym(0xFF1B), named(NamedKey::Escape));
        assert_eq!(typed_of_keysym(0xFF51), named(NamedKey::Left));
        assert_eq!(typed_of_keysym(0xFFFF), named(NamedKey::Delete));
        assert_eq!(typed_of_keysym(0xFFBE), named(NamedKey::F(1)));
        assert_eq!(typed_of_keysym(0xFFC9), named(NamedKey::F(12)));
        assert_eq!(typed_of_keysym(0xFFE1), None, "Shift types nothing");
        assert_eq!(typed_of_keysym(0xFE51), None, "a dead key");
        assert_eq!(typed_of_keysym(0), None, "NoSymbol");
    }
}
