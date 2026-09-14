//! HL7 person names to DICOM PN.
//!
//! The component orders differ, and getting this wrong is a classic
//! interface bug:
//!
//! * HL7 XPN (PID-5, ORC-12 name part): `family^given^middle^suffix^prefix^degree`
//! * HL7 XCN (ORC-12, OBR-16, PV1-8): `id^family^given^middle^suffix^prefix^degree`
//! * DICOM PN (PS3.5 6.2): `family^given^middle^prefix^suffix`
//!
//! So HL7 suffix (XPN.4) goes to PN position 5 and HL7 prefix (XPN.5) to PN
//! position 4. The HL7 family name is an FN type whose first subcomponent
//! is the surname (`von&Neumann` style ownership prefixes live in later
//! subcomponents); only the first is used.

/// A converted name and whether a prefix or suffix had to be moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonName {
    /// `family^given^middle^prefix^suffix`, trailing empty components dropped.
    pub pn: String,
    /// True when a prefix or suffix was present, so the reorder actually
    /// changed something.
    pub reordered: bool,
}

/// From an XPN value (PID-5 and similar): `family^given^middle^suffix^prefix`.
pub fn from_xpn(xpn: &str) -> Option<PersonName> {
    let c: Vec<&str> = xpn.split('^').map(str::trim).collect();
    let get = |i: usize| c.get(i).copied().unwrap_or("");
    build(get(0), get(1), get(2), get(4), get(3))
}

/// From an XCN value (ORC-12, OBR-16, PV1-8): `id^family^given^middle^suffix^prefix`.
pub fn from_xcn(xcn: &str) -> Option<PersonName> {
    let c: Vec<&str> = xcn.split('^').map(str::trim).collect();
    let get = |i: usize| c.get(i).copied().unwrap_or("");
    build(get(1), get(2), get(3), get(5), get(4))
}

fn build(
    family: &str,
    given: &str,
    middle: &str,
    prefix: &str,
    suffix: &str,
) -> Option<PersonName> {
    // FN: surname is the first subcomponent.
    let family = family.split('&').next().unwrap_or("").trim();
    let parts = [family, given, middle, prefix, suffix];
    if parts.iter().all(|p| p.is_empty()) {
        return None;
    }
    let last = parts.iter().rposition(|p| !p.is_empty()).unwrap_or(0);
    let pn = parts[..=last].join("^");
    Some(PersonName {
        pn,
        reordered: !prefix.is_empty() || !suffix.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpn_prefix_and_suffix_swap_positions() {
        let n = from_xpn("Doe^Jane^Marie^Jr^Dr").unwrap();
        assert_eq!(n.pn, "Doe^Jane^Marie^Dr^Jr");
        assert!(n.reordered);
        let plain = from_xpn("Doe^Jane").unwrap();
        assert_eq!(plain.pn, "Doe^Jane");
        assert!(!plain.reordered);
        assert_eq!(from_xpn("Doe^^^^Dr").unwrap().pn, "Doe^^^Dr");
        assert_eq!(from_xpn("Doe^Jane^^PhD").unwrap().pn, "Doe^Jane^^^PhD");
    }

    #[test]
    fn xcn_skips_the_id_component() {
        let n = from_xcn("12345^Curie^Marie^^^Prof").unwrap();
        assert_eq!(n.pn, "Curie^Marie^^Prof");
        assert!(n.reordered);
        assert_eq!(from_xcn("12345^Curie^Marie").unwrap().pn, "Curie^Marie");
        assert!(from_xcn("12345").is_none(), "an id alone is not a name");
    }

    #[test]
    fn family_name_takes_the_surname_subcomponent() {
        assert_eq!(from_xpn("Neumann&von^John").unwrap().pn, "Neumann^John");
        assert!(from_xpn("").is_none());
        assert!(from_xpn("^^^").is_none());
    }
}
