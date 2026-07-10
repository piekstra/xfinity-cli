//! Minimal date helpers — no calendar crate. Enough to format "today" and
//! "yesterday" as `MM/DD/YYYY` for payment dates and usage windows.

use std::time::{SystemTime, UNIX_EPOCH};

/// A civil date, produced from the system clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i64,
    pub month: i64,
    pub day: i64,
}

fn days_since_epoch(offset_days: i64) -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs / 86_400 + offset_days
}

/// Convert a day count since 1970-01-01 to a civil date
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    Date {
        year: if m <= 2 { y + 1 } else { y },
        month: m,
        day: d,
    }
}

pub fn today() -> Date {
    civil_from_days(days_since_epoch(0))
}

/// `MM/DD/YYYY` — the format Xfinity's payment form uses.
pub fn fmt_mm_dd_yyyy(d: Date) -> String {
    format!("{:02}/{:02}/{:04}", d.month, d.day, d.year)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_day_zero_is_1970() {
        assert_eq!(
            civil_from_days(0),
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
        );
    }

    #[test]
    fn formats_padded() {
        let d = Date {
            year: 2026,
            month: 3,
            day: 7,
        };
        assert_eq!(fmt_mm_dd_yyyy(d), "03/07/2026");
    }
}
