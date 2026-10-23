//! Calendar dates for retirement schedules: parse `YYYY-MM-DD`, count days
//! between two dates and print them for people (`14 Jan 2027`).
//!
//! Dates are proleptic Gregorian days since 1970-01-01 (the civil-date
//! algorithms from Howard Hinnant's `chrono`-compatible date library), so no
//! date crate is needed and day counts are exact across leap years.

use serde::{Serialize, Serializer};
use std::fmt;

/// A calendar date, without a time or time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days: i64,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl Date {
    /// The date for `year-month-day`, or `None` if it does not exist.
    pub fn from_ymd(year: i64, month: u32, day: u32) -> Option<Date> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date {
            days: days_from_civil(year, month, day),
        })
    }

    /// Parses exactly `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Option<Date> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let part = &text[range];
            if part.bytes().all(|b| b.is_ascii_digit()) {
                part.parse().ok()
            } else {
                None
            }
        };
        let year = digits(0..4)?;
        let month = u32::try_from(digits(5..7)?).ok()?;
        let day = u32::try_from(digits(8..10)?).ok()?;
        Date::from_ymd(year, month, day)
    }

    /// Today in UTC, from the system clock.
    pub fn today_utc() -> Date {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Date {
            days: seconds.div_euclid(86_400),
        }
    }

    /// `(year, month, day)`.
    pub fn ymd(self) -> (i64, u32, u32) {
        civil_from_days(self.days)
    }

    /// Days from `self` to `later`: positive when `later` is after `self`.
    pub fn days_until(self, later: Date) -> i64 {
        later.days - self.days
    }

    /// `2027-01-14`.
    pub fn iso(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// `14 Jan 2027`.
    pub fn human(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.iso())
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.iso())
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a valid civil date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    #[test]
    fn parses_and_prints() {
        let date = d("2027-01-14");
        assert_eq!(date.ymd(), (2027, 1, 14));
        assert_eq!(date.iso(), "2027-01-14");
        assert_eq!(date.human(), "14 Jan 2027");
        assert_eq!(d("1970-01-01").days, 0);
        assert_eq!(d("2000-02-29").human(), "29 Feb 2000");
        assert_eq!(
            serde_json::to_string(&d("2026-10-18")).unwrap(),
            "\"2026-10-18\""
        );
    }

    #[test]
    fn rejects_malformed_and_impossible_dates() {
        for bad in [
            "",
            "2026-1-01",
            "2026/10/18",
            "2026-13-01",
            "2026-00-10",
            "2026-02-29",
            "2100-02-29",
            "2026-04-31",
            "20x6-10-18",
            "2026-10-18T00:00",
            "+026-10-18",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
        assert!(Date::parse("2024-02-29").is_some());
        assert!(Date::parse("2000-02-29").is_some());
    }

    #[test]
    fn counts_days_across_months_and_leap_years() {
        // The launch numbers: OpenAI's 14 Jan 2027 shutdown seen on 18 Oct 2026.
        assert_eq!(d("2026-10-18").days_until(d("2027-01-14")), 88);
        assert_eq!(d("2026-10-18").days_until(d("2026-10-23")), 5);
        assert_eq!(d("2026-10-18").days_until(d("2026-06-15")), -125);
        assert_eq!(d("2024-02-28").days_until(d("2024-03-01")), 2);
        assert_eq!(d("2023-02-28").days_until(d("2023-03-01")), 1);
        assert_eq!(d("1999-12-31").days_until(d("2000-01-01")), 1);
        assert_eq!(d("2026-10-18").days_until(d("2026-10-18")), 0);
    }

    #[test]
    fn round_trips_every_day_for_four_centuries() {
        let start = d("1900-01-01");
        let end = d("2300-12-31");
        let mut previous: Option<Date> = None;
        for days in start.days..=end.days {
            let date = Date { days };
            let (y, m, dd) = date.ymd();
            assert_eq!(Date::from_ymd(y, m, dd), Some(date));
            assert_eq!(Date::parse(&date.iso()), Some(date));
            if let Some(prev) = previous {
                assert_eq!(prev.days_until(date), 1);
            }
            previous = Some(date);
        }
    }
}
