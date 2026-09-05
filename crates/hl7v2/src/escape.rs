//! HL7 escape sequence decoding.

use crate::Encoding;
use std::borrow::Cow;

/// Decode HL7 escape sequences in `text` using the message's encoding characters.
///
/// Handled: `\F\` `\S\` `\T\` `\R\` `\E\` (the five delimiters), `\Xdd..\`
/// (hex bytes, decoded as UTF-8 with replacement), `\.br\` (line break, emitted
/// as `\n`). Formatting commands (`\H\`, `\N\`, `\.sp\`, ...) and unknown
/// sequences are kept verbatim so nothing is silently lost. An unterminated
/// escape is also kept verbatim.
///
/// Returns the input unchanged (borrowed) when it contains no escape character.
pub fn unescape<'a>(text: &'a str, enc: Encoding) -> Cow<'a, str> {
    let esc = enc.escape as char;
    if !text.contains(esc) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(esc) {
        out.push_str(&rest[..start]);
        let after = &rest[start + esc.len_utf8()..];
        match after.find(esc) {
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
            Some(end) => {
                let seq = &after[..end];
                match decode_sequence(seq, enc) {
                    Some(s) => out.push_str(&s),
                    None => {
                        out.push(esc);
                        out.push_str(seq);
                        out.push(esc);
                    }
                }
                rest = &after[end + esc.len_utf8()..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn decode_sequence(seq: &str, enc: Encoding) -> Option<String> {
    match seq {
        "F" => Some((enc.field as char).to_string()),
        "S" => Some((enc.component as char).to_string()),
        "T" => Some((enc.subcomponent as char).to_string()),
        "R" => Some((enc.repetition as char).to_string()),
        "E" => Some((enc.escape as char).to_string()),
        ".br" => Some("\n".to_string()),
        _ => {
            let hex = seq.strip_prefix('X')?;
            if hex.is_empty() || hex.len() % 2 != 0 {
                return None;
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_when_no_escape() {
        let e = Encoding::STANDARD;
        assert!(matches!(unescape("plain", e), Cow::Borrowed(_)));
    }

    #[test]
    fn delimiters() {
        let e = Encoding::STANDARD;
        assert_eq!(unescape(r"a\F\b\S\c\T\d\R\e\E\f", e), "a|b^c&d~e\\f");
    }

    #[test]
    fn hex_and_break() {
        let e = Encoding::STANDARD;
        assert_eq!(unescape(r"caf\XC3A9\ x\.br\y", e), "café x\ny");
    }

    #[test]
    fn unknown_and_unterminated_kept() {
        let e = Encoding::STANDARD;
        assert_eq!(
            unescape(r"\H\bold\N\ and \oops", e),
            r"\H\bold\N\ and \oops"
        );
        assert_eq!(unescape(r"\XZZ\", e), r"\XZZ\");
    }

    #[test]
    fn custom_escape_char() {
        let e = Encoding::from_msh2(b'|', b"^~#&").unwrap();
        assert_eq!(unescape("a#F#b", e), "a|b");
    }
}
