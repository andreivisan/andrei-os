//! Time helpers. Apple's stores use Core Data timestamps: seconds (f64) since
//! 2001-01-01T00:00:00Z.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

/// Seconds between the Unix epoch and the Core Data epoch (2001-01-01Z).
pub const CORE_DATA_EPOCH_OFFSET: i64 = 978_307_200;

pub fn from_core_data(secs: f64) -> Option<DateTime<Utc>> {
    if !secs.is_finite() {
        return None;
    }
    let whole = secs.floor();
    let nanos = ((secs - whole) * 1e9).round() as u32;
    Utc.timestamp_opt(
        whole as i64 + CORE_DATA_EPOCH_OFFSET,
        nanos.min(999_999_999),
    )
    .single()
}

pub fn iso_utc(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn now_iso() -> String {
    iso_utc(Utc::now())
}

pub fn today(tz: Tz) -> NaiveDate {
    Utc::now().with_timezone(&tz).date_naive()
}

/// Does a Reminders list named after weekdays ("Monday", "Saturday & Sunday")
/// cover `day`? Lists that are not weekday lists return false.
pub fn weekday_list_matches(list: &str, day: Weekday) -> bool {
    let l = list.to_lowercase();
    let name = match day {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    };
    l.split(|c: char| !c.is_alphabetic()).any(|w| w == name)
}

/// Lowercase weekday names covered by today (for SQL filters).
pub fn weekday_name(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_data_epoch() {
        assert_eq!(
            iso_utc(from_core_data(0.0).unwrap()),
            "2001-01-01T00:00:00Z"
        );
        // 2026-11-16T00:00:00Z
        assert_eq!(
            iso_utc(from_core_data(816_480_000.0).unwrap()),
            "2026-11-16T00:00:00Z"
        );
        assert!(from_core_data(f64::NAN).is_none());
    }

    #[test]
    fn weekday_lists() {
        assert!(weekday_list_matches("Monday", Weekday::Mon));
        assert!(weekday_list_matches("Saturday & Sunday", Weekday::Sun));
        assert!(weekday_list_matches("Saturday & Sunday", Weekday::Sat));
        assert!(!weekday_list_matches("Inbox", Weekday::Mon));
        assert!(!weekday_list_matches("Mondays-ish", Weekday::Mon));
    }
}
