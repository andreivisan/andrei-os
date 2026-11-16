//! The only write to Apple data: marking a reminder complete / incomplete
//! through AppleScript (never by writing the Reminders SQLite, which would
//! break CloudKit sync state).
//!
//! The first call triggers macOS's Automation prompt ("... wants to control
//! Reminders"); a denial surfaces as AppleScript error -1743.

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

const SCRIPT: &str = r#"on run argv
    set rid to item 1 of argv
    set newState to (item 2 of argv) is "true"
    tell application "Reminders"
        set completed of reminder id rid to newState
    end tell
end run"#;

pub async fn set_reminder_completed(uuid: &str, completed: bool) -> anyhow::Result<()> {
    // Validate: the id goes to osascript as an argv item (never interpolated
    // into the script), but reject anything that is not a UUID anyway.
    if uuid.len() != 36 || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        anyhow::bail!("not a reminder UUID: {uuid:?}");
    }
    let id = format!("x-apple-reminder://{}", uuid.to_uppercase());
    let child = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(SCRIPT)
        .arg(&id)
        .arg(if completed { "true" } else { "false" })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let out = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("osascript timed out talking to Reminders"))??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-1743") {
            anyhow::bail!(
                "macOS denied automation of Reminders. Allow it in System Settings → Privacy & Security → Automation."
            );
        }
        anyhow::bail!("osascript failed: {}", err.trim());
    }
    Ok(())
}
