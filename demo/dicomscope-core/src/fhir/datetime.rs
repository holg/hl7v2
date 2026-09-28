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
//!
//! What counts as malformed differs by source. DICOM DA, TM and (0008,0201)
//! are three attributes: a bad TM or offset drops the time and keeps the
//! date, a bad DA drops everything. An HL7 DTM is one value: a bad zone,
//! an out-of-range time or a fraction that is not `.S{1,4}` makes the
//! whole value malformed and it is refused, date and all. `2026-09-05` is
//! therefore `None`, not the year 2026 with a broken zone.

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
        // A sign announces a zone; a zone that does not parse is not a
        // zone, and the value is not a DTM.
        Some(i) => (&dtm[..i], Some(zone(&dtm[i..])?)),
        None => (dtm, None),
    };
    let (main, frac) = match body.find('.') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    if !main.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // A fraction is `.S` to `.SSSS` and only ever follows seconds.
    if let Some(f) = frac {
        if f.is_empty() || f.len() > 4 || !f.chars().all(|c| c.is_ascii_digit()) || main.len() < 14
        {
            return None;
        }
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
    // A time that is present must be a time (`HH`, `HHMM`, `HHMMSS` in
    // range) whether or not it can be emitted: `20260905240000` is not a
    // date with a bad time, it is a malformed DTM.
    if !time.is_empty() {
        split_time(time)?;
    }
    let (Some(zone), true) = (zone, time.len() >= 4) else {
        return Some(date);
    };
    let (h, mi, s) = split_time(time)?;
    let frac = frac.map(|f| format!(".{f}")).unwrap_or_default();
    Some(format!("{date}T{h}:{mi}:{s}{frac}{zone}"))
}

/// The date part of an HL7 DTM, for `birthDate`: the whole value must be
/// a DTM, then the time (if any was emitted) is cut off.
pub fn hl7_date(dtm: &str) -> Option<String> {
    let full = hl7_datetime(dtm)?;
    Some(match full.find('T') {
        Some(i) => full[..i].to_string(),
        None => full,
    })
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

/// `YYYYMMDD` to validated parts: a real calendar date, leap years
/// included, so `20230229` and `20240431` are refused rather than passed
/// on for a validator to reject later.
fn split_date(digits: &str) -> Option<(&str, &str, &str)> {
    if digits.len() != 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (y, m, d) = (&digits[..4], &digits[4..6], &digits[6..8]);
    let year: u16 = y.parse().ok()?;
    let month: u8 = m.parse().ok()?;
    let day: u8 = d.parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((y, m, d))
}

/// Proleptic Gregorian, as FHIR and DICOM both assume.
pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
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
    let frac = match tm.find('.') {
        // `.F{1,6}` after the seconds; a bare dot or more digits is not TM.
        Some(_)
            if (1..=6).contains(&frac.len())
                && frac.chars().all(|c| c.is_ascii_digit())
                && main.len() == 6 =>
        {
            format!(".{frac}")
        }
        Some(_) => return None,
        None => String::new(),
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
        assert_eq!(dicom_date(" 20151207 ").as_deref(), Some("2015-12-07"));
        assert_eq!(dicom_date("20151307"), None);
        assert_eq!(dicom_date("2015120"), None);
        assert_eq!(dicom_date("201512070"), None);
        assert_eq!(dicom_date("2015-12-07"), None, "ISO is not DA");
        assert_eq!(dicom_date("20151200"), None);
        assert_eq!(dicom_date("20150007"), None);
        assert_eq!(dicom_date("00000101"), None, "year 0 does not exist");
        assert_eq!(dicom_date(""), None);
    }

    #[test]
    fn calendar_days_are_checked_per_month_with_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(1900, 2), 28, "century rule");
        assert_eq!(days_in_month(2000, 2), 29, "400-year rule");
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 12), 31);
        assert_eq!(days_in_month(2024, 13), 0);
        assert_eq!(days_in_month(2024, 0), 0);

        assert_eq!(dicom_date("20240229").as_deref(), Some("2024-02-29"));
        assert_eq!(dicom_date("20230229"), None);
        assert_eq!(dicom_date("19000229"), None);
        assert_eq!(dicom_date("20000229").as_deref(), Some("2000-02-29"));
        assert_eq!(dicom_date("20240230"), None);
        assert_eq!(dicom_date("20240431"), None);
        assert_eq!(dicom_date("20240631"), None);
        assert_eq!(dicom_date("20240931"), None);
        assert_eq!(dicom_date("20241131"), None);
        assert_eq!(dicom_date("20241231").as_deref(), Some("2024-12-31"));
        assert_eq!(dicom_date("20240132"), None);
        // Same rules for HL7 DTM.
        assert_eq!(hl7_datetime("20230229"), None);
        assert_eq!(
            hl7_datetime("20240229120000+0100").as_deref(),
            Some("2024-02-29T12:00:00+01:00")
        );
        assert_eq!(hl7_date("20230230"), None);
    }

    #[test]
    fn dicom_times_at_every_precision_and_legacy_form() {
        let dt = |tm: &str| dicom_datetime("20151207", Some(tm), Some("+0000"));
        assert_eq!(dt("07").as_deref(), Some("2015-12-07T07:00:00+00:00"));
        assert_eq!(dt("0731").as_deref(), Some("2015-12-07T07:31:00+00:00"));
        assert_eq!(dt("073153").as_deref(), Some("2015-12-07T07:31:53+00:00"));
        assert_eq!(
            dt("073153.1").as_deref(),
            Some("2015-12-07T07:31:53.1+00:00")
        );
        assert_eq!(
            dt("073153.123456").as_deref(),
            Some("2015-12-07T07:31:53.123456+00:00")
        );
        assert_eq!(
            dt("07:31:53").as_deref(),
            Some("2015-12-07T07:31:53+00:00"),
            "ACR-NEMA form"
        );
        assert_eq!(dt("07:31").as_deref(), Some("2015-12-07T07:31:00+00:00"));
        assert_eq!(
            dt(" 073153 ").as_deref(),
            Some("2015-12-07T07:31:53+00:00"),
            "padded"
        );
        assert_eq!(
            dt("235960").as_deref(),
            Some("2015-12-07T23:59:60+00:00"),
            "leap second"
        );
        // Malformed times drop to the date, never a partial dateTime.
        for bad in [
            "",
            "7",
            "073",
            "07315",
            "0731530",
            "240000",
            "076000",
            "073161",
            "073153.1234567",
            "073153.",
            "073153.1a",
            "0731ab",
            "07.31.53",
        ] {
            assert_eq!(dt(bad).as_deref(), Some("2015-12-07"), "{bad:?}");
        }
        // A time without an offset is dropped even when well formed.
        assert_eq!(
            dicom_datetime("20151207", Some("073153"), None).as_deref(),
            Some("2015-12-07")
        );
        assert_eq!(
            dicom_datetime("20151207", None, Some("+0100")).as_deref(),
            Some("2015-12-07")
        );
        assert_eq!(
            dicom_datetime("20151207", None, None).as_deref(),
            Some("2015-12-07")
        );
    }

    #[test]
    fn hl7_precision_ladder() {
        assert_eq!(hl7_datetime("2026").as_deref(), Some("2026"));
        assert_eq!(hl7_datetime("202609").as_deref(), Some("2026-09"));
        assert_eq!(hl7_datetime("20260905").as_deref(), Some("2026-09-05"));
        assert_eq!(
            hl7_datetime("2026090511").as_deref(),
            Some("2026-09-05"),
            "hour only, no zone"
        );
        assert_eq!(
            hl7_datetime("202609051130").as_deref(),
            Some("2026-09-05"),
            "no zone: date only"
        );
        assert_eq!(
            hl7_datetime("20260905113000").as_deref(),
            Some("2026-09-05")
        );
        assert_eq!(
            hl7_datetime("20260905113000.1234").as_deref(),
            Some("2026-09-05")
        );
        assert_eq!(
            hl7_datetime("202609051130+0000").as_deref(),
            Some("2026-09-05T11:30:00+00:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000+0000").as_deref(),
            Some("2026-09-05T11:30:00+00:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000.1+0000").as_deref(),
            Some("2026-09-05T11:30:00.1+00:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000.1234+0000").as_deref(),
            Some("2026-09-05T11:30:00.1234+00:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000.12345+0000"),
            None,
            "5 fraction digits are not DTM: refused, not truncated"
        );
        assert_eq!(hl7_datetime("20260905113000.+0000"), None, "empty fraction");
        assert_eq!(hl7_datetime("20260905113000.1a+0000"), None);
        assert_eq!(
            hl7_datetime("202609051130.5+0000"),
            None,
            "a fraction needs seconds"
        );
        assert_eq!(hl7_datetime("20260905.5"), None);
        // Odd lengths and non-digits are refused outright.
        for bad in [
            "",
            "2",
            "202",
            "20260",
            "2026090",
            "202609051",
            "2026090511300",
            "202609051130001",
            "2026-09-05",
            "2026x905",
            "20260905T113000",
            "+0100",
            "20261305",
            "20260931",
            "202600",
            "202613",
            "00000101",
        ] {
            assert_eq!(hl7_datetime(bad), None, "{bad:?}");
        }
        assert_eq!(
            hl7_datetime("20260905113000+0100").as_deref(),
            Some("2026-09-05T11:30:00+01:00")
        );
        assert_eq!(
            hl7_datetime("  20260905113000+0100  ").as_deref(),
            Some("2026-09-05T11:30:00+01:00")
        );
    }

    #[test]
    fn hl7_time_of_day_is_range_checked() {
        assert_eq!(
            hl7_datetime("20260905240000+0000").as_deref(),
            None,
            "hour 24"
        );
        assert_eq!(
            hl7_datetime("20260905116000+0000").as_deref(),
            None,
            "minute 60"
        );
        assert_eq!(
            hl7_datetime("20260905113061+0000").as_deref(),
            None,
            "second 61"
        );
        assert_eq!(
            hl7_datetime("20260905235960+0000").as_deref(),
            Some("2026-09-05T23:59:60+00:00"),
            "leap second allowed"
        );
        assert_eq!(
            hl7_datetime("20260905000000+0000").as_deref(),
            Some("2026-09-05T00:00:00+00:00")
        );
    }

    #[test]
    fn hl7_zone_edge_cases() {
        assert_eq!(
            hl7_datetime("20260905113000+1400").as_deref(),
            Some("2026-09-05T11:30:00+14:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000-1200").as_deref(),
            Some("2026-09-05T11:30:00-12:00")
        );
        assert_eq!(
            hl7_datetime("20260905113000+0530").as_deref(),
            Some("2026-09-05T11:30:00+05:30")
        );
        assert_eq!(
            hl7_datetime("20260905113000+0930").as_deref(),
            Some("2026-09-05T11:30:00+09:30")
        );
        assert_eq!(
            hl7_datetime("20260905113000+0545").as_deref(),
            Some("2026-09-05T11:30:00+05:45"),
            "Nepal"
        );
        assert_eq!(
            hl7_datetime("20260905113000+1245").as_deref(),
            Some("2026-09-05T11:30:00+12:45"),
            "Chatham"
        );
        assert_eq!(
            hl7_datetime("20260905113000-0000").as_deref(),
            Some("2026-09-05T11:30:00-00:00")
        );
        // A sign that is not followed by a zone makes the value malformed:
        // refused whole, not read as a date with a broken tail.
        for bad in [
            "+1500", "-1500", "+0160", "+01", "+010", "+01000", "+01:00", "+ab00", "+", "-",
        ] {
            assert_eq!(
                hl7_datetime(&format!("20260905113000{bad}")),
                None,
                "{bad:?}"
            );
            assert_eq!(hl7_datetime(&format!("20260905{bad}")), None, "{bad:?}");
        }
        // The first sign starts the zone: a second sign makes it invalid.
        assert_eq!(hl7_datetime("20260905113000+01-00"), None);
        // A zone without a time is fine: the date is emitted.
        assert_eq!(hl7_datetime("20260905+0100").as_deref(), Some("2026-09-05"));
        assert_eq!(
            hl7_datetime("2026-0100").as_deref(),
            Some("2026"),
            "year with a zone"
        );
    }

    #[test]
    fn hl7_dates_for_birth_date() {
        assert_eq!(hl7_date("1970").as_deref(), Some("1970"));
        assert_eq!(hl7_date("197001").as_deref(), Some("1970-01"));
        assert_eq!(hl7_date("19700101").as_deref(), Some("1970-01-01"));
        assert_eq!(hl7_date("1970010112").as_deref(), Some("1970-01-01"));
        assert_eq!(
            hl7_date("19700101120000.5+0100").as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(hl7_date("19700101.5"), None, "a fraction without seconds");
        assert_eq!(hl7_date("19700101-0500").as_deref(), Some("1970-01-01"));
        assert_eq!(hl7_date("19700101-9999"), None, "bad zone: not a DTM");
        assert_eq!(hl7_date("19700229"), None, "1970 was not a leap year");
        assert_eq!(hl7_date("197013"), None);
        assert_eq!(hl7_date("19"), None);
        assert_eq!(hl7_date(""), None);
        assert_eq!(hl7_date("1970-01-01"), None, "ISO is not DTM");
        assert_eq!(hl7_date("\"\""), None, "HL7 null value");
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
        assert_eq!(zone("+0000").as_deref(), Some("+00:00"));
        assert_eq!(zone("+1400").as_deref(), Some("+14:00"));
        assert_eq!(zone("-1200").as_deref(), Some("-12:00"));
        assert_eq!(zone("+0530").as_deref(), Some("+05:30"));
        assert_eq!(zone("-0330").as_deref(), Some("-03:30"));
        assert_eq!(zone("+1459").as_deref(), Some("+14:59"));
        assert_eq!(zone(" +0100 ").as_deref(), Some("+01:00"));
        assert_eq!(zone("+1500"), None);
        assert_eq!(zone("-1500"), None);
        assert_eq!(zone("+0060"), None);
        assert_eq!(zone("0100"), None);
        assert_eq!(zone("+01:00"), None);
        assert_eq!(zone("+010"), None);
        assert_eq!(zone("+01000"), None);
        assert_eq!(zone("Z"), None, "DICOM and HL7 have no Z");
        assert_eq!(zone("+"), None);
        assert_eq!(zone("±0100"), None, "non-ASCII sign");
        assert_eq!(zone("+١٢٠٠"), None, "non-ASCII digits");
        assert_eq!(zone(""), None);
    }

    #[test]
    fn parsers_do_not_panic_on_multibyte_input() {
        // Byte slicing on char boundaries would panic on these.
        for s in [
            "éééééééé",
            "2026é905",
            "20260905é",
            "日本語の日付",
            "2026-é",
            "é",
            "20260905113000+é100",
            "20260905113000.é+0100",
        ] {
            let _ = hl7_datetime(s);
            let _ = hl7_date(s);
            let _ = dicom_date(s);
            let _ = dicom_datetime(s, Some(s), Some(s));
            let _ = dicom_datetime("20260905", Some(s), Some("+0100"));
            let _ = dicom_datetime("20260905", Some("1200"), Some(s));
            let _ = zone(s);
        }
    }
}
