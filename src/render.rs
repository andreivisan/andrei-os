//! Markdown -> HTML for the wiki, Wikipedia style:
//! - `[[Title]]`, `[[Title|label]]`, `[[Title#Section]]`, `[[#Section]]` links
//!   (links to missing articles render as "wanted" pages);
//! - automatic linking of the first mention of another article's title
//!   (the notes came from Obsidian without explicit links);
//! - heading anchors and a table of contents;
//! - raw HTML in notes is shown as text, never executed;
//! - audio attachments become players with their transcript.

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};
use regex::Regex;

use crate::text::{match_key, slugify};

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// Only allow safe URL schemes in links and media.
fn safe_href(u: &str) -> String {
    let t = u.trim();
    let lower = t.to_ascii_lowercase();
    let ok = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('/')
        || lower.starts_with('#')
        || !lower.contains(':');
    if ok { esc(t) } else { "#".to_string() }
}

#[derive(Debug, Clone)]
pub struct Article {
    pub slug: String,
    pub title: String,
}

/// Everything needed to resolve links between articles.
#[derive(Default)]
pub struct LinkIndex {
    /// match_key(title) -> article
    by_key: HashMap<String, Article>,
    auto: Option<Regex>,
    /// lowercase matched text -> match_key, for auto links
    auto_keys: HashMap<String, String>,
}

impl LinkIndex {
    /// `articles`: (title, slug). Titles shared by several articles are
    /// ambiguous and excluded from automatic linking.
    pub fn new(articles: &[(String, String)]) -> Self {
        let mut by_key = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (title, slug) in articles {
            *counts.entry(match_key(title)).or_default() += 1;
            by_key.insert(
                match_key(title),
                Article {
                    slug: slug.clone(),
                    title: title.clone(),
                },
            );
        }
        // Auto-link candidates: distinctive titles only.
        let mut cands: Vec<&String> = articles
            .iter()
            .map(|(t, _)| t)
            .filter(|t| counts.get(&match_key(t)) == Some(&1))
            .filter(|t| {
                let words = t.split_whitespace().count();
                let len = t.chars().count();
                (words >= 2 && len >= 8)
                    || (words == 1 && len >= 5 && t.chars().next().is_some_and(char::is_uppercase))
            })
            .collect();
        cands.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        cands.dedup();
        let mut auto_keys = HashMap::new();
        let alts: Vec<String> = cands
            .iter()
            .map(|t| {
                auto_keys.insert(t.to_lowercase(), match_key(t));
                let pat = regex::escape(t.trim());
                // multi-word titles match case-insensitively; single words
                // must keep their capitalisation ("Tokio", not "tokio" in prose)
                if t.split_whitespace().count() > 1 {
                    format!("(?i:{pat})")
                } else {
                    pat
                }
            })
            .collect();
        let auto = if alts.is_empty() {
            None
        } else {
            Regex::new(&format!(r"\b(?:{})\b", alts.join("|"))).ok()
        };
        Self {
            by_key,
            auto,
            auto_keys,
        }
    }

    pub fn resolve(&self, title: &str) -> Option<&Article> {
        self.by_key.get(&match_key(title))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TocEntry {
    pub level: u8,
    pub id: String,
    pub text: String,
    pub number: String,
}

#[derive(Debug, Default)]
pub struct Rendered {
    pub html: String,
    pub toc: Vec<TocEntry>,
    /// Outgoing links: (match key, display title), unique.
    pub links: Vec<(String, String)>,
    pub plain: String,
    pub lead: String,
}

pub struct MediaInfo {
    pub title: Option<String>,
    pub transcript: Option<String>,
}

pub struct RenderCtx<'a> {
    pub links: &'a LinkIndex,
    pub self_slug: Option<&'a str>,
    pub media: &'a dyn Fn(&str) -> Option<MediaInfo>,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

fn is_audio(src: &str) -> bool {
    let l = src.to_ascii_lowercase();
    [".m4a", ".mp3", ".wav", ".aac", ".ogg", ".caf"]
        .iter()
        .any(|e| l.ends_with(e))
}

pub fn render(md: &str, ctx: &RenderCtx<'_>) -> Rendered {
    let mut out: Vec<Event<'_>> = Vec::new();
    let mut toc = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut linked: HashSet<String> = HashSet::new(); // auto-link once per target
    let mut plain = String::new();
    let mut lead = String::new();
    let mut in_code = false;
    let mut link_depth = 0usize;
    let mut custom_link_stack: Vec<bool> = Vec::new();
    let mut heading: Option<(HeadingLevel, Vec<Event<'_>>, String)> = None;
    let mut para_depth = 0usize;
    let mut first_para_done = false;
    let mut cur_para = String::new();
    let mut in_metadata = false;
    let mut image: Option<(String, String)> = None; // (src, alt text)
    let (mut h2, mut h3) = (0u32, 0u32);

    let note_link = |key: String, title: String, links: &mut Vec<(String, String)>| {
        if !links.iter().any(|(k, _)| *k == key) {
            links.push((key, title));
        }
    };

    for ev in Parser::new_ext(md, options()) {
        // ---- buffer heading contents to compute its id
        if let Some((_, buf, text)) = heading.as_mut() {
            match &ev {
                Event::End(TagEnd::Heading(_)) => {}
                Event::Text(t) | Event::Code(t) => {
                    text.push_str(t);
                    buf.push(Event::Text(t.clone()));
                    continue;
                }
                Event::Html(t) | Event::InlineHtml(t) => {
                    text.push_str(t);
                    buf.push(Event::Text(t.clone()));
                    continue;
                }
                // drop links inside headings (Wikipedia doesn't link headings)
                Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link) => continue,
                other => {
                    buf.push(other.clone());
                    continue;
                }
            }
        }

        match ev {
            Event::Start(Tag::MetadataBlock(_)) => in_metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata = false,
            _ if in_metadata => {}

            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level, Vec::new(), String::new()))
            }
            Event::End(TagEnd::Heading(_)) => {
                let (level, buf, text) = heading.take().expect("heading started");
                let base = {
                    let s = slugify(&text);
                    if s.is_empty() {
                        "section".to_string()
                    } else {
                        s
                    }
                };
                let n = ids.entry(base.clone()).or_insert(0);
                *n += 1;
                let id = if *n == 1 { base } else { format!("{base}-{n}") };
                let lvl = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                if lvl <= 3 {
                    let number = if lvl == 2 {
                        h2 += 1;
                        h3 = 0;
                        format!("{h2}")
                    } else {
                        h3 += 1;
                        format!("{}.{h3}", h2.max(1))
                    };
                    toc.push(TocEntry {
                        level: lvl,
                        id: id.clone(),
                        text: text.trim().to_string(),
                        number,
                    });
                }
                plain.push_str(&text);
                plain.push('\n');
                out.push(Event::Html(format!("<h{lvl} id=\"{}\">", esc(&id)).into()));
                out.extend(buf);
                out.push(Event::Html(format!("</h{lvl}>\n").into()));
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) if !l.is_empty() => {
                        format!(" class=\"language-{}\"", esc(l))
                    }
                    _ => String::new(),
                };
                out.push(Event::Html(format!("<pre><code{lang}>").into()));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                plain.push('\n');
                out.push(Event::Html("</code></pre>\n".into()));
            }

            Event::Start(Tag::Paragraph) => {
                para_depth += 1;
                cur_para.clear();
                out.push(Event::Start(Tag::Paragraph));
            }
            Event::End(TagEnd::Paragraph) => {
                para_depth = para_depth.saturating_sub(1);
                if !first_para_done && !cur_para.trim().is_empty() {
                    lead = cur_para.trim().to_string();
                    first_para_done = true;
                }
                plain.push('\n');
                out.push(Event::End(TagEnd::Paragraph));
            }

            // ---- links
            Event::Start(Tag::Link {
                link_type: LinkType::WikiLink { .. },
                dest_url,
                ..
            }) => {
                link_depth += 1;
                custom_link_stack.push(true);
                let dest = dest_url.to_string();
                let (page, section) = match dest.split_once('#') {
                    Some((p, s)) => (p.trim().to_string(), Some(s.trim().to_string())),
                    None => (dest.trim().to_string(), None),
                };
                let anchor = section
                    .map(|s| format!("#{}", slugify(&s)))
                    .unwrap_or_default();
                let html = if page.is_empty() {
                    format!("<a href=\"{}\">", esc(&anchor))
                } else if let Some(a) = ctx.links.resolve(&page) {
                    note_link(match_key(&page), a.title.clone(), &mut links);
                    format!("<a href=\"/wiki/{}{}\">", esc(&a.slug), esc(&anchor))
                } else {
                    note_link(match_key(&page), page.clone(), &mut links);
                    format!(
                        "<a class=\"is-missing\" title=\"Not written yet\" href=\"/wiki/{}\">",
                        esc(&slugify(&page))
                    )
                };
                out.push(Event::Html(html.into()));
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                link_depth += 1;
                custom_link_stack.push(true);
                let href = safe_href(&dest_url);
                let external = dest_url.starts_with("http://") || dest_url.starts_with("https://");
                let t = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", esc(&title))
                };
                let ext = if external {
                    " target=\"_blank\" rel=\"noopener noreferrer\" class=\"ext\""
                } else {
                    ""
                };
                out.push(Event::Html(format!("<a href=\"{href}\"{t}{ext}>").into()));
            }
            Event::End(TagEnd::Link) => {
                link_depth = link_depth.saturating_sub(1);
                if custom_link_stack.pop().unwrap_or(false) {
                    out.push(Event::Html("</a>".into()));
                } else {
                    out.push(Event::End(TagEnd::Link));
                }
            }

            // ---- images & audio: collect alt text, emit on end
            Event::Start(Tag::Image { dest_url, .. }) => {
                image = Some((dest_url.to_string(), String::new()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((src, alt)) = image.take() {
                    out.push(Event::Html(media_html(&src, &alt, ctx).into()));
                }
            }
            Event::Text(t) if image.is_some() => {
                if let Some((_, alt)) = image.as_mut() {
                    alt.push_str(&t);
                }
            }

            // ---- text
            Event::Text(t) => {
                plain.push_str(&t);
                if para_depth > 0 {
                    cur_para.push_str(&t);
                }
                if in_code || link_depth > 0 {
                    out.push(Event::Text(t));
                } else {
                    auto_link(t, ctx, &mut linked, &mut links, &mut out);
                }
            }
            Event::Code(t) => {
                plain.push_str(&t);
                if para_depth > 0 {
                    cur_para.push_str(&t);
                }
                out.push(Event::Code(t));
            }
            // Raw HTML from notes is displayed, not interpreted.
            Event::Html(t) | Event::InlineHtml(t) => {
                plain.push_str(&t);
                out.push(Event::Text(t));
            }
            Event::SoftBreak | Event::HardBreak => {
                plain.push(' ');
                if para_depth > 0 {
                    cur_para.push(' ');
                }
                out.push(ev);
            }
            other => {
                // keep words of adjacent blocks/cells apart in the plain text
                if matches!(
                    other,
                    Event::End(
                        TagEnd::Item
                            | TagEnd::TableCell
                            | TagEnd::TableRow
                            | TagEnd::TableHead
                            | TagEnd::BlockQuote(_)
                            | TagEnd::List(_)
                    )
                ) {
                    plain.push('\n');
                }
                out.push(other)
            }
        }
    }

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, out.into_iter());
    // wrap tables so wide ones scroll on small screens
    let html = html
        .replace("<table>", "<div class=\"table-wrap\"><table>")
        .replace("</table>", "</table></div>");
    Rendered {
        html,
        toc,
        links,
        plain,
        lead,
    }
}

fn auto_link<'a>(
    t: CowStr<'a>,
    ctx: &RenderCtx<'_>,
    linked: &mut HashSet<String>,
    links: &mut Vec<(String, String)>,
    out: &mut Vec<Event<'a>>,
) {
    let Some(re) = ctx.links.auto.as_ref() else {
        out.push(Event::Text(t));
        return;
    };
    let s = t.as_ref();
    let mut last = 0;
    let mut pieces: Vec<Event<'a>> = Vec::new();
    for m in re.find_iter(s) {
        let Some(key) = ctx.links.auto_keys.get(&m.as_str().to_lowercase()) else {
            continue;
        };
        let Some(a) = ctx.links.by_key.get(key) else {
            continue;
        };
        if ctx.self_slug == Some(a.slug.as_str()) || linked.contains(key) {
            continue;
        }
        linked.insert(key.clone());
        if !links.iter().any(|(k, _)| k == key) {
            links.push((key.clone(), a.title.clone()));
        }
        pieces.push(Event::Text(s[last..m.start()].to_string().into()));
        pieces.push(Event::Html(
            format!("<a href=\"/wiki/{}\">", esc(&a.slug)).into(),
        ));
        pieces.push(Event::Text(m.as_str().to_string().into()));
        pieces.push(Event::Html("</a>".into()));
        last = m.end();
    }
    if pieces.is_empty() {
        out.push(Event::Text(t));
    } else {
        pieces.push(Event::Text(s[last..].to_string().into()));
        out.extend(pieces);
    }
}

fn media_html(src: &str, alt: &str, ctx: &RenderCtx<'_>) -> String {
    let href = safe_href(src);
    if is_audio(src) {
        let info = (ctx.media)(src);
        let title = info
            .as_ref()
            .and_then(|i| i.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                if alt.is_empty() {
                    "Recording".to_string()
                } else {
                    alt.to_string()
                }
            });
        let transcript = match info.and_then(|i| i.transcript) {
            Some(t) if !t.trim().is_empty() => format!(
                "<details class=\"transcript\"><summary>Transcript</summary><p>{}</p></details>",
                esc(t.trim())
            ),
            _ => "<p class=\"meta\">No transcript yet</p>".to_string(),
        };
        format!(
            "<figure class=\"audio\"><figcaption>{}</figcaption><audio controls preload=\"none\" src=\"{href}\"></audio>{transcript}</figure>",
            esc(&title)
        )
    } else {
        format!(
            "<a class=\"figure\" href=\"{href}\" target=\"_blank\"><img src=\"{href}\" alt=\"{}\" loading=\"lazy\"></a>",
            esc(alt)
        )
    }
}

/// Plain text of Markdown (for search and leads), without rendering links.
pub fn plain_text(md: &str) -> String {
    let idx = LinkIndex::default();
    let none = |_: &str| None;
    render(
        md,
        &RenderCtx {
            links: &idx,
            self_slug: None,
            media: &none,
        },
    )
    .plain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> LinkIndex {
        LinkIndex::new(&[
            ("Tokio".into(), "tokio".into()),
            ("Actors with Tokio".into(), "actors-with-tokio".into()),
            ("Shared Mutable State".into(), "shared-mutable-state".into()),
            ("Data".into(), "data".into()),
            (
                "Architecture (HudAgents)".into(),
                "architecture-hudagents".into(),
            ),
        ])
    }

    fn r(md: &str, slug: Option<&str>) -> Rendered {
        let i = idx();
        let none = |_: &str| None;
        render(
            md,
            &RenderCtx {
                links: &i,
                self_slug: slug,
                media: &none,
            },
        )
    }

    #[test]
    fn wikilinks_resolve_and_missing() {
        let out = r(
            "See [[Tokio]], [[tokio|the runtime]] and [[Nope]] and [[#Intro]].",
            None,
        );
        assert!(
            out.html.contains("<a href=\"/wiki/tokio\">Tokio</a>"),
            "{}",
            out.html
        );
        assert!(out.html.contains("<a href=\"/wiki/tokio\">the runtime</a>"));
        assert!(out.html.contains("class=\"is-missing\""));
        assert!(out.html.contains("<a href=\"#intro\">"));
        assert_eq!(
            out.links.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
            vec!["tokio", "nope"]
        );
    }

    #[test]
    fn auto_links_first_mention_only_and_not_self() {
        let out = r(
            "Tokio powers actors. Tokio again. shared mutable state is hard. tokio lower.",
            Some("other"),
        );
        assert_eq!(
            out.html.matches("href=\"/wiki/tokio\"").count(),
            1,
            "{}",
            out.html
        );
        assert!(
            out.html
                .contains("<a href=\"/wiki/shared-mutable-state\">shared mutable state</a>")
        );
        // too generic to auto-link
        let out = r("Data is everywhere", None);
        assert!(!out.html.contains("<a"));
        // never link an article to itself, nor inside code
        let out = r("Tokio and `Tokio`", Some("tokio"));
        assert!(!out.html.contains("<a"));
        // longest title wins
        let out = r("Read Actors with Tokio.", None);
        assert!(out.html.contains("/wiki/actors-with-tokio"));
        assert!(!out.html.contains("/wiki/tokio\""));
    }

    #[test]
    fn headings_toc_and_unique_ids() {
        let out = r(
            "Lead para.\n\n## Intro\n\ntext\n\n### Detail\n\n## Intro\n",
            None,
        );
        assert_eq!(out.lead, "Lead para.");
        let ids: Vec<_> = out
            .toc
            .iter()
            .map(|t| (t.id.as_str(), t.number.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![("intro", "1"), ("detail", "1.1"), ("intro-2", "2")]
        );
        assert!(out.html.contains("<h2 id=\"intro\">Intro</h2>"));
    }

    #[test]
    fn raw_html_is_escaped_and_js_links_blocked() {
        let out = r(
            "<script>alert(1)</script>\n\nVec<T> and [x](javascript:alert(1))",
            None,
        );
        assert!(!out.html.contains("<script>"));
        assert!(out.html.contains("&lt;script&gt;"));
        assert!(out.html.contains("Vec&lt;T&gt;"));
        assert!(out.html.contains("href=\"#\""));
    }

    #[test]
    fn audio_embeds_player_with_transcript() {
        let i = idx();
        let media = |src: &str| {
            (src == "/media/a.m4a").then(|| MediaInfo {
                title: Some("New Recording".into()),
                transcript: Some("hello".into()),
            })
        };
        let out = render(
            "![rec](/media/a.m4a)",
            &RenderCtx {
                links: &i,
                self_slug: None,
                media: &media,
            },
        );
        assert!(out.html.contains("<audio controls"));
        assert!(
            out.html
                .contains("<summary>Transcript</summary><p>hello</p>")
        );
    }

    #[test]
    fn plain_text_separates_blocks() {
        let out = r("- one\n- two\n\n| a | b |\n|---|---|\n| c | d |\n", None);
        let words: Vec<&str> = out.plain.split_whitespace().collect();
        assert_eq!(words, vec!["one", "two", "a", "b", "c", "d"]);
    }

    #[test]
    fn tables_render_wrapped() {
        let out = r("| a | b |\n|---|---|\n| 1 | 2 |\n", None);
        assert!(out.html.contains("<div class=\"table-wrap\"><table>"));
    }
}
