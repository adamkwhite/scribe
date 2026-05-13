use crate::audio::AudioSessionTimestamp;

pub trait RuntimeClock: Send + Sync {
    fn recording_timestamp(&self) -> AudioSessionTimestamp;
    fn note_date(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalRuntimeClock;

impl RuntimeClock for LocalRuntimeClock {
    fn recording_timestamp(&self) -> AudioSessionTimestamp {
        AudioSessionTimestamp::now_local()
    }

    fn note_date(&self) -> String {
        chrono::Local::now().format("%B %-d, %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_timestamp_matches_yyyy_mm_dd_hhmmss_format() {
        let clock = LocalRuntimeClock;
        let ts = clock.recording_timestamp();
        let s = ts.as_str();

        // Format: "YYYY-MM-DD_HHMMSS" — 17 chars, with separators in fixed slots.
        assert_eq!(s.len(), 17, "got: {s}");
        assert!(s.chars().nth(4) == Some('-'), "got: {s}");
        assert!(s.chars().nth(7) == Some('-'), "got: {s}");
        assert!(s.chars().nth(10) == Some('_'), "got: {s}");
        for (i, c) in s.chars().enumerate() {
            if matches!(i, 4 | 7 | 10) {
                continue;
            }
            assert!(c.is_ascii_digit(), "non-digit '{c}' at index {i} in: {s}");
        }
    }

    #[test]
    fn note_date_contains_current_year_and_comma() {
        let clock = LocalRuntimeClock;
        let date = clock.note_date();

        let year = chrono::Local::now().format("%Y").to_string();
        assert!(date.contains(&year), "missing year in: {date}");
        assert!(
            date.contains(", "),
            "missing 'Month D, YYYY' comma in: {date}"
        );
    }
}
