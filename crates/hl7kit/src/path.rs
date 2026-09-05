//! Query paths such as `PID-3.1` or `OBX[2]-5[1].2.1`.

use crate::PathError;

/// A parsed query path.
///
/// Grammar (whitespace not allowed):
///
/// ```text
/// SEG [ '[' occurrence ']' ] [ ('-' | '.') field [ '[' repetition ']' ] [ '.' component [ '.' subcomponent ] ] ]
/// ```
///
/// All indices are one-based, as in the HL7 standard. `occurrence` selects the
/// n-th segment with that name (default 1); `repetition` selects the n-th
/// repetition of the field (default 1). Examples:
///
/// * `MSH-9.1` — message code
/// * `PID-3.1` — first patient identifier
/// * `PID-3[2].1` — second patient identifier
/// * `OBX[3]-5` — observation value of the third OBX
/// * `ZDS-1.1` — study instance UID
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// Segment name, e.g. `PID`.
    pub segment: String,
    /// One-based occurrence of the segment.
    pub occurrence: usize,
    /// One-based field number, `None` to address the whole segment.
    pub field: Option<usize>,
    /// One-based repetition, `None` for the first (or whole field when no
    /// component is given).
    pub repetition: Option<usize>,
    /// One-based component.
    pub component: Option<usize>,
    /// One-based subcomponent.
    pub subcomponent: Option<usize>,
}

impl Path {
    /// Parse a path string.
    pub fn parse(text: &str) -> Result<Path, PathError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        let seg_end = text
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(text.len());
        let segment = &text[..seg_end];
        if segment.len() != 3 {
            return Err(PathError::BadSegment(segment.to_string()));
        }
        let mut path = Path {
            segment: segment.to_ascii_uppercase(),
            occurrence: 1,
            field: None,
            repetition: None,
            component: None,
            subcomponent: None,
        };
        let mut rest = &text[seg_end..];
        if let Some(r) = rest.strip_prefix('[') {
            let (n, r) = bracket(r)?;
            path.occurrence = n;
            rest = r;
        }
        if rest.is_empty() {
            return Ok(path);
        }
        let Some(r) = rest.strip_prefix(['-', '.']) else {
            return Err(PathError::Trailing(rest.to_string()));
        };
        let (n, r) = number(r)?;
        path.field = Some(n);
        rest = r;
        if let Some(r) = rest.strip_prefix('[') {
            let (n, r) = bracket(r)?;
            path.repetition = Some(n);
            rest = r;
        }
        if let Some(r) = rest.strip_prefix('.') {
            let (n, r) = number(r)?;
            path.component = Some(n);
            rest = r;
            if let Some(r) = rest.strip_prefix('.') {
                let (n, r) = number(r)?;
                path.subcomponent = Some(n);
                rest = r;
            }
        }
        if rest.is_empty() {
            Ok(path)
        } else {
            Err(PathError::Trailing(rest.to_string()))
        }
    }
}

fn number(text: &str) -> Result<(usize, &str), PathError> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let digits = &text[..end];
    match digits.parse::<usize>() {
        Ok(n) if n > 0 => Ok((n, &text[end..])),
        _ => Err(PathError::BadIndex(digits.to_string())),
    }
}

fn bracket(text: &str) -> Result<(usize, &str), PathError> {
    let (n, rest) = number(text)?;
    match rest.strip_prefix(']') {
        Some(rest) => Ok((n, rest)),
        None => Err(PathError::Trailing(rest.to_string())),
    }
}

impl std::str::FromStr for Path {
    type Err = PathError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Path::parse(s)
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.segment)?;
        if self.occurrence > 1 {
            write!(f, "[{}]", self.occurrence)?;
        }
        if let Some(field) = self.field {
            write!(f, "-{field}")?;
            if let Some(r) = self.repetition {
                write!(f, "[{r}]")?;
            }
            if let Some(c) = self.component {
                write!(f, ".{c}")?;
                if let Some(s) = self.subcomponent {
                    write!(f, ".{s}")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_path() {
        let p = Path::parse("OBX[3]-5[2].1.4").unwrap();
        assert_eq!(p.segment, "OBX");
        assert_eq!(p.occurrence, 3);
        assert_eq!(p.field, Some(5));
        assert_eq!(p.repetition, Some(2));
        assert_eq!(p.component, Some(1));
        assert_eq!(p.subcomponent, Some(4));
        assert_eq!(p.to_string(), "OBX[3]-5[2].1.4");
    }

    #[test]
    fn short_forms() {
        assert_eq!(Path::parse("pid").unwrap().segment, "PID");
        assert_eq!(
            Path::parse("PID.3.1").unwrap(),
            Path::parse("PID-3.1").unwrap()
        );
        assert_eq!(Path::parse("PID-3").unwrap().field, Some(3));
    }

    #[test]
    fn errors() {
        assert_eq!(Path::parse(""), Err(PathError::Empty));
        assert!(matches!(Path::parse("PI-1"), Err(PathError::BadSegment(_))));
        assert!(matches!(Path::parse("PID-0"), Err(PathError::BadIndex(_))));
        assert!(matches!(Path::parse("PID-x"), Err(PathError::BadIndex(_))));
        assert!(matches!(
            Path::parse("PID-3.1.2.9"),
            Err(PathError::Trailing(_))
        ));
        assert!(matches!(
            Path::parse("PID[2-3"),
            Err(PathError::Trailing(_))
        ));
    }
}
