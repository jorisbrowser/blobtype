//! Platform-independent conversion of text into key actions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// One UTF-16 code unit sent with KEYEVENTF_UNICODE (surrogate pairs are
    /// sent as two consecutive units, which is how Windows expects them).
    Unicode(u16),
    Enter,
    Tab,
}

/// Convert text to key actions. Newlines become Enter, tabs Tab, other
/// control characters are dropped.
pub fn plan_keys(text: &str) -> Vec<KeyAction> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(KeyAction::Enter);
            }
            '\n' => out.push(KeyAction::Enter),
            '\t' => out.push(KeyAction::Tab),
            c if c.is_control() => {}
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push(KeyAction::Unicode(*unit));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_newlines() {
        assert_eq!(
            plan_keys("a\r\nb\nc"),
            vec![
                KeyAction::Unicode('a' as u16),
                KeyAction::Enter,
                KeyAction::Unicode('b' as u16),
                KeyAction::Enter,
                KeyAction::Unicode('c' as u16),
            ]
        );
    }

    #[test]
    fn surrogate_pairs() {
        let k = plan_keys("😀");
        assert_eq!(
            k,
            vec![KeyAction::Unicode(0xD83D), KeyAction::Unicode(0xDE00)]
        );
    }

    #[test]
    fn drops_control_chars_keeps_tab() {
        assert_eq!(
            plan_keys("\u{7}\tä"),
            vec![KeyAction::Tab, KeyAction::Unicode('ä' as u16)]
        );
    }
}
