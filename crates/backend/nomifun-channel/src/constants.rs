use std::time::Duration;

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

/// Length of the numeric pairing code (6 digits).
pub const PAIRING_CODE_LENGTH: usize = 6;

/// How long a pairing code remains valid.
pub const PAIRING_CODE_TTL: Duration = Duration::from_secs(10 * 60);

/// Interval between expired-pairing cleanup sweeps.
pub const PAIRING_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Platform message limits
// ---------------------------------------------------------------------------

/// Maximum characters per Telegram message.
pub const TELEGRAM_MESSAGE_LIMIT: usize = 4096;

/// Maximum characters per Lark (Feishu) message.
pub const LARK_MESSAGE_LIMIT: usize = 4000;

/// Maximum characters per DingTalk message.
pub const DINGTALK_MESSAGE_LIMIT: usize = 4000;

// ---------------------------------------------------------------------------
// Plugin watchdog
// ---------------------------------------------------------------------------

/// Interval between plugin health sweeps in the `ChannelManager` watchdog.
pub const WATCHDOG_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Sliding window for watchdog restart-rate limiting.
pub const WATCHDOG_RESTART_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Maximum automatic restart attempts per plugin within
/// `WATCHDOG_RESTART_WINDOW`.
pub const WATCHDOG_MAX_RESTARTS_PER_WINDOW: u32 = 3;

/// Base delay for exponential backoff between watchdog restart attempts
/// (delay after the n-th attempt = base * 2^(n-1)).
pub const WATCHDOG_BACKOFF_BASE: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Reconnection (shared by all long-connection plugins)
// ---------------------------------------------------------------------------

/// Maximum consecutive reconnection attempts before a plugin loop gives up.
pub const RECONNECT_MAX_ATTEMPTS: u32 = 10;

/// Maximum delay between reconnection attempts (exponential backoff cap).
pub const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Lark
// ---------------------------------------------------------------------------

/// TTL for Lark event deduplication cache.
pub const LARK_EVENT_DEDUP_TTL: Duration = Duration::from_secs(5 * 60);

// ---------------------------------------------------------------------------
// WeChat (iLink Bot)
// ---------------------------------------------------------------------------

/// Maximum consecutive failures before WeChat applies longer backoff.
pub const WEIXIN_MAX_RETRIES: u32 = 3;

/// Short retry delay between WeChat poll attempts on failure.
pub const WEIXIN_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Long-polling timeout for WeChat getupdates (matches iLink API).
pub const WEIXIN_POLL_TIMEOUT: Duration = Duration::from_secs(35);

/// Timeout for non-polling WeChat API calls.
pub const WEIXIN_API_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

/// Maximum characters per Discord message.
pub const DISCORD_MESSAGE_LIMIT: usize = 2000;

// ---------------------------------------------------------------------------
// Mattermost
// ---------------------------------------------------------------------------

/// Maximum characters per Mattermost post (hard limit ~65535 bytes; rendering
/// degrades well before that, so cap conservatively).
pub const MATTERMOST_MESSAGE_LIMIT: usize = 16000;

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------

/// Maximum characters per Matrix message (`m.text` body).
pub const MATRIX_MESSAGE_LIMIT: usize = 65536;

/// Timeout for Matrix /sync long-poll (the `timeout` query parameter, in ms).
pub const MATRIX_SYNC_TIMEOUT_MS: u32 = 30_000;

/// Timeout for non-sync Matrix API calls (send, edit, whoami).
pub const MATRIX_API_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Slack (Socket Mode)
// ---------------------------------------------------------------------------

/// Maximum characters per Slack message.
pub const SLACK_MESSAGE_LIMIT: usize = 3500;

// ---------------------------------------------------------------------------
// Twitch (IRC-over-WebSocket)
// ---------------------------------------------------------------------------

/// Maximum characters per Twitch IRC PRIVMSG (~500 char IRC limit; margin).
pub const TWITCH_MESSAGE_LIMIT: usize = 480;

// ---------------------------------------------------------------------------
// QQ Bot (official gateway WS + REST)
// ---------------------------------------------------------------------------

/// Maximum characters per QQ Bot message.
pub const QQBOT_MESSAGE_LIMIT: usize = 4000;

/// Maximum passive replies per inbound msg_id (QQ API hard limit: 5).
pub const QQBOT_PASSIVE_REPLY_MAX: u32 = 5;

/// Passive-reply window TTL per inbound msg_id (QQ API hard limit: 1 hour).
pub const QQBOT_PASSIVE_REPLY_WINDOW: Duration = Duration::from_secs(3600);


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_length_is_six() {
        assert_eq!(PAIRING_CODE_LENGTH, 6);
    }

    #[test]
    fn pairing_code_ttl_is_ten_minutes() {
        assert_eq!(PAIRING_CODE_TTL, Duration::from_secs(600));
    }

    #[test]
    fn cleanup_interval_is_sixty_seconds() {
        assert_eq!(PAIRING_CLEANUP_INTERVAL, Duration::from_secs(60));
    }

    #[test]
    fn telegram_message_limit() {
        assert_eq!(TELEGRAM_MESSAGE_LIMIT, 4096);
    }

    #[test]
    fn lark_message_limit() {
        assert_eq!(LARK_MESSAGE_LIMIT, 4000);
    }

    #[test]
    fn dingtalk_message_limit() {
        assert_eq!(DINGTALK_MESSAGE_LIMIT, 4000);
    }

    #[test]
    fn reconnect_limits() {
        assert_eq!(RECONNECT_MAX_ATTEMPTS, 10);
        assert_eq!(RECONNECT_MAX_DELAY, Duration::from_secs(30));
    }

    #[test]
    fn watchdog_constants() {
        assert_eq!(WATCHDOG_SWEEP_INTERVAL, Duration::from_secs(60));
        assert_eq!(WATCHDOG_RESTART_WINDOW, Duration::from_secs(3600));
        assert_eq!(WATCHDOG_MAX_RESTARTS_PER_WINDOW, 3);
        assert_eq!(WATCHDOG_BACKOFF_BASE, Duration::from_secs(60));
    }

    #[test]
    fn lark_event_dedup_ttl_is_five_minutes() {
        assert_eq!(LARK_EVENT_DEDUP_TTL, Duration::from_secs(300));
    }

    #[test]
    fn weixin_constants() {
        assert_eq!(WEIXIN_MAX_RETRIES, 3);
        assert_eq!(WEIXIN_RETRY_DELAY, Duration::from_secs(2));
        assert_eq!(WEIXIN_POLL_TIMEOUT, Duration::from_secs(35));
        assert_eq!(WEIXIN_API_TIMEOUT, Duration::from_secs(15));
    }
}
