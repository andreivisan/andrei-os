//! Transcription of Notes audio recordings with whisper.cpp.
//!
//! Apple already stores an (English-only) transcript for most recordings;
//! the Notes sync imports those. This fills the gaps with whisper.cpp, and
//! `andrei-os transcribe --force` re-transcribes everything (Romanian too).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sqlx::{Row, SqlitePool};
use tokio::process::Command;

use crate::config::Config;
use crate::time::now_iso;

#[derive(Debug, Default)]
pub struct AudioReport {
    pub transcribed: usize,
    pub pending: usize,
    pub warnings: Vec<String>,
}

pub fn whisper_ready(cfg: &Config) -> Result<PathBuf, String> {
    let bin = cfg
        .whisper_bin
        .clone()
        .ok_or_else(|| "whisper.cpp not installed (brew install whisper.cpp)".to_string())?;
    let ok = std::fs::metadata(&cfg.whisper_model)
        .map(|m| m.len() > 50_000_000)
        .unwrap_or(false);
    if !ok {
        return Err(format!(
            "whisper model missing at {} (see README: Transcription)",
            cfg.whisper_model.display()
        ));
    }
    Ok(bin)
}

/// Transcribe up to `limit` recordings. `force` re-does Apple transcripts too.
pub async fn transcribe(
    cfg: &Config,
    pool: &SqlitePool,
    limit: i64,
    force: bool,
) -> anyhow::Result<AudioReport> {
    let mut report = AudioReport::default();
    let sql = if force {
        "SELECT id, media_path FROM attachments WHERE kind='audio' AND media_path IS NOT NULL
           AND COALESCE(transcript_source,'') <> 'whisper' ORDER BY id LIMIT ?"
    } else {
        "SELECT id, media_path FROM attachments WHERE kind='audio' AND media_path IS NOT NULL
           AND (transcript IS NULL OR transcript = '') ORDER BY id LIMIT ?"
    };
    let rows = sqlx::query(sql).bind(limit).fetch_all(pool).await?;
    report.pending = rows.len();
    if rows.is_empty() {
        return Ok(report);
    }
    let bin = match whisper_ready(cfg) {
        Ok(b) => b,
        Err(e) => {
            report.warnings.push(format!(
                "{} recording(s) without transcript: {e}",
                rows.len()
            ));
            return Ok(report);
        }
    };
    let work = std::env::temp_dir().join("andrei-os-audio");
    std::fs::create_dir_all(&work)?;
    for r in rows {
        let id: i64 = r.get("id");
        let media: String = r.get("media_path");
        let file = cfg.media_dir().join(media.trim_start_matches("/media/"));
        match run_whisper(cfg, &bin, &file, &work, id).await {
            Ok(text) => {
                sqlx::query(
                    "UPDATE attachments SET transcript=?, transcript_source='whisper', transcribed_at=? WHERE id=?",
                )
                .bind(text)
                .bind(now_iso())
                .bind(id)
                .execute(pool)
                .await?;
                report.transcribed += 1;
            }
            Err(e) => report
                .warnings
                .push(format!("transcribe {}: {e}", file.display())),
        }
    }
    Ok(report)
}

async fn run_cmd(cmd: &mut Command, secs: u64) -> anyhow::Result<std::process::Output> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let out = tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("timed out after {secs}s"))??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        anyhow::bail!("{tail}");
    }
    Ok(out)
}

async fn run_whisper(
    cfg: &Config,
    bin: &Path,
    input: &Path,
    work: &Path,
    id: i64,
) -> anyhow::Result<String> {
    if !input.is_file() {
        anyhow::bail!("file missing");
    }
    let wav = work.join(format!("{id}.wav"));
    // whisper.cpp needs 16 kHz mono PCM; prefer ffmpeg, fall back to macOS afconvert.
    let conv = if cfg.ffmpeg_bin.exists() {
        run_cmd(
            Command::new(&cfg.ffmpeg_bin)
                .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
                .arg(input)
                .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
                .arg(&wav),
            120,
        )
        .await
    } else {
        run_cmd(
            Command::new("/usr/bin/afconvert")
                .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
                .arg(input)
                .arg(&wav),
            120,
        )
        .await
    };
    conv?;
    let prefix = work.join(format!("{id}"));
    let res = run_cmd(
        Command::new(bin)
            .arg("-m")
            .arg(&cfg.whisper_model)
            .arg("-f")
            .arg(&wav)
            .args(["-l", "auto", "-otxt", "-np", "-nt"])
            .arg("-of")
            .arg(&prefix),
        900,
    )
    .await;
    let _ = std::fs::remove_file(&wav);
    res?;
    let txt_path = prefix.with_extension("txt");
    let text = std::fs::read_to_string(&txt_path)?;
    let _ = std::fs::remove_file(&txt_path);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        anyhow::bail!("empty transcript");
    }
    Ok(text)
}
