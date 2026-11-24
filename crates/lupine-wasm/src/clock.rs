//! The clock nostr-sdk reads, which the browser provides on the web.

use std::sync::OnceLock;

use universal_time::{Instant, MonotonicClock, SystemTime, WallClock, define_time_provider};

struct BrowserClock;

impl WallClock for BrowserClock {
    fn system_time(&self) -> SystemTime {
        let since_epoch = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default();
        SystemTime::from_unix_duration(since_epoch)
    }
}

impl MonotonicClock for BrowserClock {
    fn instant(&self) -> Instant {
        static ORIGIN: OnceLock<web_time::Instant> = OnceLock::new();
        Instant::from_ticks(ORIGIN.get_or_init(web_time::Instant::now).elapsed())
    }
}

define_time_provider!(BrowserClock);
