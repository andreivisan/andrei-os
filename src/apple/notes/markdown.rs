//! Apple Notes body (text + attribute runs) -> Markdown.
//!
//! Rules (derived from the verified format, docs/apple-formats.md):
//! - run lengths are UTF-16 code units;
//! - `\n` ends a paragraph; U+2028 is a soft line break inside a paragraph;
//! - paragraph style comes from `paragraph_style.style_type` (absent = body);
//! - attachments are a single U+FFFC carrying `attachment_info`;
//! - body text is passed through verbatim: literal Markdown the user typed
//!   (backticks, `**bold**`, `1. ` lists) is intentional and should render.

use std::collections::HashMap;

use super::proto::{AttributeRun, Note};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    /// Hex of the checklist UUID (stable across edits), or a positional id.
    pub id: String,
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: String,
    pub uti: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub title: String,
    /// Body without the title line.
    pub markdown: String,
    pub checklist: Vec<ChecklistItem>,
    pub attachments: Vec<AttachmentRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Body,
    Title,
    Heading,
    Subheading,
    Code,
    Bullet,
    Dashed,
    Numbered,
    Checklist,
}

impl Kind {
    fn from_style(s: Option<i32>) -> Self {
        match s {
            Some(0) => Kind::Title,
            Some(1) => Kind::Heading,
            Some(2) => Kind::Subheading,
            Some(4) => Kind::Code,
            Some(100) => Kind::Bullet,
            Some(101) => Kind::Dashed,
            Some(102) => Kind::Numbered,
            Some(103) => Kind::Checklist,
            _ => Kind::Body,
        }
    }
    fn is_list(self) -> bool {
        matches!(
            self,
            Kind::Bullet | Kind::Dashed | Kind::Numbered | Kind::Checklist
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Inline {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<String>,
}

#[derive(Debug, Clone)]
struct Seg {
    text: String,
    inline: Inline,
    attachment: Option<AttachmentRef>,
}

#[derive(Debug, Clone)]
struct ParaStyle {
    kind: Kind,
    indent: usize,
    checklist: Option<(String, bool)>,
    list_start: Option<i32>,
    quote: bool,
}

#[derive(Debug, Clone)]
struct Para {
    style: ParaStyle,
    segs: Vec<Seg>,
}

impl Para {
    fn plain(&self) -> String {
        self.segs
            .iter()
            .filter(|s| s.attachment.is_none())
            .map(|s| s.text.as_str())
            .collect()
    }
    fn attachments(&self) -> impl Iterator<Item = &AttachmentRef> {
        self.segs.iter().filter_map(|s| s.attachment.as_ref())
    }
}

fn para_style(run: &AttributeRun) -> ParaStyle {
    let ps = run.paragraph_style.as_ref();
    let kind = Kind::from_style(ps.and_then(|p| p.style_type));
    ParaStyle {
        kind,
        indent: ps.and_then(|p| p.indent_amount).unwrap_or(0).clamp(0, 8) as usize,
        checklist: ps.and_then(|p| p.checklist.as_ref()).map(|c| {
            let id = c.uuid.as_deref().map(hex).unwrap_or_default();
            (id, c.done.unwrap_or(0) != 0)
        }),
        list_start: ps.and_then(|p| p.list_start),
        quote: ps.and_then(|p| p.block_quote).unwrap_or(0) != 0,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn inline_of(run: &AttributeRun) -> Inline {
    let w = run.font_weight.unwrap_or(0);
    Inline {
        bold: w == 1 || w == 3,
        italic: w == 2 || w == 3,
        strike: run.strikethrough.unwrap_or(0) != 0,
        link: run.link.clone().filter(|l| !l.trim().is_empty()),
    }
}

/// Split the note into styled paragraphs.
fn paragraphs(note: &Note) -> Vec<Para> {
    let text = note.note_text.as_deref().unwrap_or("");
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut pieces: Vec<(String, Option<&AttributeRun>)> = Vec::new();
    let mut pos = 0usize;
    for run in &note.attribute_run {
        let len = run.length.unwrap_or(0) as usize;
        let end = (pos + len).min(units.len());
        if end > pos {
            pieces.push((String::from_utf16_lossy(&units[pos..end]), Some(run)));
        }
        pos = end;
    }
    if pos < units.len() {
        pieces.push((String::from_utf16_lossy(&units[pos..]), None));
    }

    let default_style = ParaStyle {
        kind: Kind::Body,
        indent: 0,
        checklist: None,
        list_start: None,
        quote: false,
    };
    let mut out = Vec::new();
    let mut segs: Vec<Seg> = Vec::new();
    let mut style: Option<ParaStyle> = None;
    for (piece, run) in pieces {
        let run_style = run.map(para_style).unwrap_or_else(|| default_style.clone());
        let inline = run.map(inline_of).unwrap_or_default();
        let attachment = run.and_then(|r| r.attachment_info.as_ref()).and_then(|a| {
            Some(AttachmentRef {
                id: a.attachment_identifier.clone()?,
                uti: a.type_uti.clone().unwrap_or_default(),
            })
        });
        for (k, part) in piece.split('\n').enumerate() {
            if k > 0 {
                out.push(Para {
                    style: style.take().unwrap_or_else(|| run_style.clone()),
                    segs: std::mem::take(&mut segs),
                });
            }
            if part.is_empty() {
                continue;
            }
            if style.is_none() {
                style = Some(run_style.clone());
            }
            if let Some(att) = &attachment
                && part.contains('\u{FFFC}')
            {
                segs.push(Seg {
                    text: String::new(),
                    inline: Inline::default(),
                    attachment: Some(att.clone()),
                });
                let rest = part.replace('\u{FFFC}', "");
                if !rest.is_empty() {
                    segs.push(Seg {
                        text: rest,
                        inline: inline.clone(),
                        attachment: None,
                    });
                }
            } else {
                segs.push(Seg {
                    text: part.replace('\u{FFFC}', ""),
                    inline: inline.clone(),
                    attachment: None,
                });
            }
        }
    }
    if !segs.is_empty() {
        out.push(Para {
            style: style.unwrap_or(default_style),
            segs,
        });
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain, // headings: no emphasis markers
    Rich,  // body and list items
}

fn escape_link_text(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

fn encode_url(u: &str) -> String {
    u.trim()
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
        .replace('<', "%3C")
        .replace('>', "%3E")
}

/// Render a paragraph's text runs. `cont` is the prefix for continuation lines
/// after a soft break (keeps list items together).
fn render_inline(segs: &[Seg], mode: Mode, cont: &str) -> String {
    // Merge consecutive segments with identical inline style; Apple splits
    // runs for reasons invisible in the output (edit timestamps).
    let mut groups: Vec<(Inline, String)> = Vec::new();
    for s in segs.iter().filter(|s| s.attachment.is_none()) {
        let style = if mode == Mode::Plain {
            Inline {
                link: s.inline.link.clone(),
                ..Inline::default()
            }
        } else {
            s.inline.clone()
        };
        match groups.last_mut() {
            Some((g, t)) if *g == style => t.push_str(&s.text),
            _ => groups.push((style, s.text.clone())),
        }
    }
    let mut out = String::new();
    for (style, text) in groups {
        out.push_str(&wrap(&style, &text));
    }
    let brk = match mode {
        Mode::Plain => " ".to_string(),
        Mode::Rich => format!("\\\n{cont}"),
    };
    let out = out.replace('\u{2028}', &brk);
    out.trim_end_matches(['\\', '\n', ' ']).to_string()
}

fn wrap(style: &Inline, text: &str) -> String {
    let core = text.trim();
    if core.is_empty() || (*style == Inline::default()) {
        return text.to_string();
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let mut inner = core.to_string();
    if style.strike {
        inner = format!("~~{inner}~~");
    }
    match (style.bold, style.italic) {
        (true, true) => inner = format!("***{inner}***"),
        (true, false) => inner = format!("**{inner}**"),
        (false, true) => inner = format!("*{inner}*"),
        _ => {}
    }
    if let Some(url) = &style.link {
        inner = format!("[{}]({})", escape_link_text(&inner), encode_url(url));
    }
    format!("{lead}{inner}{trail}")
}

fn fence_for(code: &str) -> String {
    let mut max = 0;
    let mut cur = 0;
    for c in code.chars() {
        if c == '`' {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    "`".repeat((max + 1).max(3))
}

fn is_indented_code(p: &Para) -> bool {
    if p.style.kind != Kind::Body {
        return false;
    }
    let t = p.plain();
    (t.starts_with('\t') || t.starts_with("    ")) && !t.trim().is_empty()
}

enum Block {
    Para(String),
    /// list item: text, list kind, nesting level
    Item(String, Kind, usize),
}

/// Convert a decoded note. `render_attachment(id, uti)` returns the Markdown
/// block for an attachment (table, image, audio, link) or None to drop it.
pub fn convert(
    note: &Note,
    render_attachment: &dyn Fn(&AttachmentRef) -> Option<String>,
) -> Converted {
    let paras = paragraphs(note);
    let mut blocks: Vec<Block> = Vec::new();
    let mut checklist = Vec::new();
    let mut attachments = Vec::new();

    // ---- title: the first non-empty paragraph (Apple shows it as the title)
    let mut idx = 0;
    while idx < paras.len()
        && paras[idx].plain().trim().is_empty()
        && paras[idx].attachments().next().is_none()
    {
        idx += 1;
    }
    let mut title = String::new();
    if let Some(p) = paras.get(idx)
        && !p.plain().trim().is_empty()
    {
        let plain = p.plain();
        let mut lines = plain.split('\u{2028}');
        title = lines.next().unwrap_or("").trim().to_string();
        let rest: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
        if !rest.is_empty() {
            blocks.push(Block::Para(rest.join("\\\n")));
        }
        attachments.extend(p.attachments().cloned());
        for a in p.attachments() {
            if let Some(md) = render_attachment(a) {
                blocks.push(Block::Para(md.trim_end().to_string()));
            }
        }
        idx += 1;
    }
    let title_key = title.to_lowercase();

    // ---- body
    let mut code: Vec<String> = Vec::new();
    let mut counters: HashMap<usize, i64> = HashMap::new();
    let mut prev_list_indent: Option<usize> = None;
    let mut first_heading_checked = false;

    let flush_code = |code: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if code.is_empty() {
            return;
        }
        // trim leading/trailing blank lines
        while code.first().is_some_and(|l| l.trim().is_empty()) {
            code.remove(0);
        }
        while code.last().is_some_and(|l| l.trim().is_empty()) {
            code.pop();
        }
        if !code.is_empty() {
            let body = code.join("\n");
            let fence = fence_for(&body);
            blocks.push(Block::Para(format!("{fence}\n{body}\n{fence}")));
        }
        code.clear();
    };

    for p in &paras[idx.min(paras.len())..] {
        let kind = p.style.kind;
        let plain = p.plain();
        attachments.extend(p.attachments().cloned());

        // code blocks: Apple "Monospaced" paragraphs, or indented body lines
        if kind == Kind::Code || is_indented_code(p) {
            code.push(plain.replace('\u{2028}', "\n"));
            for a in p.attachments() {
                flush_code(&mut code, &mut blocks);
                if let Some(md) = render_attachment(a) {
                    blocks.push(Block::Para(md.trim_end().to_string()));
                }
            }
            continue;
        }
        flush_code(&mut code, &mut blocks);

        if !kind.is_list() {
            counters.clear();
            prev_list_indent = None;
        }

        let empty = plain.trim().is_empty();
        if !empty {
            match kind {
                Kind::Title | Kind::Heading | Kind::Subheading => {
                    let text = render_inline(&p.segs, Mode::Plain, "");
                    let text = text.trim();
                    // Notes often repeat the title as their first heading.
                    let dup = !first_heading_checked && text.to_lowercase() == title_key;
                    first_heading_checked = true;
                    if !dup {
                        let hashes = if kind == Kind::Subheading {
                            "###"
                        } else {
                            "##"
                        };
                        blocks.push(Block::Para(format!("{hashes} {text}")));
                    }
                }
                Kind::Bullet | Kind::Dashed | Kind::Numbered | Kind::Checklist => {
                    let level = p.style.indent;
                    if let Some(prev) = prev_list_indent
                        && level < prev
                    {
                        counters.retain(|l, _| *l <= level);
                    }
                    prev_list_indent = Some(level);
                    let pad = "    ".repeat(level);
                    let marker = match kind {
                        Kind::Numbered => {
                            let n = counters
                                .entry(level)
                                .and_modify(|n| *n += 1)
                                .or_insert_with(|| {
                                    i64::from(p.style.list_start.unwrap_or(1).max(0))
                                });
                            format!("{n}. ")
                        }
                        Kind::Checklist => {
                            let done = p.style.checklist.as_ref().is_some_and(|c| c.1);
                            if done {
                                "- [x] ".to_string()
                            } else {
                                "- [ ] ".to_string()
                            }
                        }
                        _ => "- ".to_string(),
                    };
                    let cont = format!("{pad}{}", " ".repeat(marker.len()));
                    let text = render_inline(&p.segs, Mode::Rich, &cont);
                    if kind == Kind::Checklist {
                        let (id, done) = p.style.checklist.clone().unwrap_or_default();
                        let id = if id.is_empty() {
                            format!("pos{}", checklist.len())
                        } else {
                            id
                        };
                        checklist.push(ChecklistItem {
                            id,
                            text: plain.replace('\u{2028}', " ").trim().to_string(),
                            done,
                        });
                    }
                    blocks.push(Block::Item(
                        format!("{pad}{marker}{}", text.trim_start()),
                        kind,
                        level,
                    ));
                }
                Kind::Body | Kind::Code => {
                    let text = render_inline(&p.segs, Mode::Rich, "");
                    if !text.trim().is_empty() {
                        blocks.push(Block::Para(text));
                    }
                }
            }
            if p.style.quote
                && let Some(Block::Para(t) | Block::Item(t, ..)) = blocks.last_mut()
            {
                *t = t
                    .lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
        for a in p.attachments() {
            if let Some(md) = render_attachment(a) {
                blocks.push(Block::Para(md.trim_end().to_string()));
            }
        }
    }
    flush_code(&mut code, &mut blocks);

    // ---- join: list items stay tight, everything else is separated by a blank line
    let mut markdown = String::new();
    // A top-level list of a different kind starts a new list (blank line).
    let mut prev: Option<(Kind, usize)> = None;
    for b in &blocks {
        let (text, item) = match b {
            Block::Para(t) => (t, None),
            Block::Item(t, k, l) => (t, Some((*k, *l))),
        };
        if !markdown.is_empty() {
            let tight = match (prev, item) {
                (Some((pk, pl)), Some((k, l))) => !(pl == 0 && l == 0 && pk != k),
                _ => false,
            };
            markdown.push_str(if tight { "\n" } else { "\n\n" });
        }
        markdown.push_str(text);
        prev = item;
    }
    if !markdown.is_empty() {
        markdown.push('\n');
    }
    Converted {
        title,
        markdown,
        checklist,
        attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::super::proto::{AttachmentInfo, AttributeRun, Checklist, Note, ParagraphStyle};
    use super::*;

    fn run(len: usize, style: Option<i32>) -> AttributeRun {
        AttributeRun {
            length: Some(len as u32),
            paragraph_style: Some(ParagraphStyle {
                style_type: style,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Build a note from (text, style, weight, link) runs.
    fn note(runs: &[(&str, Option<i32>)]) -> Note {
        let text: String = runs.iter().map(|(t, _)| *t).collect();
        Note {
            note_text: Some(text),
            attribute_run: runs
                .iter()
                .map(|(t, s)| run(t.encode_utf16().count(), *s))
                .collect(),
        }
    }

    fn none(_: &AttachmentRef) -> Option<String> {
        None
    }

    #[test]
    fn title_headings_lists_code() {
        let n = note(&[
            ("Architecture\n", Some(0)),
            ("Architecture\n", Some(1)), // duplicate of title: dropped
            ("Intro with `code` and **bold**.\n", None),
            ("1. Flow\n", Some(1)),
            ("one\n", Some(100)),
            ("two\n", Some(100)),
            ("first\n", Some(102)),
            ("second\n", Some(102)),
            ("fn main() {\n", Some(4)),
            ("    println!(\"```\");\n", Some(4)),
            ("}\n", Some(4)),
            ("Sub\n", Some(2)),
            ("end", None),
        ]);
        let c = convert(&n, &none);
        assert_eq!(c.title, "Architecture");
        assert_eq!(
            c.markdown,
            "Intro with `code` and **bold**.\n\n## 1. Flow\n\n- one\n- two\n\n1. first\n2. second\n\n\
             ````\nfn main() {\n    println!(\"```\");\n}\n````\n\n### Sub\n\nend\n"
        );
    }

    #[test]
    fn utf16_run_lengths_with_emoji() {
        // 🦀 is 2 UTF-16 units; the bold run must cover exactly "🦀 crab".
        let text = "T\nA 🦀 crab!";
        let mut bold = run("🦀 crab".encode_utf16().count(), None);
        bold.font_weight = Some(1);
        let n = Note {
            note_text: Some(text.into()),
            attribute_run: vec![run(2, Some(0)), run(2, None), bold, run(1, None)],
        };
        let c = convert(&n, &none);
        assert_eq!(c.markdown, "A **🦀 crab**!\n");
    }

    #[test]
    fn soft_breaks_and_links_and_checklists() {
        let text = "T\nline one\u{2028}line two\nsee docs\ntask a\ntask b\n";
        let mut link = run("docs".len(), None);
        link.link = Some("https://example.com/a b".into());
        let ck = |done| AttributeRun {
            length: Some(7),
            paragraph_style: Some(ParagraphStyle {
                style_type: Some(103),
                checklist: Some(Checklist {
                    uuid: Some(vec![0xab, 0xcd]),
                    done: Some(done),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let n = Note {
            note_text: Some(text.into()),
            attribute_run: vec![
                run(2, Some(0)),
                run(18, None),
                run(4, None),
                link,
                run(1, None),
                ck(0),
                ck(1),
            ],
        };
        let c = convert(&n, &none);
        assert_eq!(
            c.markdown,
            "line one\\\nline two\n\nsee [docs](https://example.com/a%20b)\n\n- [ ] task a\n- [x] task b\n"
        );
        assert_eq!(c.checklist.len(), 2);
        assert_eq!(
            c.checklist[0],
            ChecklistItem {
                id: "abcd".into(),
                text: "task a".into(),
                done: false
            }
        );
        assert!(c.checklist[1].done);
    }

    #[test]
    fn attachments_become_blocks() {
        let text = "T\nBefore\n\u{FFFC}\nAfter";
        let mut att = run(1, None);
        att.attachment_info = Some(AttachmentInfo {
            attachment_identifier: Some("TBL".into()),
            type_uti: Some("com.apple.notes.table".into()),
        });
        let n = Note {
            note_text: Some(text.into()),
            attribute_run: vec![run(2, Some(0)), run(7, None), att, run(6, None)],
        };
        let c = convert(&n, &|a| Some(format!("| {} |\n|---|\n", a.id)));
        assert_eq!(c.markdown, "Before\n\n| TBL |\n|---|\n\nAfter\n");
        assert_eq!(
            c.attachments,
            vec![AttachmentRef {
                id: "TBL".into(),
                uti: "com.apple.notes.table".into()
            }]
        );
    }

    #[test]
    fn heading_bold_is_not_doubled() {
        let mut h = run("Flow\n".len(), Some(1));
        h.font_weight = Some(1);
        let n = Note {
            note_text: Some("T\nFlow\n".into()),
            attribute_run: vec![run(2, Some(0)), h],
        };
        assert_eq!(convert(&n, &none).markdown, "## Flow\n");
    }

    #[test]
    fn long_title_with_soft_breaks_becomes_lead() {
        let n = note(&[
            (
                "Actors with Tokio\u{2028}\u{2028}This article is about actors\n",
                Some(0),
            ),
            ("Body", None),
        ]);
        let c = convert(&n, &none);
        assert_eq!(c.title, "Actors with Tokio");
        assert_eq!(c.markdown, "This article is about actors\n\nBody\n");
    }
}
