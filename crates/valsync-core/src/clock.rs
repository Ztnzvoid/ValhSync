//! Timestamps. UTC everywhere; a manifest is compared across machines.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

/// Current time as RFC 3339 (`2026-09-10T18:00:00Z`).
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Filesystem-friendly stamp for backup and quarantine folders
/// (`20260910-180000`). Sorts chronologically as text.
pub fn dir_stamp() -> String {
    let fmt = format_description!("[year][month][day]-[hour][minute][second]");
    OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_else(|_| "19700101-000000".to_string())
}

/// Is this a well-formed RFC 3339 timestamp?
pub fn is_rfc3339(text: &str) -> bool {
    OffsetDateTime::parse(text, &Rfc3339).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_parse_back() {
        assert!(is_rfc3339(&now_rfc3339()));
        assert!(!is_rfc3339("yesterday"));
        let stamp = dir_stamp();
        assert_eq!(stamp.len(), 15);
        assert_eq!(&stamp[8..9], "-");
    }
}
