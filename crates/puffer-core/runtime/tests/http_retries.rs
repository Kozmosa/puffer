use std::time::Duration;

struct ScopedEnvVar {
    name: &'static str,
    old_value: Option<String>,
}

impl ScopedEnvVar {
    fn set(name: &'static str, value: &str) -> Self {
        let old_value = std::env::var(name).ok();
        std::env::set_var(name, value);
        Self { name, old_value }
    }

    fn unset(name: &'static str) -> Self {
        let old_value = std::env::var(name).ok();
        std::env::remove_var(name);
        Self { name, old_value }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(value) = self.old_value.take() {
            std::env::set_var(self.name, value);
        } else {
            std::env::remove_var(self.name);
        }
    }
}

#[test]
fn http_retry_config_defaults_to_no_retries() {
    let _lock = super::refresh_env_lock().lock().unwrap();
    let _attempts = ScopedEnvVar::unset(super::super::HTTP_RETRY_ATTEMPTS_ENV);
    let _delay = ScopedEnvVar::unset(super::super::HTTP_RETRY_DELAY_MS_ENV);

    assert_eq!(
        super::super::http_retry_config(),
        super::super::HttpRetryConfig {
            retries: 0,
            delay_ms: 1_000,
        }
    );
}

#[test]
fn http_retry_config_reads_and_clamps_env_values() {
    let _lock = super::refresh_env_lock().lock().unwrap();
    let _attempts = ScopedEnvVar::set(super::super::HTTP_RETRY_ATTEMPTS_ENV, "99");
    let _delay = ScopedEnvVar::set(super::super::HTTP_RETRY_DELAY_MS_ENV, "999999");

    assert_eq!(
        super::super::http_retry_config(),
        super::super::HttpRetryConfig {
            retries: 10,
            delay_ms: 30_000,
        }
    );
}

#[test]
fn retry_delay_scales_with_attempt_number() {
    let config = super::super::HttpRetryConfig {
        retries: 5,
        delay_ms: 250,
    };

    assert_eq!(
        super::super::retry_delay(config, 3),
        Duration::from_millis(750)
    );
}

#[test]
fn retryable_http_error_accepts_timeout_io_errors() {
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "timed out",
    ));

    assert!(super::super::is_retryable_http_error(&error));
}

#[test]
fn retryable_http_error_rejects_invalid_data_io_errors() {
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "bad payload",
    ));

    assert!(!super::super::is_retryable_http_error(&error));
}
