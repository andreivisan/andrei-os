//! Runtime configuration, from environment variables (a `.env` file is not
//! loaded automatically; use your shell or the launchd plist).

use std::path::PathBuf;

use chrono_tz::Tz;

#[derive(Debug, Clone)]
pub struct Config {
    /// Root for everything the app owns: DB, Markdown content, media, posters.
    pub data_dir: PathBuf,
    pub bind: String,
    pub tz: Tz,
    /// Name of the top-level Apple Notes folder that holds the second brain.
    pub notes_root_folder: String,
    pub notes_db: PathBuf,
    pub notes_container: PathBuf,
    pub reminders_stores: PathBuf,
    pub calendar_db: PathBuf,
    /// Calendars to import (ANDREIOS_CALENDARS="Business,Life"); empty = every
    /// titled calendar except the system ones in `apple::calendar::EXCLUDED`.
    pub calendars: Vec<String>,
    pub tmdb_api_key: Option<String>,
    /// Background sync interval for `serve`; 0 disables it.
    pub sync_interval_mins: u64,
    pub whisper_bin: Option<PathBuf>,
    pub whisper_model: PathBuf,
    pub ffmpeg_bin: PathBuf,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let home = home();
        let data_dir = env("ANDREIOS_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Library/Application Support/AndreiOS"));
        let tz: Tz = env("ANDREIOS_TZ")
            .unwrap_or_else(|| "Europe/Bucharest".into())
            .parse()
            .map_err(|e| anyhow::anyhow!("ANDREIOS_TZ: {e}"))?;
        let notes_container = home.join("Library/Group Containers/group.com.apple.notes");
        let whisper_bin = env("ANDREIOS_WHISPER_BIN").map(PathBuf::from).or_else(|| {
            [
                "/opt/homebrew/bin/whisper-cli",
                "/usr/local/bin/whisper-cli",
            ]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
        });
        Ok(Self {
            bind: env("ANDREIOS_BIND").unwrap_or_else(|| "127.0.0.1:4000".into()),
            tz,
            notes_root_folder: env("ANDREIOS_NOTES_ROOT").unwrap_or_else(|| "AndreiOS".into()),
            notes_db: notes_container.join("NoteStore.sqlite"),
            notes_container,
            reminders_stores: home
                .join("Library/Group Containers/group.com.apple.reminders/Container_v1/Stores"),
            calendar_db: home
                .join("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb"),
            calendars: env("ANDREIOS_CALENDARS")
                .map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            tmdb_api_key: env("TMDB_API_KEY"),
            sync_interval_mins: env("ANDREIOS_SYNC_INTERVAL_MINS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(15),
            whisper_bin,
            whisper_model: env("ANDREIOS_WHISPER_MODEL")
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("models/ggml-large-v3-turbo.bin")),
            ffmpeg_bin: env("ANDREIOS_FFMPEG_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/opt/homebrew/bin/ffmpeg")),
            data_dir,
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("andrei-os.db")
    }
    pub fn content_dir(&self) -> PathBuf {
        self.data_dir.join("content")
    }
    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }
    pub fn posters_dir(&self) -> PathBuf {
        self.data_dir.join("posters")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for d in [
            self.data_dir.clone(),
            self.content_dir(),
            self.media_dir(),
            self.posters_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}
