use chrono::{DateTime, Local, Timelike};

pub struct Clock {
    last_chime: Option<String>,
}

impl Clock {
    pub fn new() -> Self {
        Self { last_chime: None }
    }

    pub fn check(&mut self, now: DateTime<Local>) -> Option<u32> {
        // 毎時00分、最初の2秒以内
        if now.minute() != 0 || now.second() >= 2 {
            return None;
        }

        let key = now.format("%Y-%m-%d %H").to_string();

        if self.last_chime.as_ref() == Some(&key) {
            return None;
        }

        self.last_chime = Some(key);
        Some(hour_count(now.hour()))
    }
}

pub fn hour_count(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        n => n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hour_count() {
        assert_eq!(hour_count(0), 12);
        assert_eq!(hour_count(1), 1);
        assert_eq!(hour_count(12), 12);
        assert_eq!(hour_count(13), 1);
        assert_eq!(hour_count(23), 11);
    }

    #[test]
    fn test_no_duplicate_chime() {
        let mut clock = Clock::new();

        let now = Local::now().with_minute(0).unwrap().with_second(0).unwrap();

        assert!(clock.check(now).is_some());
        assert!(clock.check(now).is_none());
    }
}
