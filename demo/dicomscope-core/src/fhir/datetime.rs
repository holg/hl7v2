//! DICOM DA/TM/DT and HL7 DTM to FHIR `date` / `dateTime`.
//!
//! The one rule that matters: a FHIR `dateTime` with a time of day **must**
//! carry a timezone (FHIR R4 datatypes, `dateTime`). DICOM DA and TM carry
//! none, and HL7 DTM usually carries none. So a time is emitted only when
//! an offset is known: (0008,0201) Timezone Offset From UTC for DICOM, the
//! trailing `±ZZZZ` for HL7. Otherwise the date alone is emitted and the
//! time is dropped, which loses information but is valid. Malformed input
//! gives `None` and the element is omitted; a partial string is never
//! emitted.

/// `YYYYMMDD` (or the legacy `YYYY.MM.DD`) to `YYYY-MM-DD`.
pub fn dicom_date(da: &str) -> Option<String> {
    let digits: String = da.trim().chars().filter(|c| *c != '.').collect();
    let (y, m, d) = split_date(&digits)?;
    Some(format!("{y}-{m}-{d}"))
}

/// DICOM DA plus optional TM and (0008,0201) offset to a FHIR `dateTime`
/// when the offset is present, else a FHIR `date`.
pub fn dicom_datetime(da: &str, tm: Option<&str>, offset: Option<&str>) -> Option<String> {
    let date = dicom_date(da)?;
    let (Some(tm), Some(offset)) = (tm, offset) else {
        return Some(date);
    };
    let Some(zone) = zone(offset) else {
        return Some(date);
    };
    match parse_tm(tm) {
        Some((h, m, s, frac)) => Some(format!("{date}T{h}:{m}:{s}{frac}{zone}")),
        // A malformed time must not spoil the date.
        None => Some(date),
    }
}

/// HL7 DTM `YYYY[MM[DD[HH[MM[SS[.S[S[S[S]]]]]]]]][±ZZZZ]` to a FHIR
/// `dateTime` when hours, minutes and an offset are present, else the
/// date at the precision given.
pub fn hl7_datetime(dtm: &str) -> Option<String> {
    let dtm = dtm.trim();
    let (body, zone) = match dtm.find(['+', '-']) {
        Some(i) => (&dtm[..i], zone(&dtm[i..])),
        None => (dtm, None),
    };
    let (main, frac) = match body.find('.') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    if !main.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let date = match main.len() {
        4 => return Some(main.to_string()),
        6 => {
            let m = &main[4..6];
            if !(1..=12).contains(&m.parse::<u8>().ok()?) {
                return None;
            }
            return Some(format!("{}-{m}", &main[..4]));
        }
        8.. => {
            let (y, m, d) = split_date(&main[..8])?;
            format!("{y}-{m}-{d}")
        }
        _ => return None,
    };
    let time = &main[8..];
    let (Some(zone), true) = (zone, time.len() >= 4) else {
        return Some(date);
    };
    let (h, mi, s) = split_time(time)?;
    let frac = frac
        .filter(|f| !f.is_empty() && f.len() <= 4 && f.chars().all(|c| c.is_ascii_digit()))
        .map(|f| format!(".{f}"))
        .unwrap_or_default();
    Some(format!("{date}T{h}:{mi}:{s}{frac}{zone}"))
}

/// The date part of an HL7 DTM, for `birthDate`.
pub fn hl7_date(dtm: &str) -> Option<String> {
    let dtm = dtm.trim();
    let end = dtm.find(['+', '-', '.']).unwrap_or(dtm.len());
    let digits = &dtm[..end.min(8)];
    match digits.len() {
        4 | 6 | 8 => hl7_datetime(digits),
        _ => None,
    }
}

/// `±HHMM` to `±hh:mm`, validated.
pub fn zone(offset: &str) -> Option<String> {
    let offset = offset.trim();
    let (sign, rest) = offset.split_at(offset.chars().next()?.len_utf8());
    if !(sign == "+" || sign == "-") || rest.len() != 4 || !rest.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let h: u8 = rest[..2].parse().ok()?;
    let m: u8 = rest[2..].parse().ok()?;
    if h > 14 || m > 59 {
        return None;
    }
    Some(format!("{sign}{:02}:{:02}", h, m))
}

/// `YYYYMMDD` to validated parts.
fn split_date(digits: &str) -> Option<(&str, &str, &str)> {
    if digits.len() != 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (y, m, d) = (&digits[..4], &digits[4..6], &digits[6..8]);
    let month: u8 = m.parse().ok()?;
    let day: u8 = d.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((y, m, d))
}

/// `HHMMSS`, `HHMM` or `HH` to validated `hh`, `mm`, `ss`.
fn split_time(t: &str) -> Option<(String, String, String)> {
    if !t.chars().all(|c| c.is_ascii_digit()) || t.len() < 2 || t.len() > 6 || t.len() % 2 != 0 {
        return None;
    }
    let h: u8 = t[..2].parse().ok()?;
    let m: u8 = t.get(2..4).map(|s| s.parse().ok()).unwrap_or(Some(0))?;
    let s: u8 = t.get(4..6).map(|s| s.parse().ok()).unwrap_or(Some(0))?;
    if h > 23 || m > 59 || s > 60 {
        return None;
    }
    Some((format!("{h:02}"), format!("{m:02}"), format!("{s:02}")))
}

/// DICOM TM `HH[MM[SS[.F{1,6}]]]` (or legacy `HH:MM:SS`) to parts plus a
/// `.frac` suffix (empty when absent).
fn parse_tm(tm: &str) -> Option<(String, String, String, String)> {
    let tm = tm.trim();
    let (main, frac) = match tm.find('.') {
        Some(i) => (&tm[..i], &tm[i + 1..]),
        None => (tm, ""),
    };
    let main: String = main.chars().filter(|c| *c != ':').collect();
    let (h, m, s) = split_time(&main)?;
    let frac = if frac.is_empty() {
        String::new()
    } else if frac.len() <= 6 && frac.chars().all(|c| c.is_ascii_digit()) {
        format!(".{frac}")
    } else {
        return None;
    };
    Some((h, m, s, frac))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dicom_dates() {
        assert_eq!(dicom_date("20151207").as_deref(), Some("2015-12-07"));
        assert_eq!(dicom_date("2015.12.07").as_deref(), Some("2015-12-07"));
        assert_eq!(dicom_date("20151307"), None);
        assert_eq!(dicom_date("2015120"), None);
        assert_eq!(dicom_date(""), None);
    }

    #[test]
    fn dicom_datetime_needs_an_offset_for_a_time() {
        assert_eq!(
            dicom_datetime("20151207", Some("073153"), None).as_deref(),
            Some("2015-12-07")
        );
        assert_eq!(
            dicom_datetime("20151207", Some("073153"), Some("+0100")).as_deref(),
            Some("2015-12-07T07:31:53+01:00")
        );
        assert_eq!(
            dicom_datetime("20151207", Some("073153.250000"), Some("-0500")).as_deref(),
            Some("2015-12-07T07:31:53.250000-05:00")
        );
        assert_eq!(
            dicom_datetime("20151207", Some("0731"), Some("+0000")).as_deref(),
            Some("2015-12-07T07:31:00+00:00")
        );
        // Bad offset or bad time: the date survives, the time does not.
        assert_eq!(
            dicom_datetime("20151207", Some("073153"), Some("+2500")).as_deref(),
            Some("2015-12-07")
        );
        assert_eq!(
            dicom_datetime("20151207", Some("9999"), Some("+0100")).as_deref(),
            Some("2015-12-07")
        );
        assert_eq!(dicom_datetime("bad", Some("073153"), Some("+0100")), None);
    }

    #[test]
    fn hl7_datetimes() {
        assert_eq!(
            hl7_datetime("20260905113000").as_deref(),
            Some("2026-09-05")
        );
        assert_eq!(
            hl7_datetime("20260905113000+0200").as_deref(),
            Some("2026-09-05T11:30:00+02:00")
        );
        assert_eq!(
            hl7_datetime("202609051130-0330").as_deref(),
            Some("2026-09-05T11:30:00-03:30")
        );
        assert_eq!(
            hl7_datetime("20260905113000.5+0000").as_deref(),
            Some("2026-09-05T11:30:00.5+00:00")
        );
        assert_eq!(hl7_datetime("20260905").as_deref(), Some("2026-09-05"));
        assert_eq!(hl7_datetime("202609").as_deref(), Some("2026-09"));
        assert_eq!(hl7_datetime("2026").as_deref(), Some("2026"));
        assert_eq!(
            hl7_datetime("2026090511+0100").as_deref(),
            Some("2026-09-05"),
            "hour without minute"
        );
        assert_eq!(hl7_datetime("20261305"), None);
        assert_eq!(hl7_datetime("2026x905"), None);
        assert_eq!(hl7_date("19700101").as_deref(), Some("1970-01-01"));
        assert_eq!(
            hl7_date("19700101120000+0100").as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(hl7_date("197001").as_deref(), Some("1970-01"));
        assert_eq!(hl7_date("1970010"), None);
    }

    #[test]
    fn zones() {
        assert_eq!(zone("+0100").as_deref(), Some("+01:00"));
        assert_eq!(zone("-0000").as_deref(), Some("-00:00"));
        assert_eq!(zone("+1400").as_deref(), Some("+14:00"));
        assert_eq!(zone("+1500"), None);
        assert_eq!(zone("0100"), None);
        assert_eq!(zone("+01:00"), None);
        assert_eq!(zone(""), None);
    }
}
