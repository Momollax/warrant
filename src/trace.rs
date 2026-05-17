use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

pub fn enabled() -> bool {
    env_flag("OPPORTUNITY_DEBUG") || env_flag("DEBUG") || env_flag("APP_DEBUG")
}

pub fn every(default_value: usize) -> usize {
    std::env::var("OPPORTUNITY_DEBUG_EVERY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_value)
}

pub fn log(message: impl AsRef<str>) {
    if !enabled() {
        return;
    }

    let elapsed = START.get_or_init(Instant::now).elapsed();
    eprintln!(
        "[DEBUG +{:>6.2}s] {}",
        elapsed.as_secs_f64(),
        message.as_ref()
    );
}

pub fn log_progress(done: usize, total: usize, label: &str, current: &str) {
    if !enabled() {
        return;
    }

    let every = every(25);
    if done == 1 || done == total || done % every == 0 {
        log(format!("{label}: {done}/{total} ({current})"));
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}
