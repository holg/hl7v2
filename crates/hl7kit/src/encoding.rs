//! Encoding characters declared in MSH-1 and MSH-2.

/// The five delimiter characters of an HL7 v2 message.
///
/// The field separator is MSH-1 (byte 3 of the message); the other four are
/// MSH-2, in the order component, repetition, escape, subcomponent. HL7 2.7
/// adds an optional fifth truncation character which this crate ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Encoding {
    /// Field separator, conventionally `|`.
    pub field: u8,
    /// Component separator, conventionally `^`.
    pub component: u8,
    /// Repetition separator, conventionally `~`.
    pub repetition: u8,
    /// Escape character, conventionally `\`.
    pub escape: u8,
    /// Subcomponent separator, conventionally `&`.
    pub subcomponent: u8,
}

impl Default for Encoding {
    fn default() -> Self {
        Self {
            field: b'|',
            component: b'^',
            repetition: b'~',
            escape: b'\\',
            subcomponent: b'&',
        }
    }
}

impl Encoding {
    /// The standard encoding, `|^~\&`.
    pub const STANDARD: Encoding = Encoding {
        field: b'|',
        component: b'^',
        repetition: b'~',
        escape: b'\\',
        subcomponent: b'&',
    };

    /// Build an encoding from the field separator (MSH-1) and the raw bytes of
    /// MSH-2. Returns `None` when MSH-2 is malformed: fewer than four or more
    /// than five bytes, non-ASCII, duplicated, or colliding with the field
    /// separator. Callers should fall back to [`Encoding::with_field`].
    pub fn from_msh2(field: u8, msh2: &[u8]) -> Option<Encoding> {
        if !(4..=5).contains(&msh2.len()) {
            return None;
        }
        let chars = &msh2[..4];
        if !chars
            .iter()
            .all(|c| c.is_ascii() && !c.is_ascii_alphanumeric())
        {
            return None;
        }
        let all = [field, chars[0], chars[1], chars[2], chars[3]];
        for (i, a) in all.iter().enumerate() {
            if all[i + 1..].contains(a) {
                return None;
            }
        }
        Some(Encoding {
            field,
            component: chars[0],
            repetition: chars[1],
            escape: chars[2],
            subcomponent: chars[3],
        })
    }

    /// The standard encoding characters with a custom field separator.
    pub fn with_field(field: u8) -> Encoding {
        Encoding {
            field,
            ..Encoding::STANDARD
        }
    }

    /// The MSH-2 value that declares this encoding, e.g. `^~\&`.
    pub fn msh2(&self) -> String {
        [
            self.component,
            self.repetition,
            self.escape,
            self.subcomponent,
        ]
        .iter()
        .map(|&b| b as char)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_roundtrip() {
        let e = Encoding::from_msh2(b'|', b"^~\\&").unwrap();
        assert_eq!(e, Encoding::STANDARD);
        assert_eq!(e.msh2(), "^~\\&");
    }

    #[test]
    fn accepts_truncation_char() {
        let e = Encoding::from_msh2(b'|', b"^~\\&#").unwrap();
        assert_eq!(e, Encoding::STANDARD);
    }

    #[test]
    fn rejects_short_duplicate_and_alnum() {
        assert!(Encoding::from_msh2(b'|', b"^~\\").is_none());
        assert!(Encoding::from_msh2(b'|', b"^^\\&").is_none());
        assert!(Encoding::from_msh2(b'|', b"^~|&").is_none());
        assert!(Encoding::from_msh2(b'|', b"a~\\&").is_none());
        assert!(Encoding::from_msh2(b'|', "é~\\&".as_bytes()).is_none());
    }
}
