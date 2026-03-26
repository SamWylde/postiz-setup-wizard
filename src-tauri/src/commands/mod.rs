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
pub mod upgrade;
pub mod web_link;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Create a `Command` that won't spawn a visible console window on Windows.
pub fn silent_cmd(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

/// Shared mapping of env-file keys to provider IDs.
/// Used by import, snapshot, and transfer to detect configured providers.
/// Note: Instagram uses the same Facebook App keys (FACEBOOK_APP_ID/SECRET),
/// so it is not listed separately.
/// Sanitize a log line before emitting it to the frontend.
/// Redacts only the values of KEY=VALUE pairs where the key contains a secret keyword.
/// Leaves normal prose (error messages mentioning "password", "token", etc.) untouched.
pub fn sanitize_log_line(line: &str) -> String {
    let secret_keys = [
        "password",
        "secret",
        "token",
        "apikey",
        "api_key",
        "auth_token",
        "private_key",
    ];

    // Process the line by finding all '=' characters and checking whether the
    // preceding key contains a secret keyword.  We build the result in a single
    // forward pass so there is no stale-index problem.
    let mut result = String::with_capacity(line.len());
    let mut last_end = 0;

    for (i, ch) in line.char_indices() {
        if ch == '=' && i > 0 {
            // Look backwards for the key start (word chars before the =)
            let key_start = line[..i]
                .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                .map(|p| p + 1)
                .unwrap_or(0);

            // Only consider keys that start at or after where we last wrote to
            if key_start < last_end {
                continue;
            }

            let key = &line[key_start..i];
            let key_lower = key.to_lowercase();

            let is_secret = secret_keys.iter().any(|kw| key_lower.contains(kw));

            if is_secret {
                let val_start = i + 1;
                let val_end = if line.as_bytes().get(val_start) == Some(&b'"') {
                    line[val_start + 1..]
                        .find('"')
                        .map(|p| val_start + 1 + p + 1)
                        .unwrap_or(line.len())
                } else {
                    line[val_start..]
                        .find(|c: char| c.is_whitespace())
                        .map(|p| val_start + p)
                        .unwrap_or(line.len())
                };

                // Write everything from last_end up to and including the '=', then redact the value
                result.push_str(&line[last_end..=i]);
                result.push_str("***");
                last_end = val_end;
            }
        }
    }

    // Append any remaining text
    result.push_str(&line[last_end..]);
    result
}

/// Parse `docker compose ps --format json` output, handling both NDJSON
/// (one JSON object per line — Docker Compose v2.21+) and JSON array format
/// (older versions that emit `[{...}, {...}]`).
pub fn parse_docker_ps_json(stdout: &str) -> Vec<serde_json::Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Older Docker Compose versions emit a JSON array
    if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            return arr;
        }
    }

    // Modern NDJSON: one JSON object per line
    let mut results = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            results.push(val);
        }
    }
    results
}

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
