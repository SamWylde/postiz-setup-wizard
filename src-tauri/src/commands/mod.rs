pub mod bootstrap;
pub mod diagnostics;
pub mod docker;
pub mod env_file;
pub mod import;
pub mod install;
pub mod resume;
pub mod secrets;
pub mod snapshot;
pub mod transfer;
pub mod tunnel;
pub mod updater;

/// Shared mapping of env-file keys to provider IDs.
/// Used by import, snapshot, and transfer to detect configured providers.
/// Note: Instagram uses the same Facebook App keys (FACEBOOK_APP_ID/SECRET),
/// so it is not listed separately.
pub const PROVIDER_ENV_KEYS: &[(&str, &str)] = &[
    ("X_API_KEY", "x"),
    ("FACEBOOK_APP_ID", "facebook"),
    ("LINKEDIN_CLIENT_ID", "linkedin"),
    ("REDDIT_CLIENT_ID", "reddit"),
    ("THREADS_APP_ID", "threads"),
    ("YOUTUBE_CLIENT_ID", "youtube"),
    ("TIKTOK_CLIENT_ID", "tiktok"),
    ("PINTEREST_CLIENT_ID", "pinterest"),
    ("DISCORD_CLIENT_ID", "discord"),
    ("SLACK_ID", "slack"),
    ("MASTODON_CLIENT_ID", "mastodon"),
    ("DRIBBBLE_CLIENT_ID", "dribbble"),
];
