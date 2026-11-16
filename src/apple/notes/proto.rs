//! Protobuf messages of an Apple Notes body (`ZICNOTEDATA.ZDATA`, gzipped).
//! Field numbers verified against this Mac's NoteStore (see docs/apple-formats.md).
//! Unknown fields (timestamps, font info, paragraph UUIDs...) are skipped by prost.

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct NoteStoreProto {
    #[prost(message, optional, tag = "2")]
    pub document: Option<Document>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Document {
    #[prost(message, optional, tag = "3")]
    pub note: Option<Note>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Note {
    #[prost(string, optional, tag = "2")]
    pub note_text: Option<String>,
    #[prost(message, repeated, tag = "5")]
    pub attribute_run: Vec<AttributeRun>,
}

#[derive(Clone, PartialEq, Message)]
pub struct AttributeRun {
    /// Length in UTF-16 code units.
    #[prost(uint32, optional, tag = "1")]
    pub length: Option<u32>,
    #[prost(message, optional, tag = "2")]
    pub paragraph_style: Option<ParagraphStyle>,
    /// 1 bold, 2 italic, 3 bold+italic.
    #[prost(int32, optional, tag = "5")]
    pub font_weight: Option<i32>,
    #[prost(int32, optional, tag = "6")]
    pub underlined: Option<i32>,
    #[prost(int32, optional, tag = "7")]
    pub strikethrough: Option<i32>,
    #[prost(string, optional, tag = "9")]
    pub link: Option<String>,
    #[prost(message, optional, tag = "12")]
    pub attachment_info: Option<AttachmentInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ParagraphStyle {
    /// Absent = body. 0 title, 1 heading, 2 subheading, 4 monospaced,
    /// 100 bullet, 101 dashed, 102 numbered, 103 checklist.
    #[prost(int32, optional, tag = "1")]
    pub style_type: Option<i32>,
    #[prost(int32, optional, tag = "4")]
    pub indent_amount: Option<i32>,
    #[prost(message, optional, tag = "5")]
    pub checklist: Option<Checklist>,
    /// Start number of a numbered list (observed only on style 102).
    #[prost(int32, optional, tag = "7")]
    pub list_start: Option<i32>,
    #[prost(int32, optional, tag = "8")]
    pub block_quote: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Checklist {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub uuid: Option<Vec<u8>>,
    #[prost(int32, optional, tag = "2")]
    pub done: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct AttachmentInfo {
    #[prost(string, optional, tag = "1")]
    pub attachment_identifier: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub type_uti: Option<String>,
}

/// Gunzip (if needed) and decode a note body.
pub fn decode_note(data: &[u8]) -> anyhow::Result<Note> {
    let raw = gunzip_if_needed(data)?;
    let store = NoteStoreProto::decode(raw.as_slice())?;
    store
        .document
        .and_then(|d| d.note)
        .ok_or_else(|| anyhow::anyhow!("note protobuf has no document/note"))
}

pub fn gunzip_if_needed(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data).read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(data.to_vec())
    }
}
