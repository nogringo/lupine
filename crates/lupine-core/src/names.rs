//! Node names: validation, suffixes for conflict copies and collisions, and
//! the case-insensitive comparison that decides collisions.

use std::cmp::Ordering;

use focaccia::CaseFold;

use crate::error::{Error, Result};

pub const MAX_NAME_BYTES: usize = 255;
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// Names come from the network and end up as path components on disk.
pub fn validate(name: &str) -> Result<()> {
    let invalid = name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name == "."
        || name == ".."
        || name.contains(['/', '\0']);
    if invalid {
        return Err(Error::Malformed("invalid node name"));
    }
    Ok(())
}

/// `(stem, extension)`. The extension starts at the last `.`, unless that
/// `.` is the first character.
pub fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `<stem><suffix><extension>`, with the stem cut at a character boundary
/// to stay within 255 bytes. An extension too long to leave room for the
/// suffix counts as part of the stem.
pub fn with_suffix(name: &str, suffix: &str) -> String {
    let (mut stem, mut extension) = split_extension(name);
    if suffix.len() + extension.len() >= MAX_NAME_BYTES {
        (stem, extension) = (name, "");
    }
    let stem = truncate(stem, MAX_NAME_BYTES - suffix.len() - extension.len());
    format!("{stem}{suffix}{extension}")
}

/// Name of the conflict copy of a head written at `time` (Unix ms).
pub fn conflict_copy(name: &str, time: i64) -> String {
    with_suffix(name, &format!(" (conflict {})", utc_date(time)))
}

/// Name a colliding node takes: `<stem> (n)<extension>`, `n` from 2.
pub fn numbered(name: &str, n: u32) -> String {
    with_suffix(name, &format!(" ({n})"))
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let end = (0..=max)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    &text[..end]
}

/// `YYYY-MM-DD` in UTC.
fn utc_date(millis: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let days = millis.div_euclid(DAY_MILLIS) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A name ordered by Unicode default (full) case folding, so names that a
/// case-insensitive file system would confuse compare equal. Names are
/// already NFC.
#[derive(Clone, Debug)]
pub struct Folded(pub String);

impl Ord for Folded {
    fn cmp(&self, other: &Self) -> Ordering {
        CaseFold::Full.casecmp(&self.0, &other.0)
    }
}

impl PartialOrd for Folded {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Folded {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Folded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_names() {
        for name in ["", ".", "..", "a/b", "nul\0", &"x".repeat(256)] {
            assert!(validate(name).is_err(), "{name:?}");
        }
        validate("..hidden").unwrap();
        validate(&"x".repeat(255)).unwrap();
    }

    #[test]
    fn extensions() {
        assert_eq!(split_extension("photo.jpeg"), ("photo", ".jpeg"));
        assert_eq!(split_extension("archive.tar.gz"), ("archive.tar", ".gz"));
        assert_eq!(split_extension(".bashrc"), (".bashrc", ""));
        assert_eq!(split_extension("Makefile"), ("Makefile", ""));
        assert_eq!(split_extension("trailing."), ("trailing", "."));
    }

    #[test]
    fn suffixes_go_before_the_extension() {
        assert_eq!(numbered("report.pdf", 2), "report (2).pdf");
        assert_eq!(numbered(".env", 3), ".env (3)");
        assert_eq!(
            conflict_copy("notes.txt", 1_763_856_000_000),
            "notes (conflict 2025-11-23).txt"
        );
    }

    #[test]
    fn suffixed_names_stay_within_the_limit() {
        let long = format!("{}.txt", "é".repeat(126));
        let numbered = numbered(&long, 12);
        assert!(numbered.len() <= MAX_NAME_BYTES);
        assert!(numbered.ends_with("é (12).txt"));
        validate(&numbered).unwrap();

        let long_extension = format!("a.{}", "x".repeat(250));
        let numbered = super::numbered(&long_extension, 2);
        assert_eq!(numbered.len(), MAX_NAME_BYTES);
        assert!(numbered.ends_with(" (2)"));
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(-1), "1969-12-31");
        assert_eq!(utc_date(951_782_400_000), "2000-02-29");
        assert_eq!(utc_date(4_107_542_399_999), "2100-02-28");
    }

    #[test]
    fn folding_matches_case_insensitive_file_systems() {
        let folded = |s: &str| Folded(s.to_owned());
        assert_eq!(folded("Photos"), folded("PHOTOS"));
        assert_eq!(folded("Straße"), folded("STRASSE"));
        assert_eq!(folded("Ωmega"), folded("ωMEGA"));
        assert_ne!(folded("a"), folded("b"));
    }
}
