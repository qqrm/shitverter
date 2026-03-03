use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum QuotaDecision {
    Allowed {
        user_count: u32,
        user_limit: u32,
        global_count: u32,
        global_limit: u32,
        day_index: u64,
    },
    UserLimitExceeded {
        user_count: u32,
        user_limit: u32,
        global_count: u32,
        global_limit: u32,
        day_index: u64,
    },
    GlobalLimitExceeded {
        global_count: u32,
        global_limit: u32,
        day_index: u64,
    },
}

#[derive(Debug)]
pub struct RateLimiter {
    day_index: u64,
    user_daily_limit: u32,
    global_daily_limit: u32,
    global_count: u32,
    user_counts: HashMap<i64, u32>,
}

pub fn utc_day_index(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
        / 86_400
}

impl RateLimiter {
    pub fn new(user_daily_limit: u32, global_daily_limit: u32) -> Self {
        Self {
            day_index: utc_day_index(SystemTime::now()),
            user_daily_limit,
            global_daily_limit,
            global_count: 0,
            user_counts: HashMap::new(),
        }
    }

    pub fn current_day_index(&self) -> u64 {
        self.day_index
    }

    pub fn reset_if_new_day(&mut self, now_day_index: u64) -> bool {
        if now_day_index != self.day_index {
            self.day_index = now_day_index;
            self.global_count = 0;
            self.user_counts.clear();
            return true;
        }
        false
    }

    pub fn check_and_consume(&mut self, user_id: i64, now_day_index: u64) -> QuotaDecision {
        self.reset_if_new_day(now_day_index);

        if self.global_count >= self.global_daily_limit {
            return QuotaDecision::GlobalLimitExceeded {
                global_count: self.global_count,
                global_limit: self.global_daily_limit,
                day_index: self.day_index,
            };
        }

        let user_count = *self.user_counts.get(&user_id).unwrap_or(&0);
        if user_count >= self.user_daily_limit {
            return QuotaDecision::UserLimitExceeded {
                user_count,
                user_limit: self.user_daily_limit,
                global_count: self.global_count,
                global_limit: self.global_daily_limit,
                day_index: self.day_index,
            };
        }

        let new_user_count = user_count + 1;
        let new_global_count = self.global_count + 1;

        self.user_counts.insert(user_id, new_user_count);
        self.global_count = new_global_count;

        QuotaDecision::Allowed {
            user_count: new_user_count,
            user_limit: self.user_daily_limit,
            global_count: new_global_count,
            global_limit: self.global_daily_limit,
            day_index: self.day_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{utc_day_index, QuotaDecision, RateLimiter};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn enforces_per_user_limit() {
        let mut limiter = RateLimiter::new(2, 100);
        let day = 20_000;

        assert!(matches!(
            limiter.check_and_consume(1, day),
            QuotaDecision::Allowed { user_count: 1, .. }
        ));
        assert!(matches!(
            limiter.check_and_consume(1, day),
            QuotaDecision::Allowed { user_count: 2, .. }
        ));
        assert!(matches!(
            limiter.check_and_consume(1, day),
            QuotaDecision::UserLimitExceeded {
                user_count: 2,
                user_limit: 2,
                ..
            }
        ));
    }

    #[test]
    fn enforces_global_limit() {
        let mut limiter = RateLimiter::new(10, 2);
        let day = 20_000;

        assert!(matches!(
            limiter.check_and_consume(1, day),
            QuotaDecision::Allowed {
                global_count: 1,
                ..
            }
        ));
        assert!(matches!(
            limiter.check_and_consume(2, day),
            QuotaDecision::Allowed {
                global_count: 2,
                ..
            }
        ));
        assert!(matches!(
            limiter.check_and_consume(3, day),
            QuotaDecision::GlobalLimitExceeded {
                global_count: 2,
                global_limit: 2,
                ..
            }
        ));
    }

    #[test]
    fn resets_on_new_day() {
        let mut limiter = RateLimiter::new(1, 1);
        let day1 = 20_000;
        let day2 = day1 + 1;

        assert!(matches!(
            limiter.check_and_consume(1, day1),
            QuotaDecision::Allowed { .. }
        ));
        assert!(matches!(
            limiter.check_and_consume(1, day1),
            QuotaDecision::GlobalLimitExceeded { .. }
        ));
        assert!(matches!(
            limiter.check_and_consume(1, day2),
            QuotaDecision::Allowed { .. }
        ));
    }

    #[test]
    fn computes_utc_day_index() {
        let start = UNIX_EPOCH + Duration::from_secs(0);
        let next_day = UNIX_EPOCH + Duration::from_secs(86_400);
        assert_eq!(utc_day_index(start), 0);
        assert_eq!(utc_day_index(next_day), 1);
    }
}
