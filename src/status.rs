#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeState {
    percent: u16,
    known: bool,
    muted: bool,
}

impl VolumeState {
    pub const UNKNOWN: Self = Self {
        percent: 0,
        known: false,
        muted: false,
    };

    pub const fn new(percent: u16, muted: bool) -> Self {
        Self {
            percent,
            known: true,
            muted,
        }
    }

    fn push_to(self, out: &mut String) {
        if !self.known {
            out.push_str("??%");
            return;
        }

        let value = if self.muted { 0 } else { self.percent };
        push_u16(out, value);
        out.push('%');
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutState {
    bytes: [u8; 3],
    len: u8,
}

impl LayoutState {
    pub const UNKNOWN: Self = Self {
        bytes: [b'?', b'?', 0],
        len: 2,
    };

    pub const fn from_bytes(bytes: [u8; 3], len: u8) -> Self {
        Self { bytes, len }
    }

    pub fn from_name(name: &str) -> Self {
        if name.contains("English (US)") {
            return Self::from_ascii("us");
        }

        if name.contains("Russian") {
            return Self::from_ascii("ru");
        }

        if name.contains("Ukrainian") {
            return Self::from_ascii("ua");
        }

        let mut bytes = [0u8; 3];
        let mut len = 0usize;

        for ch in name.bytes() {
            if !ch.is_ascii_alphabetic() {
                continue;
            }

            bytes[len] = ch.to_ascii_lowercase();
            len += 1;

            if len == bytes.len() {
                break;
            }
        }

        if len == 0 {
            Self::UNKNOWN
        } else {
            Self::from_bytes(bytes, len as u8)
        }
    }

    pub fn from_ascii(code: &str) -> Self {
        let bytes = code.as_bytes();
        let mut out = [0u8; 3];
        let len = bytes.len().min(out.len());
        out[..len].copy_from_slice(&bytes[..len]);
        Self::from_bytes(out, len as u8)
    }

    pub fn is_unknown(self) -> bool {
        self == Self::UNKNOWN
    }

    fn push_to(self, out: &mut String) {
        out.push_str(std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("??"));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockState {
    bytes: [u8; 22],
    len: u8,
}

impl ClockState {
    pub const fn from_bytes(bytes: [u8; 22], len: u8) -> Self {
        Self { bytes, len }
    }

    fn push_to(self, out: &mut String) {
        out.push_str(std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or(""));
    }
}

pub fn render_into(
    out: &mut String,
    volume: VolumeState,
    layout: LayoutState,
    time: ClockState,
) -> &str {
    out.clear();
    out.reserve(32);
    volume.push_to(out);
    out.push(' ');
    layout.push_to(out);
    out.push(' ');
    time.push_to(out);
    out.as_str()
}

fn push_u16(out: &mut String, value: u16) {
    if value >= 100 {
        out.push(char::from(b'0' + ((value / 100) % 10) as u8));
        out.push(char::from(b'0' + ((value / 10) % 10) as u8));
        out.push(char::from(b'0' + (value % 10) as u8));
    } else if value >= 10 {
        out.push(char::from(b'0' + ((value / 10) % 10) as u8));
        out.push(char::from(b'0' + (value % 10) as u8));
    } else {
        out.push(char::from(b'0' + value as u8));
    }
}

#[cfg(test)]
mod tests {
    use super::{render_into, ClockState, LayoutState, VolumeState};

    #[test]
    fn renders_unknown_values() {
        let mut buf = String::new();
        let clock = ClockState::from_bytes(*b"2026-04-23 09:44:01 PM", 22);
        assert_eq!(
            render_into(&mut buf, VolumeState::UNKNOWN, LayoutState::UNKNOWN, clock),
            "??% ?? 2026-04-23 09:44:01 PM"
        );
    }

    #[test]
    fn maps_custom_layout_names_to_compact_codes() {
        assert_eq!(
            LayoutState::from_name("English (US)"),
            LayoutState::from_ascii("us")
        );
        assert_eq!(
            LayoutState::from_name("German"),
            LayoutState::from_ascii("ger")
        );
    }

    #[test]
    fn renders_muted_as_zero_percent() {
        let mut buf = String::new();
        let clock = ClockState::from_bytes(*b"2026-04-23 09:44:01 PM", 22);
        assert_eq!(
            render_into(
                &mut buf,
                VolumeState::new(42, true),
                LayoutState::from_ascii("us"),
                clock
            ),
            "0% us 2026-04-23 09:44:01 PM"
        );
    }
}
