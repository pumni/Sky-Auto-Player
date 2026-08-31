//! Application-level update policy.
//!
//! Network access and verified apply/handoff remain outside this crate. This
//! module owns only the already-established channel, preference, and throttle
//! decisions that can be tested without a network or updater process.

use crate::settings::{UpdateChannel, UpdatePreferences};

pub const RETRY_INTERVAL_S: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPolicy {
    pub include_prerelease: bool,
    pub github_api_path: &'static str,
}

pub fn channel_policy(channel: &UpdateChannel) -> ChannelPolicy {
    match channel {
        UpdateChannel::Stable => ChannelPolicy {
            include_prerelease: false,
            github_api_path: "/releases/latest",
        },
        UpdateChannel::Beta => ChannelPolicy {
            include_prerelease: true,
            github_api_path: "/releases?per_page=10",
        },
    }
}

pub fn should_auto_check(preferences: &UpdatePreferences, now_ts: i64) -> bool {
    if !preferences.auto_check {
        return false;
    }
    let success_elapsed = now_ts.saturating_sub(preferences.last_check_ts);
    if now_ts < preferences.last_check_ts || success_elapsed >= preferences.check_interval_s {
        return true;
    }
    if preferences.last_error_ts != 0 {
        let error_elapsed = now_ts.saturating_sub(preferences.last_error_ts);
        if now_ts < preferences.last_error_ts || error_elapsed >= RETRY_INTERVAL_S {
            return true;
        }
    }
    false
}

pub fn retry_delay(preferences: &UpdatePreferences, now_ts: i64) -> i64 {
    if preferences.last_error_ts == 0 {
        return 0;
    }
    let elapsed = now_ts.saturating_sub(preferences.last_error_ts);
    if now_ts < preferences.last_error_ts || elapsed >= RETRY_INTERVAL_S {
        0
    } else {
        RETRY_INTERVAL_S - elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_and_throttle_match_current_service() {
        assert_eq!(
            channel_policy(&UpdateChannel::Stable).github_api_path,
            "/releases/latest"
        );
        assert!(channel_policy(&UpdateChannel::Beta).include_prerelease);
        let preferences = UpdatePreferences {
            last_check_ts: 1_000,
            ..Default::default()
        };
        assert!(!should_auto_check(&preferences, 1_500));
        assert!(should_auto_check(&preferences, 1_000 + 86_400));
    }

    #[test]
    fn failed_check_uses_short_backoff() {
        let preferences = UpdatePreferences {
            last_error_ts: 1_000,
            ..Default::default()
        };
        assert_eq!(retry_delay(&preferences, 1_100), 200);
        assert!(should_auto_check(&preferences, 1_300));
    }
}
