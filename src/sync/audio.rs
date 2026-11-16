//! Transcription of Notes audio recordings, in-process with whisper.cpp.
//!
//! Same stack as `~/dev/rustscribe`: `transcribe-rs` (wrapping whisper-rs)
//! with the Metal backend and energy-adaptive chunking. Apple already stores
//! a transcript for most recordings (its transcriber does not support
//! Romanian); the Notes sync imports those, this fills the gaps, and
//! `andrei-os transcribe --force` re-transcribes everything. The language is
//! auto-detected, so English and Romanian both work.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Once;
use std::time::Duration;

use sqlx::{Row, SqlitePool};
use tokio::process::Command;
use transcribe_rs::transcriber::{EnergyAdaptiveChunked, EnergyAdaptiveConfig, Transcriber};
use transcribe_rs::whisper_cpp::WhisperEngine;
use transcribe_rs::{SpeechModel, TranscribeOptions};

use crate::config::Config;
use crate::time::now_iso;

#[derive(Debug, Default)]
pub struct AudioReport {
    pub transcribed: usize,
    pub pending: usize,
    pub warnings: Vec<String>,
}

/// The model file, if it is present (a GGML Whisper model is > 50 MB).
pub fn whisper_ready(cfg: &Config) -> Result<PathBuf, String> {
    let ok = std::fs::metadata(&cfg.whisper_model)
        .map(|m| m.len() > 50_000_000)
        .unwrap_or(false);
    if ok {
        Ok(cfg.whisper_model.clone())
    } else {
        Err(format!(
            "Whisper model missing at {} (see README: Transcription)",
            cfg.whisper_model.display()
        ))
    }
}

static CONFIGURE_ACCELERATOR: Once = Once::new();

/// Load the model (blocking, a few seconds). The accelerator preference is
/// process-wide in transcribe-rs, so it is set once before the first load.
/// whisper.cpp/GGML print hundreds of lines per run; routing them through
/// whisper-rs's hooks (no log backend enabled) drops them. Failures still
/// surface as errors from transcribe-rs.
fn load_model(path: &Path) -> anyhow::Result<WhisperEngine> {
    CONFIGURE_ACCELERATOR.call_once(|| {
        whisper_rs::install_logging_hooks();
        transcribe_rs::set_whisper_accelerator(transcribe_rs::WhisperAccelerator::Gpu);
    });
    WhisperEngine::load(path).map_err(|e| anyhow::anyhow!("load Whisper model: {e}"))
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
    let model_path = match whisper_ready(cfg) {
        Ok(p) => p,
        Err(e) => {
            report.warnings.push(format!(
                "{} recording(s) without transcript: {e}",
                rows.len()
            ));
            return Ok(report);
        }
    };
    // Loaded once per batch and dropped afterwards: the model holds ~1.6 GB
    // that the server should not keep between syncs.
    let mut model = tokio::task::spawn_blocking(move || load_model(&model_path)).await??;

    let work = std::env::temp_dir().join("andrei-os-audio");
    std::fs::create_dir_all(&work)?;
    for r in rows {
        let id: i64 = r.get("id");
        let media: String = r.get("media_path");
        let file = cfg.media_dir().join(media.trim_start_matches("/media/"));
        let wav = work.join(format!("{id}.wav"));
        let result = match to_wav(cfg, &file, &wav).await {
            Ok(()) => {
                let wav_in = wav.clone();
                let (m, res) = tokio::task::spawn_blocking(move || {
                    let res = transcribe_wav(&mut model, &wav_in);
                    (model, res)
                })
                .await?;
                model = m;
                res
            }
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&wav);
        match result {
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

/// Whisper needs 16 kHz mono 16-bit PCM. Prefer ffmpeg (as rustscribe does);
/// fall back to macOS's own afconvert, which is always present (launchd
/// processes do not see Homebrew's PATH).
async fn to_wav(cfg: &Config, input: &Path, wav: &Path) -> anyhow::Result<()> {
    if !input.is_file() {
        anyhow::bail!("file missing");
    }
    let mut cmd = if cfg.ffmpeg_bin.exists() {
        let mut c = Command::new(&cfg.ffmpeg_bin);
        c.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"])
            .arg(input)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(wav);
        c
    } else {
        let mut c = Command::new("/usr/bin/afconvert");
        c.args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(input)
            .arg(wav);
        c
    };
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let out = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("audio conversion timed out"))??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("audio conversion failed: {}", err.trim());
    }
    Ok(())
}

/// Feed the WAV to the model in ~25 s chunks cut at quiet points (the
/// rustscribe settings), with the language left to auto-detection.
fn transcribe_wav(model: &mut dyn SpeechModel, wav: &Path) -> anyhow::Result<String> {
    let mut reader = hound::WavReader::open(wav)?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        anyhow::bail!("expected 16 kHz mono 16-bit PCM, got {spec:?}");
    }
    if reader.duration() == 0 {
        anyhow::bail!("recording contains no audio");
    }
    let config = EnergyAdaptiveConfig {
        target_chunk_secs: 25.0,
        search_window_secs: 2.0,
        padding_secs: 0.0,
        min_chunk_secs: 0.0,
        ..Default::default()
    };
    // language: None = whisper.cpp auto-detects (per chunk).
    let mut chunker = EnergyAdaptiveChunked::new(config, TranscribeOptions::default());
    let mut buffer: Vec<f32> = Vec::with_capacity(16_000);
    for sample in reader.samples::<i16>() {
        buffer.push(f32::from(sample?) / 32768.0);
        if buffer.len() == 16_000 {
            chunker.feed(model, &buffer)?;
            buffer.clear();
        }
    }
    chunker.feed(model, &buffer)?;
    let result = chunker.finish(model)?;
    let text = result.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        anyhow::bail!("empty transcript");
    }
    Ok(text)
}
