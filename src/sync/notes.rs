//! Apple Notes -> Markdown files + index.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::apple::notes::markdown::{AttachmentRef, Converted, convert};
use crate::apple::notes::proto::decode_note;
use crate::apple::notes::store::{NoteStore, RawAttachment};
use crate::apple::notes::{table, transcript};
use crate::config::Config;
use crate::model::{area_from_folder, area_label};
use crate::render::{LinkIndex, RenderCtx, render};
use crate::sync::library;
use crate::sync::tasks::{self, SourceTask};
use crate::text::{hash_hex, match_key, slugify};
use crate::time::{from_core_data, iso_utc, now_iso};

#[derive(Debug, Default)]
pub struct NotesReport {
    pub notes: usize,
    pub projects: usize,
    pub tasks: usize,
    pub library: usize,
    pub warnings: Vec<String>,
}

/// One converted note, ready to index.
pub struct NoteDoc {
    pub apple_id: String,
    pub title: String,
    pub slug: String,
    pub rel_folder: String,
    pub area: &'static str,
    pub project_id: Option<i64>,
    pub markdown: String,
    pub created: String,
    pub modified: String,
    pub conv: Converted,
    pub attachments: Vec<AttRow>,
}

pub struct AttRow {
    pub apple_id: String,
    pub kind: &'static str,
    pub uti: String,
    pub source_path: Option<String>,
    pub media_path: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub apple_transcript: Option<String>,
}

fn rel_folder(cfg: &Config, path: &str) -> Option<String> {
    if path == cfg.notes_root_folder {
        return Some(String::new());
    }
    path.strip_prefix(&format!("{}/", cfg.notes_root_folder))
        .map(str::to_string)
}

fn sniff_ext(path: &Path) -> &'static str {
    let mut buf = [0u8; 12];
    let n = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut buf))
        .unwrap_or(0);
    let b = &buf[..n];
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" {
        let brand = &b[8..12];
        if brand == b"heic" || brand == b"heix" || brand == b"mif1" {
            "heic"
        } else {
            "m4a"
        }
    } else if b.starts_with(b"%PDF") {
        "pdf"
    } else {
        "bin"
    }
}

fn kind_for(uti: &str) -> &'static str {
    let u = uti.to_ascii_lowercase();
    if u.contains("audio") {
        "audio"
    } else if u == "public.url" {
        "url"
    } else if u.contains("jpeg")
        || u.contains("png")
        || u.contains("heic")
        || u.contains("gif")
        || u.contains("image")
    {
        "image"
    } else {
        "file"
    }
}

/// Copy an attachment file into <data>/media (only when missing or changed).
fn copy_media(media_dir: &Path, att: &RawAttachment) -> Option<(String, PathBuf)> {
    let src = att.file.as_ref()?;
    let ext = match src.extension().and_then(|e| e.to_str()) {
        Some(e) if !e.is_empty() && e.len() <= 5 => e.to_ascii_lowercase(),
        _ => sniff_ext(src).to_string(),
    };
    let name = format!("{}.{ext}", att.identifier.to_lowercase());
    let dest = media_dir.join(&name);
    let src_len = std::fs::metadata(src).map(|m| m.len()).ok()?;
    let same = std::fs::metadata(&dest)
        .map(|m| m.len() == src_len)
        .unwrap_or(false);
    if !same && let Err(e) = std::fs::copy(src, &dest) {
        tracing::warn!("copy {}: {e}", src.display());
        return None;
    }
    Some((format!("/media/{name}"), dest))
}

/// Link-card titles from shops carry noise: "Title: Author, A: ISBN: Amazon.com: Books"
/// or "Author, A: Title: Subtitle". Keep "Title: Subtitle".
pub fn clean_link_title(t: &str) -> String {
    let t = t.trim();
    let t = [" | ", " - Amazon", " – Amazon"]
        .iter()
        .filter_map(|sep| t.find(sep))
        .min()
        .map_or(t, |i| &t[..i]);
    let author = regex::Regex::new(r"^\p{Lu}[\p{L}'’-]+, \p{Lu}[\p{L}.]*(?: \p{Lu}[\p{L}.]*)?$")
        .expect("valid regex");
    let isbn = regex::Regex::new(r"^[0-9Xx-]{10,17}$").expect("valid regex");
    let noise = [
        "books",
        "bücher",
        "livres",
        "kindle edition",
        "paperback",
        "hardcover",
    ];
    let parts: Vec<&str> = t
        .split(": ")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| !author.is_match(p) && !isbn.is_match(p))
        .filter(|p| {
            !p.to_lowercase().starts_with("amazon.") && !noise.contains(&p.to_lowercase().as_str())
        })
        .collect();
    if parts.is_empty() {
        t.to_string()
    } else {
        parts.join(": ")
    }
}

fn md_link_text(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

pub async fn sync(cfg: &Config, pool: &SqlitePool) -> anyhow::Result<NotesReport> {
    let mut report = NotesReport::default();
    let store = NoteStore::open(&cfg.notes_db, &cfg.notes_container).await?;
    let folders = store.folder_paths().await?;
    let raw_notes = store.notes().await?;

    // ---- decode bodies of notes inside the AndreiOS tree
    let mut decoded = Vec::new();
    for rn in raw_notes {
        let Some(rel) = rel_folder(cfg, &rn.folder_path) else {
            continue;
        };
        match decode_note(&rn.body) {
            Ok(n) => decoded.push((rn, rel, n)),
            Err(e) => report
                .warnings
                .push(format!("note {} ({}): {e}", rn.title_hint, rn.identifier)),
        }
    }
    let att_ids: Vec<String> = decoded
        .iter()
        .flat_map(|(_, _, n)| n.attribute_run.iter())
        .filter_map(|r| r.attachment_info.as_ref()?.attachment_identifier.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let atts = store.attachments(&att_ids).await?;
    store.close().await;

    let mut conn = pool.acquire().await?;
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;

    // ---- projects: every folder under Projects/ (empty ones too)
    let mut project_folders: Vec<String> = folders
        .values()
        .filter_map(|p| rel_folder(cfg, p))
        .filter(|r| r.starts_with("Projects/"))
        .collect();
    project_folders.sort();
    let projects = upsert_projects(&mut tx, &project_folders).await?;
    report.projects = projects.len();

    // ---- convert
    let media_dir = cfg.media_dir();
    let mut docs: Vec<NoteDoc> = Vec::new();
    for (rn, rel, note) in &decoded {
        let mut rows: Vec<AttRow> = Vec::new();
        let render_att = |a: &AttachmentRef| -> Option<String> {
            let att = atts.get(&a.id);
            let uti = att
                .map(|x| x.uti.as_str())
                .filter(|u| !u.is_empty())
                .unwrap_or(&a.uti);
            if uti == "com.apple.notes.table" {
                let blob = att.and_then(|x| x.mergeable.as_ref())?;
                return Some(match table::decode(blob) {
                    Ok(t) => table::to_markdown(&t),
                    Err(e) => {
                        tracing::warn!("table {}: {e}", a.id);
                        "*[table could not be read]*".to_string()
                    }
                });
            }
            let att = att?;
            match kind_for(uti) {
                "url" => {
                    let url = att.url.as_deref()?;
                    let title = att
                        .title
                        .as_deref()
                        .map(clean_link_title)
                        .filter(|t| !t.is_empty());
                    Some(match title {
                        Some(t) => format!("[{}]({})", md_link_text(&t), url.replace(' ', "%20")),
                        None => format!("<{url}>"),
                    })
                }
                "audio" | "image" | "file" => {
                    let (href, _) = copy_media(&media_dir, att)?;
                    let title = att.title.clone().unwrap_or_default();
                    Some(
                        if kind_for(uti) == "file"
                            && !href.ends_with(".jpg")
                            && !href.ends_with(".png")
                        {
                            format!(
                                "[{}]({href})",
                                md_link_text(if title.is_empty() {
                                    "Attachment"
                                } else {
                                    &title
                                })
                            )
                        } else {
                            format!("![{}]({href})", md_link_text(&title))
                        },
                    )
                }
                _ => None,
            }
        };
        let conv = convert(note, &render_att);

        for a in &conv.attachments {
            let Some(att) = atts.get(&a.id) else { continue };
            let kind = kind_for(&att.uti);
            if att.uti == "com.apple.notes.table" {
                continue;
            }
            let media = if matches!(kind, "audio" | "image" | "file") {
                copy_media(&media_dir, att)
            } else {
                None
            };
            let apple_transcript = if kind == "audio" {
                att.mergeable
                    .as_deref()
                    .and_then(|b| transcript::decode(b).ok())
                    .map(|t| t.text)
                    .filter(|t| !t.is_empty())
            } else {
                None
            };
            rows.push(AttRow {
                apple_id: a.id.clone(),
                kind,
                uti: att.uti.clone(),
                source_path: att.file.as_ref().map(|p| p.display().to_string()),
                media_path: media.map(|m| m.0),
                url: att.url.clone(),
                title: att.title.clone(),
                apple_transcript,
            });
        }

        let title = if conv.title.is_empty() {
            rn.title_hint.trim().to_string()
        } else {
            conv.title.clone()
        };
        let title = if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        };
        let area = area_from_folder(rel);
        let project_id = deepest_project(&projects, rel);
        docs.push(NoteDoc {
            apple_id: rn.identifier.clone(),
            title,
            slug: String::new(),
            rel_folder: rel.clone(),
            area,
            project_id,
            markdown: conv.markdown.clone(),
            created: rn
                .created
                .and_then(from_core_data)
                .map(iso_utc)
                .unwrap_or_else(now_iso),
            modified: rn
                .modified
                .and_then(from_core_data)
                .map(iso_utc)
                .unwrap_or_else(now_iso),
            conv,
            attachments: rows,
        });
    }

    disambiguate(&mut docs);

    // ---- files
    write_files(&cfg.content_dir(), &docs)?;

    // ---- notes table
    sqlx::query("UPDATE notes SET slug = 'tmp-' || id")
        .execute(&mut *tx)
        .await?;
    let mut ids: HashMap<String, i64> = HashMap::new();
    for d in &docs {
        let rel_path = rel_path(d);
        let hash = hash_hex(&[&d.title, &d.markdown, &d.rel_folder]);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (apple_id, slug, title, title_key, area, folder_path, project_id, rel_path,
                                content_hash, created_at, modified_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(apple_id) DO UPDATE SET slug=excluded.slug, title=excluded.title,
                title_key=excluded.title_key, area=excluded.area, folder_path=excluded.folder_path,
                project_id=excluded.project_id, rel_path=excluded.rel_path, content_hash=excluded.content_hash,
                created_at=excluded.created_at, modified_at=excluded.modified_at
             RETURNING id",
        )
        .bind(&d.apple_id)
        .bind(&d.slug)
        .bind(&d.title)
        .bind(match_key(&d.title))
        .bind(d.area)
        .bind(&d.rel_folder)
        .bind(d.project_id)
        .bind(&rel_path)
        .bind(&hash)
        .bind(&d.created)
        .bind(&d.modified)
        .fetch_one(&mut *tx)
        .await?;
        ids.insert(d.apple_id.clone(), id);
    }
    // notes gone from Apple Notes
    let gone: Vec<i64> = sqlx::query_scalar("SELECT id FROM notes WHERE apple_id IS NOT NULL")
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .filter(|id| !ids.values().any(|v| v == id))
        .collect();
    for id in gone {
        sqlx::query("DELETE FROM notes WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // ---- attachments
    let mut seen_att = Vec::new();
    for d in &docs {
        let note_id = ids[&d.apple_id];
        for a in &d.attachments {
            seen_att.push(a.apple_id.clone());
            sqlx::query(
                "INSERT INTO attachments (apple_id, note_id, kind, uti, source_path, media_path, url, title,
                                          transcript, transcript_source, transcribed_at)
                 VALUES (?,?,?,?,?,?,?,?,?, CASE WHEN ? IS NULL THEN NULL ELSE 'apple' END,
                         CASE WHEN ? IS NULL THEN NULL ELSE ? END)
                 ON CONFLICT(apple_id) DO UPDATE SET note_id=excluded.note_id, kind=excluded.kind, uti=excluded.uti,
                    source_path=excluded.source_path, media_path=excluded.media_path, url=excluded.url,
                    title=excluded.title,
                    transcript = CASE WHEN attachments.transcript_source = 'whisper' THEN attachments.transcript
                                      ELSE COALESCE(excluded.transcript, attachments.transcript) END,
                    transcript_source = CASE WHEN attachments.transcript_source = 'whisper' THEN 'whisper'
                                      ELSE COALESCE(excluded.transcript_source, attachments.transcript_source) END,
                    transcribed_at = COALESCE(attachments.transcribed_at, excluded.transcribed_at)",
            )
            .bind(&a.apple_id)
            .bind(note_id)
            .bind(a.kind)
            .bind(&a.uti)
            .bind(&a.source_path)
            .bind(&a.media_path)
            .bind(&a.url)
            .bind(&a.title)
            .bind(&a.apple_transcript)
            .bind(&a.apple_transcript)
            .bind(&a.apple_transcript)
            .bind(now_iso())
            .execute(&mut *tx)
            .await?;
        }
    }
    let existing_att: Vec<String> = sqlx::query_scalar("SELECT apple_id FROM attachments")
        .fetch_all(&mut *tx)
        .await?;
    for a in existing_att.into_iter().filter(|a| !seen_att.contains(a)) {
        sqlx::query("DELETE FROM attachments WHERE apple_id = ?")
            .bind(a)
            .execute(&mut *tx)
            .await?;
    }

    // ---- index: links, categories, full-text search, leads
    reindex(&mut tx, &cfg.content_dir()).await?;

    // ---- project descriptions: lead of the note named like the project
    for (folder, pid) in &projects {
        let name = folder.rsplit('/').next().unwrap_or(folder);
        let lead: Option<String> = sqlx::query_scalar(
            "SELECT lead FROM notes WHERE project_id = ? AND title_key = ? AND lead <> '' LIMIT 1",
        )
        .bind(pid)
        .bind(match_key(name))
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query("UPDATE projects SET description = ? WHERE id = ?")
            .bind(lead)
            .bind(pid)
            .execute(&mut *tx)
            .await?;
    }

    // ---- checklist items -> tasks
    let mut seen_tasks = Vec::new();
    for d in &docs {
        let note_id = ids[&d.apple_id];
        for item in &d.conv.checklist {
            let sid = format!("{}#{}", d.apple_id, item.id);
            seen_tasks.push(sid.clone());
            let t = SourceTask {
                source: "notes",
                source_id: sid,
                title: item.text.clone(),
                notes: String::new(),
                done: item.done,
                completed_at: None,
                due_date: None,
                due_time: None,
                list: None,
                project_id: d.project_id,
                area: if d.project_id.is_some() {
                    None
                } else {
                    Some(d.area.to_string())
                },
                note_id: Some(note_id),
            };
            if tasks::upsert(&mut tx, &t, false).await? != tasks::Outcome::Skipped {
                report.tasks += 1;
            }
        }
    }
    tasks::retire_missing(&mut tx, "notes", &seen_tasks).await?;

    // ---- library
    report.library = library::sync_from_notes(&mut tx, &docs, &ids).await?;

    tx.commit().await?;
    report.notes = docs.len();
    Ok(report)
}

/// Deepest project folder containing `rel`.
fn deepest_project(projects: &[(String, i64)], rel: &str) -> Option<i64> {
    projects
        .iter()
        .filter(|(f, _)| rel == f || rel.starts_with(&format!("{f}/")))
        .max_by_key(|(f, _)| f.len())
        .map(|(_, id)| *id)
}

async fn upsert_projects(
    conn: &mut SqliteConnection,
    folders: &[String],
) -> anyhow::Result<Vec<(String, i64)>> {
    let mut out: Vec<(String, i64)> = Vec::new();
    let mut used_slugs = HashSet::new();
    let mut used_tags = HashSet::new();
    for f in folders {
        let name = f.rsplit('/').next().unwrap_or(f).to_string();
        let parent_folder = f.rsplit_once('/').map(|(p, _)| p.to_string());
        let parent_id =
            parent_folder.and_then(|p| out.iter().find(|(pf, _)| *pf == p).map(|(_, id)| *id));
        let mut slug = slugify(&name);
        if slug.is_empty() || !used_slugs.insert(slug.clone()) {
            slug = slugify(f.trim_start_matches("Projects/"));
            used_slugs.insert(slug.clone());
        }
        let mut tag = match_key(&name);
        if !used_tags.insert(tag.clone()) {
            tag = match_key(f.trim_start_matches("Projects/"));
            used_tags.insert(tag.clone());
        }
        // free the slug/tag if another (renamed) project row holds it
        sqlx::query("UPDATE projects SET slug = 'old-' || id || '-' || slug, tag_key = 'old-' || id || '-' || tag_key
                      WHERE folder_path <> ? AND (slug = ? OR tag_key = ?)")
            .bind(f)
            .bind(&slug)
            .bind(&tag)
            .execute(&mut *conn)
            .await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (slug, name, parent_id, folder_path, tag_key, archived) VALUES (?,?,?,?,?,0)
             ON CONFLICT(folder_path) DO UPDATE SET slug=excluded.slug, name=excluded.name,
                parent_id=excluded.parent_id, tag_key=excluded.tag_key, archived=0
             RETURNING id",
        )
        .bind(&slug)
        .bind(&name)
        .bind(parent_id)
        .bind(f)
        .bind(&tag)
        .fetch_one(&mut *conn)
        .await?;
        out.push((f.clone(), id));
    }
    // Folders that disappeared: archive (tasks keep their project).
    let rows = sqlx::query("SELECT id, folder_path FROM projects WHERE archived = 0")
        .fetch_all(&mut *conn)
        .await?;
    for r in rows {
        let fp: String = r.get("folder_path");
        if !folders.contains(&fp) {
            sqlx::query("UPDATE projects SET archived = 1 WHERE id = ?")
                .bind(r.get::<i64, _>("id"))
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(out)
}

/// Unique display titles ("Architecture (HudAgents)") and slugs.
fn disambiguate(docs: &mut [NoteDoc]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for d in docs.iter() {
        *counts.entry(match_key(&d.title)).or_default() += 1;
    }
    for d in docs.iter_mut() {
        if counts[&match_key(&d.title)] > 1 {
            let folder = d
                .rel_folder
                .rsplit('/')
                .next()
                .filter(|f| !f.is_empty())
                .unwrap_or("AndreiOS");
            d.title = format!("{} ({folder})", d.title);
        }
    }
    // stable order for slug suffixes
    let mut order: Vec<usize> = (0..docs.len()).collect();
    order.sort_by(|a, b| {
        docs[*a]
            .created
            .cmp(&docs[*b].created)
            .then(docs[*a].apple_id.cmp(&docs[*b].apple_id))
    });
    let mut used = HashSet::new();
    for i in order {
        let base = {
            let s = slugify(&docs[i].title);
            if s.is_empty() {
                format!(
                    "note-{}",
                    docs[i].apple_id[..8.min(docs[i].apple_id.len())].to_lowercase()
                )
            } else {
                s
            }
        };
        let mut slug = base.clone();
        let mut n = 2;
        while !used.insert(slug.clone()) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        docs[i].slug = slug;
    }
}

fn safe_component(s: &str) -> String {
    let c: String = s
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let c = c.trim().trim_start_matches('.').to_string();
    if c.is_empty() { "_".into() } else { c }
}

fn rel_path(d: &NoteDoc) -> String {
    let mut p: Vec<String> = d
        .rel_folder
        .split('/')
        .filter(|s| !s.is_empty())
        .map(safe_component)
        .collect();
    p.push(format!("{}.md", d.slug));
    p.join("/")
}

fn write_files(content: &Path, docs: &[NoteDoc]) -> anyhow::Result<()> {
    let mut keep = HashSet::new();
    for d in docs {
        let path = content.join(rel_path(d));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let body = format!(
            "---\ntitle: {}\napple_id: {}\nfolder: {}\ncreated: {}\nmodified: {}\n---\n\n{}",
            q(&d.title),
            q(&d.apple_id),
            q(&d.rel_folder),
            d.created,
            d.modified,
            d.markdown
        );
        let unchanged = std::fs::read_to_string(&path)
            .map(|s| s == body)
            .unwrap_or(false);
        if !unchanged {
            std::fs::write(&path, body)?;
        }
        keep.insert(path);
    }
    remove_stale(content, &keep)?;
    Ok(())
}

/// Delete .md files we did not write this run and prune empty folders.
fn remove_stale(dir: &Path, keep: &HashSet<PathBuf>) -> std::io::Result<bool> {
    let mut empty = true;
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            if remove_stale(&p, keep)? {
                std::fs::remove_dir(&p)?;
            } else {
                empty = false;
            }
        } else if p.extension().and_then(|x| x.to_str()) == Some("md") && !keep.contains(&p) {
            std::fs::remove_file(&p)?;
        } else {
            empty = false;
        }
    }
    Ok(empty)
}

/// Rebuild links, categories, leads and the search index from the Markdown
/// files referenced by the notes table.
pub async fn reindex(conn: &mut SqliteConnection, content_dir: &Path) -> anyhow::Result<()> {
    let rows = sqlx::query("SELECT id, slug, title, area, folder_path, rel_path FROM notes")
        .fetch_all(&mut *conn)
        .await?;
    let articles: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.get::<String, _>("title"), r.get::<String, _>("slug")))
        .collect();
    let index = LinkIndex::new(&articles);
    let none = |_: &str| None;

    sqlx::query("DELETE FROM note_links")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM note_categories")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM notes_fts")
        .execute(&mut *conn)
        .await?;

    for r in &rows {
        let id: i64 = r.get("id");
        let slug: String = r.get("slug");
        let title: String = r.get("title");
        let area: String = r.get("area");
        let folder: String = r.get("folder_path");
        let md = read_markdown(&content_dir.join(r.get::<String, _>("rel_path")));
        let out = render(
            &md,
            &RenderCtx {
                links: &index,
                self_slug: Some(&slug),
                media: &none,
            },
        );

        for (key, t) in &out.links {
            sqlx::query("INSERT OR IGNORE INTO note_links (from_note_id, target_key, target_title) VALUES (?,?,?)")
                .bind(id)
                .bind(key)
                .bind(t)
                .execute(&mut *conn)
                .await?;
        }
        let mut cats = vec![area_label(&area).to_string()];
        cats.extend(
            folder
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
        for c in cats {
            sqlx::query(
                "INSERT OR IGNORE INTO note_categories (note_id, category, cat_key) VALUES (?,?,?)",
            )
            .bind(id)
            .bind(&c)
            .bind(slugify(&c))
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query("INSERT INTO notes_fts (rowid, title, body) VALUES (?,?,?)")
            .bind(id)
            .bind(&title)
            .bind(&out.plain)
            .execute(&mut *conn)
            .await?;
        sqlx::query("UPDATE notes SET lead = ? WHERE id = ?")
            .bind(crate::text::truncate_words(&out.lead, 280))
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Read a note file and strip its front matter.
pub fn read_markdown(path: &Path) -> String {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    strip_front_matter(&s).to_string()
}

pub fn strip_front_matter(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("---\n")
        && let Some(end) = rest.find("\n---\n")
    {
        return rest[end + 5..].trim_start_matches('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_titles_are_cleaned() {
        assert_eq!(
            clean_link_title(
                "System Design Interview – An insider's guide: Xu, Alex: 9781736049112: Amazon.com: Books"
            ),
            "System Design Interview – An insider's guide"
        );
        assert_eq!(
            clean_link_title(
                "Kushner, D: Masters Of Doom: How two guys created an empire and transformed pop culture"
            ),
            "Masters Of Doom: How two guys created an empire and transformed pop culture"
        );
        assert_eq!(
            clean_link_title("Hell Yeah or No: what’s worth doing"),
            "Hell Yeah or No: what’s worth doing"
        );
        assert_eq!(clean_link_title("Developer Roadmaps"), "Developer Roadmaps");
    }

    #[test]
    fn front_matter() {
        assert_eq!(strip_front_matter("---\ntitle: x\n---\n\nBody\n"), "Body\n");
        assert_eq!(strip_front_matter("Body"), "Body");
    }

    #[test]
    fn deepest() {
        let p = vec![
            ("Projects/Hudward".to_string(), 1),
            ("Projects/Hudward/HudAgents".to_string(), 2),
        ];
        assert_eq!(deepest_project(&p, "Projects/Hudward/HudAgents"), Some(2));
        assert_eq!(deepest_project(&p, "Projects/Hudward"), Some(1));
        assert_eq!(deepest_project(&p, "Projects/HudwardX"), None);
        assert_eq!(deepest_project(&p, "Learning/Rust"), None);
    }
}
