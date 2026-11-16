//! Apple's on-device transcript of a Notes audio recording, stored (not
//! gzipped) in the audio attachment's `ZMERGEABLEDATA1`. One
//! `ICTTTranscriptSegment` per word/phrase, stored out of order.

use super::crdt::{Doc, ObjectId};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppleTranscript {
    pub text: String,
    pub summary: Option<String>,
}

pub fn decode(blob: &[u8]) -> anyhow::Result<AppleTranscript> {
    let doc = Doc::from_object_data(blob)?;
    let mut segments: Vec<(f64, String)> = Vec::new();
    let mut summary = None;
    for i in 0..doc.len() {
        let Some(cm) = doc.custom_map(i)? else {
            continue;
        };
        let get = |k: &str| -> anyhow::Result<Option<ObjectId>> {
            match cm.map.get(k).and_then(ObjectId::obj) {
                Some(o) => doc.scalar(o),
                None => Ok(None),
            }
        };
        match cm.type_name.as_str() {
            "com.apple.notes.ICTTAudioRecording" => {
                for k in ["topLineSummary", "summary"] {
                    if summary.is_none()
                        && let Some(ObjectId::Str(s)) = get(k)?
                        && !s.trim().is_empty()
                    {
                        summary = Some(s.trim().to_string());
                    }
                }
            }
            "com.apple.notes.ICTTTranscriptSegment" => {
                let ts = match get("timestamp")? {
                    Some(ObjectId::Double(d)) => d,
                    Some(ObjectId::Uint(u)) => u as f64,
                    _ => 0.0,
                };
                if let Some(ObjectId::Str(t)) = get("text")? {
                    segments.push((ts, t));
                }
            }
            _ => {}
        }
    }
    segments.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut text = String::new();
    for (_, s) in segments {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        // Punctuation attaches to the previous word.
        if !text.is_empty() && !s.starts_with([',', '.', '!', '?', ';', ':']) {
            text.push(' ');
        }
        text.push_str(s);
    }
    Ok(AppleTranscript { text, summary })
}
