//! FHIR primitive types: the value, and the element parts a primitive can
//! carry beside it (`"birthDate": "1970"` plus `"_birthDate": {"id": ..,
//! "extension": [..]}`).
//!
//! Each FHIR primitive maps to one Rust value type and one [`Kind`] that
//! says which lexical rules apply. The rules are the ones in the R4
//! specification's primitive type definitions, written out by hand so the
//! crate needs no regex engine.

use crate::json::Json;
use crate::r4::Extension;

/// A primitive element: an optional value, and the optional `id` and
/// extensions that JSON carries in the `_name` twin. A value may be absent
/// when an extension says why (data-absent-reason, for instance).
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive<T> {
    /// The value, if there is one.
    pub value: Option<T>,
    /// The element id from the `_name` twin.
    pub id: Option<String>,
    /// Extensions from the `_name` twin.
    pub extension: Vec<Extension>,
}

impl<T> Default for Primitive<T> {
    fn default() -> Self {
        Primitive {
            value: None,
            id: None,
            extension: Vec::new(),
        }
    }
}

impl<T> Primitive<T> {
    /// A primitive holding just a value.
    pub fn new(value: T) -> Primitive<T> {
        Primitive {
            value: Some(value),
            id: None,
            extension: Vec::new(),
        }
    }

    /// `true` when there is neither a value nor element parts.
    pub fn is_empty(&self) -> bool {
        self.value.is_none() && self.id.is_none() && self.extension.is_empty()
    }
}

impl<T> From<T> for Primitive<T> {
    fn from(value: T) -> Self {
        Primitive::new(value)
    }
}

impl From<&str> for Primitive<String> {
    fn from(value: &str) -> Self {
        Primitive::new(value.to_string())
    }
}

/// A FHIR `decimal`: the number exactly as written, so precision survives
/// (`1.50` is not `1.5`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decimal(String);

impl Decimal {
    /// Check and wrap decimal text.
    pub fn parse(text: &str) -> Result<Decimal, String> {
        if is_decimal(text) {
            Ok(Decimal(text.to_string()))
        } else {
            Err(format!("{text:?} is not a FHIR decimal"))
        }
    }

    /// The text as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The value as a float, for arithmetic that can afford to round.
    pub fn to_f64(&self) -> f64 {
        self.0.parse().unwrap_or(f64::NAN)
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which primitive a value is, and so which rules it must follow.
/// Variants are named after the FHIR types they stand for.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Integer,
    PositiveInt,
    UnsignedInt,
    Decimal,
    String,
    Code,
    Id,
    Markdown,
    Uri,
    Url,
    Canonical,
    Oid,
    Uuid,
    Base64Binary,
    Date,
    DateTime,
    Instant,
    Time,
    Xhtml,
}

impl Kind {
    /// The FHIR type name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Boolean => "boolean",
            Kind::Integer => "integer",
            Kind::PositiveInt => "positiveInt",
            Kind::UnsignedInt => "unsignedInt",
            Kind::Decimal => "decimal",
            Kind::String => "string",
            Kind::Code => "code",
            Kind::Id => "id",
            Kind::Markdown => "markdown",
            Kind::Uri => "uri",
            Kind::Url => "url",
            Kind::Canonical => "canonical",
            Kind::Oid => "oid",
            Kind::Uuid => "uuid",
            Kind::Base64Binary => "base64Binary",
            Kind::Date => "date",
            Kind::DateTime => "dateTime",
            Kind::Instant => "instant",
            Kind::Time => "time",
            Kind::Xhtml => "xhtml",
        }
    }

    /// Check text against this kind's lexical rules. `Err` says why not.
    pub fn check(self, s: &str) -> Result<(), String> {
        let ok = match self {
            Kind::String | Kind::Markdown | Kind::Xhtml => !s.is_empty(),
            Kind::Code => is_code(s),
            Kind::Id => is_id(s),
            Kind::Uri | Kind::Url | Kind::Canonical => {
                !s.is_empty() && !s.chars().any(char::is_whitespace)
            }
            Kind::Oid => is_oid(s),
            Kind::Uuid => is_uuid(s),
            Kind::Base64Binary => is_base64(s),
            Kind::Date => is_date(s),
            Kind::DateTime => is_datetime(s),
            Kind::Instant => is_instant(s),
            Kind::Time => is_time(s),
            Kind::Decimal => is_decimal(s),
            Kind::Integer | Kind::PositiveInt | Kind::UnsignedInt | Kind::Boolean => true,
        };
        if ok {
            Ok(())
        } else if s.is_empty() {
            Err(format!(
                "an empty {} is not allowed; leave the element out",
                self.name()
            ))
        } else {
            Err(format!("{s:?} is not a valid {}", self.name()))
        }
    }
}

/// A Rust type that holds one kind of FHIR primitive value.
pub trait PrimitiveValue: Sized {
    /// Read the JSON form, checked against `kind`.
    fn from_json(v: &Json, kind: Kind) -> Result<Self, String>;
    /// The JSON form.
    fn to_json(&self) -> Json;
}

impl PrimitiveValue for bool {
    fn from_json(v: &Json, _: Kind) -> Result<Self, String> {
        match v {
            Json::Bool(b) => Ok(*b),
            other => Err(format!("expected true or false, found {}", other.kind())),
        }
    }
    fn to_json(&self) -> Json {
        Json::Bool(*self)
    }
}

impl PrimitiveValue for i32 {
    fn from_json(v: &Json, kind: Kind) -> Result<Self, String> {
        let Json::Number(n) = v else {
            return Err(format!("expected a whole number, found {}", v.kind()));
        };
        if !is_integer(n) {
            return Err(format!("{n} is not a FHIR integer"));
        }
        let x: i32 = n
            .parse()
            .map_err(|_| format!("{n} is outside the 32-bit range of a FHIR integer"))?;
        match kind {
            Kind::PositiveInt if x < 1 => Err(format!("{n} is not a positiveInt (1 or more)")),
            Kind::UnsignedInt if x < 0 => Err(format!("{n} is not an unsignedInt (0 or more)")),
            _ => Ok(x),
        }
    }
    fn to_json(&self) -> Json {
        Json::Number(self.to_string())
    }
}

impl PrimitiveValue for Decimal {
    fn from_json(v: &Json, _: Kind) -> Result<Self, String> {
        match v {
            Json::Number(n) => Decimal::parse(n),
            other => Err(format!("expected a number, found {}", other.kind())),
        }
    }
    fn to_json(&self) -> Json {
        Json::Number(self.0.clone())
    }
}

impl PrimitiveValue for String {
    fn from_json(v: &Json, kind: Kind) -> Result<Self, String> {
        match v {
            Json::String(s) => kind.check(s).map(|()| s.clone()),
            other => Err(format!("expected a string, found {}", other.kind())),
        }
    }
    fn to_json(&self) -> Json {
        Json::String(self.clone())
    }
}

// --- lexical rules, from the R4 primitive type definitions ---

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `-?(0|[1-9][0-9]*)`
pub fn is_integer(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    digits(s) && (s == "0" || !s.starts_with('0'))
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`
pub fn is_decimal(s: &str) -> bool {
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (int, frac) = match mant.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mant, None),
    };
    is_integer(int)
        && frac.is_none_or(digits)
        && exp.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)))
}

/// `[^\s]+(\s[^\s]+)*`: no leading, trailing or doubled whitespace.
pub fn is_code(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(char::is_whitespace)
        && !s.ends_with(char::is_whitespace)
        && !s
            .chars()
            .zip(s.chars().skip(1))
            .any(|(a, b)| a.is_whitespace() && b.is_whitespace())
}

/// `[A-Za-z0-9\-\.]{1,64}`
pub fn is_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

/// `urn:oid:[0-2](\.(0|[1-9][0-9]*))+`
pub fn is_oid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("urn:oid:") else {
        return false;
    };
    let mut parts = rest.split('.');
    let first_ok = matches!(parts.next(), Some("0" | "1" | "2"));
    let mut n = 0;
    let rest_ok = parts.all(|p| {
        n += 1;
        digits(p) && (p == "0" || !p.starts_with('0'))
    });
    first_ok && rest_ok && n > 0
}

/// `urn:uuid:` and a lower-case 8-4-4-4-12 hex UUID.
pub fn is_uuid(s: &str) -> bool {
    let Some(u) = s.strip_prefix("urn:uuid:") else {
        return false;
    };
    let groups: Vec<&str> = u.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Base64 with optional whitespace, length a multiple of four.
pub fn is_base64(s: &str) -> bool {
    let chars: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    !chars.is_empty()
        && chars.len() % 4 == 0
        && chars
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

fn year(s: &str) -> bool {
    s.len() == 4 && digits(s)
}

fn two(s: &str, lo: u32, hi: u32) -> bool {
    s.len() == 2 && digits(s) && s.parse::<u32>().is_ok_and(|v| (lo..=hi).contains(&v))
}

/// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
pub fn is_date(s: &str) -> bool {
    let mut p = s.splitn(3, '-');
    let (y, m, d) = (p.next(), p.next(), p.next());
    y.is_some_and(year)
        && m.is_none_or(|m| two(m, 1, 12))
        && d.is_none_or(|d| m.is_some() && two(d, 1, 31))
}

/// `HH:MM:SS` with optional fraction.
pub fn is_time(s: &str) -> bool {
    let (hms, frac) = match s.split_once('.') {
        Some((a, f)) => (a, Some(f)),
        None => (s, None),
    };
    let mut p = hms.split(':');
    let ok = p.next().is_some_and(|h| two(h, 0, 23))
        && p.next().is_some_and(|m| two(m, 0, 59))
        && p.next().is_some_and(|x| two(x, 0, 60))
        && p.next().is_none();
    ok && frac.is_none_or(digits)
}

fn zone(z: &str) -> bool {
    if z == "Z" {
        return true;
    }
    let Some(rest) = z.strip_prefix(['+', '-']) else {
        return false;
    };
    match rest.split_once(':') {
        Some((h, m)) => two(h, 0, 14) && two(m, 0, 59),
        None => false,
    }
}

/// A date, or a date and a time with a time zone.
pub fn is_datetime(s: &str) -> bool {
    match s.split_once('T') {
        None => is_date(s),
        Some((d, rest)) => {
            let zi = rest.find(['Z', '+', '-']);
            let Some(zi) = zi else { return false };
            d.len() == 10 && is_date(d) && is_time(&rest[..zi]) && zone(&rest[zi..])
        }
    }
}

/// A full date and time to the second, with a time zone.
pub fn is_instant(s: &str) -> bool {
    s.split_once('T').is_some_and(|(d, _)| d.len() == 10) && is_datetime(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times_follow_the_spec() {
        for ok in [
            "2020",
            "2020-02",
            "2020-02-29",
            "2020-02-29T10:00:00Z",
            "2020-02-29T10:00:00.123+01:00",
        ] {
            assert!(is_datetime(ok), "{ok}");
        }
        for bad in [
            "20",
            "2020-13",
            "2020-02-29T10:00",
            "2020-02-29T10:00:00",
            "2020-2-01",
            "2020-02-29T25:00:00Z",
        ] {
            assert!(!is_datetime(bad), "{bad}");
        }
        assert!(is_instant("2015-02-07T13:28:17.239+02:00"));
        assert!(!is_instant("2015-02-07"));
        assert!(is_time("23:59:60.5") && !is_time("24:00:00"));
    }

    #[test]
    fn numbers_ids_codes_and_uris() {
        assert!(
            is_decimal("-0.50") && is_decimal("1e-3") && !is_decimal("01") && !is_decimal("1.")
        );
        assert!(is_integer("-12") && !is_integer("1.0"));
        assert!(is_id("abc-1.2") && !is_id("a_b") && !is_id(&"x".repeat(65)));
        assert!(is_code("final") && is_code("a b") && !is_code(" a") && !is_code("a  b"));
        assert!(is_oid("urn:oid:1.2.840.10008") && !is_oid("urn:oid:3.1") && !is_oid("1.2.3"));
        assert!(is_uuid("urn:uuid:53fefa32-fcbb-4ff8-8a92-55ee120877b7"));
        assert!(!is_uuid("urn:uuid:53FEFA32-fcbb-4ff8-8a92-55ee120877b7"));
        assert!(is_base64("aGVsbG8=") && !is_base64("abc"));
    }

    #[test]
    fn integer_kinds_check_their_range() {
        let n = |s: &str| Json::Number(s.into());
        assert_eq!(i32::from_json(&n("5"), Kind::PositiveInt), Ok(5));
        assert!(i32::from_json(&n("0"), Kind::PositiveInt).is_err());
        assert!(i32::from_json(&n("-1"), Kind::UnsignedInt).is_err());
        assert!(i32::from_json(&n("2147483648"), Kind::Integer).is_err());
        assert!(i32::from_json(&n("1.0"), Kind::Integer).is_err());
        assert_eq!(
            Decimal::from_json(&n("1.50"), Kind::Decimal)
                .unwrap()
                .as_str(),
            "1.50"
        );
    }
}
