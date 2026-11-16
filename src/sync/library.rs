//! Interests library: items from Notes + metadata from TMDB / Open Library.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::config::Config;
use crate::sync::notes::NoteDoc;
use crate::text::match_key;
use crate::time::now_iso;

/// Library kind for a folder under Interests, if it is a library folder.
pub fn kind_for_folder(rel_folder: &str) -> Option<&'static str> {
    let mut parts = rel_folder.split('/');
    if match_key(parts.next()?) != "interests" {
        return None;
    }
    match match_key(parts.next()?).as_str() {
        "books" | "reading" => Some("book"),
        "movies" | "films" | "film" | "movie" => Some("film"),
        "tvshows" | "tv" | "shows" | "series" => Some("tv"),
        _ => None,
    }
}

/// Status implied by a list title ("Books I read in 2023", "Movies to watch").
pub fn status_from_list_title(title: &str) -> Option<&'static str> {
    let t = format!(" {} ", title.to_lowercase());
    let want = [
        "to read", "to check", "to watch", "want", "wishlist", "backlog", "queue",
    ];
    let finished = [" read ", " watched ", " seen ", " finished ", " done "];
    if want.iter().any(|w| t.contains(w)) {
        Some("want")
    } else if finished.iter().any(|w| t.contains(w)) {
        Some("finished")
    } else {
        None
    }
}

/// "Score: 8/10", "Rating - 4/5", "My score: 7.5" -> score out of 10.
pub fn parse_score(md: &str) -> Option<f64> {
    let re = Regex::new(
        r"(?im)^\W*(?:my\s+)?(?:score|rating)\s*[:=\-–]\s*(\d+(?:[.,]\d+)?)\s*(?:/\s*(\d+))?",
    )
    .ok()?;
    let c = re.captures(md)?;
    let v: f64 = c.get(1)?.as_str().replace(',', ".").parse().ok()?;
    let scale: f64 = c
        .get(2)
        .and_then(|s| s.as_str().parse().ok())
        .unwrap_or(10.0);
    if scale <= 0.0 {
        return None;
    }
    Some(((v / scale * 10.0) * 10.0).round() / 10.0).filter(|s| (0.0..=10.0).contains(s))
}

/// "Status: reading" in a note.
pub fn parse_status(md: &str) -> Option<&'static str> {
    let re = Regex::new(r"(?im)^\W*status\s*[:=\-–]\s*([a-z ]+)").ok()?;
    let s = re.captures(md)?.get(1)?.as_str().trim().to_lowercase();
    Some(match s.as_str() {
        "want" | "to read" | "to watch" | "wishlist" | "planned" => "want",
        "reading" | "watching" | "in progress" | "started" => "in_progress",
        "read" | "watched" | "finished" | "done" | "seen" => "finished",
        _ => return None,
    })
}

/// Items of a list note: one per line or link card.
pub fn list_items(md: &str) -> Vec<(String, Option<String>)> {
    let link = Regex::new(r"^\[(.+)\]\((\S+)\)$").expect("valid regex");
    let marker = Regex::new(r"^(?:[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+)").expect("valid regex");
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut in_code = false;
    for line in md.lines() {
        let l = line.trim().trim_end_matches('\\').trim();
        if l.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code
            || l.is_empty()
            || l.starts_with('#')
            || l.starts_with('|')
            || l.starts_with('!')
            || l.starts_with('>')
        {
            continue;
        }
        let l = marker.replace(l, "").trim().to_string();
        if l.starts_with("http") || l.starts_with("<http") {
            continue;
        }
        let (title, url) = match link.captures(&l) {
            Some(c) => (
                c[1].replace("\\[", "[").replace("\\]", "]"),
                Some(c[2].to_string()),
            ),
            None => (
                l.replace("**", "").replace('*', "").trim().to_string(),
                None,
            ),
        };
        if title.chars().count() < 2 || !seen.insert(match_key(&title)) {
            continue;
        }
        out.push((title, url));
    }
    out
}

/// "Title: Subtitle" -> ("Title", Some("Title: Subtitle")); plain titles -> (title, None).
pub fn split_title(full: &str) -> (String, Option<String>) {
    match full.split_once(": ") {
        Some((head, _)) if head.chars().count() >= 2 => {
            (head.trim().to_string(), Some(full.trim().to_string()))
        }
        _ => (full.trim().to_string(), None),
    }
}

pub async fn sync_from_notes(
    conn: &mut SqliteConnection,
    docs: &[NoteDoc],
    ids: &HashMap<String, i64>,
) -> anyhow::Result<usize> {
    let mut seen: HashSet<String> = HashSet::new();
    for d in docs {
        let Some(kind) = kind_for_folder(&d.rel_folder) else {
            continue;
        };
        let folder_name = d.rel_folder.split('/').nth(1).unwrap_or("");
        let note_id = ids.get(&d.apple_id).copied();
        let is_list = match_key(&d.title).starts_with(&match_key(folder_name))
            && !match_key(folder_name).is_empty();
        // (source key, display title, lookup title, url, status, score)
        type Item<'a> = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<&'a str>,
            Option<f64>,
        );
        let items: Vec<Item<'_>> = if is_list {
            let status = status_from_list_title(&d.title);
            list_items(&d.markdown)
                .into_iter()
                .map(|(full, url)| {
                    let (display, lookup) = split_title(&full);
                    (
                        format!("list:{}:{}", d.apple_id, match_key(&display)),
                        display,
                        lookup,
                        url,
                        status,
                        None,
                    )
                })
                .collect()
        } else {
            vec![(
                format!("note:{}", d.apple_id),
                d.title.clone(),
                None,
                None,
                parse_status(&d.markdown),
                parse_score(&d.markdown),
            )]
        };
        for (key, title, lookup, url, status, score) in items {
            seen.insert(key.clone());
            sqlx::query(
                "INSERT INTO library_items (source_key, note_id, kind, title, lookup_title, status, note_score, link_url)
                 VALUES (?,?,?,?,?,?,?,?)
                 ON CONFLICT(source_key) DO UPDATE SET note_id=excluded.note_id, kind=excluded.kind,
                    meta_status = CASE WHEN library_items.title <> excluded.title OR library_items.kind <> excluded.kind
                                         OR COALESCE(library_items.lookup_title,'') <> COALESCE(excluded.lookup_title,'')
                                       THEN 'pending' ELSE library_items.meta_status END,
                    title=excluded.title, lookup_title=excluded.lookup_title,
                    status=COALESCE(excluded.status, library_items.status),
                    note_score=excluded.note_score, link_url=excluded.link_url",
            )
            .bind(&key)
            .bind(if key.starts_with("note:") { note_id } else { None })
            .bind(kind)
            .bind(&title)
            .bind(&lookup)
            .bind(status)
            .bind(score)
            .bind(&url)
            .execute(&mut *conn)
            .await?;
        }
    }
    // Items whose note/line disappeared: delete, unless the user added data
    // in the app, in which case keep them as app items.
    let rows = sqlx::query("SELECT id, source_key, user_score, summary FROM library_items WHERE source_key IS NOT NULL")
        .fetch_all(&mut *conn)
        .await?;
    for r in rows {
        let key: String = r.get("source_key");
        if seen.contains(&key) {
            continue;
        }
        let id: i64 = r.get("id");
        let has_user_data = r.get::<Option<f64>, _>("user_score").is_some()
            || r.get::<Option<String>, _>("summary").is_some();
        if has_user_data {
            sqlx::query("UPDATE library_items SET source_key = NULL, note_id = NULL WHERE id = ?")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        } else {
            sqlx::query("DELETE FROM library_items WHERE id = ?")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(seen.len())
}

// ------------------------------------------------------------------ metadata

#[derive(Debug, Default)]
struct Meta {
    external_id: String,
    year: Option<String>,
    creator: Option<String>,
    genres: Option<String>,
    runtime: Option<String>,
    overview: Option<String>,
    poster_url: Option<String>,
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("AndreiOS/0.1 (personal library)")
        .build()?)
}

fn s(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

/// Among results whose title equals `title`, the most popular one (several
/// works share a title: "The Psychology of Money" 1991 vs 2020); otherwise
/// the search engine's first result.
fn best<'a>(results: &'a [Value], title: &str, field: &str, popularity: &str) -> Option<&'a Value> {
    let key = match_key(title);
    let pop = |r: &Value| r[popularity].as_f64().unwrap_or(0.0);
    results
        .iter()
        .take(10)
        .filter(|r| match_key(r[field].as_str().unwrap_or("")) == key)
        .max_by(|a, b| pop(a).total_cmp(&pop(b)))
        .or(results.first())
}

async fn tmdb(
    client: &reqwest::Client,
    key: &str,
    kind: &str,
    title: &str,
    query: &str,
) -> anyhow::Result<Option<Meta>> {
    let path = if kind == "tv" { "tv" } else { "movie" };
    let auth = |rb: reqwest::RequestBuilder| {
        // v4 read-access tokens are JWTs; v3 keys are 32 hex chars
        if key.starts_with("eyJ") {
            rb.bearer_auth(key)
        } else {
            rb.query(&[("api_key", key)])
        }
    };
    let search: Value = auth(client.get(format!("https://api.themoviedb.org/3/search/{path}")))
        .query(&[("query", query), ("include_adult", "false")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let results = search["results"].as_array().cloned().unwrap_or_default();
    let name_field = if kind == "tv" { "name" } else { "title" };
    let Some(hit) = best(&results, title, name_field, "vote_count") else {
        return Ok(None);
    };
    let id = hit["id"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("tmdb result without id"))?;
    let d: Value = auth(client.get(format!("https://api.themoviedb.org/3/{path}/{id}")))
        .query(&[("append_to_response", "credits")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let genres = d["genres"].as_array().map(|g| {
        g.iter()
            .filter_map(|x| s(&x["name"]))
            .collect::<Vec<_>>()
            .join(", ")
    });
    let (year, runtime, creator) = if kind == "tv" {
        let years = match (
            s(&d["first_air_date"]),
            s(&d["last_air_date"]),
            d["in_production"].as_bool(),
        ) {
            (Some(a), Some(b), Some(false)) if a[..4] != b[..4] => {
                Some(format!("{}–{}", &a[..4], &b[..4]))
            }
            (Some(a), _, Some(true)) => Some(format!("{}–", &a[..4.min(a.len())])),
            (Some(a), _, _) => Some(a[..4.min(a.len())].to_string()),
            _ => None,
        };
        let rt = d["episode_run_time"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_i64)
            .map(|m| format!("{m} min / episode"));
        let seasons = d["number_of_seasons"]
            .as_i64()
            .map(|n| format!("{n} season{}", if n == 1 { "" } else { "s" }));
        let rt = match (rt, seasons) {
            (Some(a), Some(b)) => Some(format!("{b} · {a}")),
            (a, b) => a.or(b),
        };
        let by = d["created_by"].as_array().map(|c| {
            c.iter()
                .filter_map(|x| s(&x["name"]))
                .collect::<Vec<_>>()
                .join(", ")
        });
        (years, rt, by)
    } else {
        let year = s(&d["release_date"]).map(|r| r[..4.min(r.len())].to_string());
        let rt = d["runtime"]
            .as_i64()
            .filter(|m| *m > 0)
            .map(|m| format!("{}h {:02}m", m / 60, m % 60));
        let director = d["credits"]["crew"].as_array().map(|c| {
            c.iter()
                .filter(|x| x["job"] == "Director")
                .filter_map(|x| s(&x["name"]))
                .collect::<Vec<_>>()
                .join(", ")
        });
        (year, rt, director)
    };
    Ok(Some(Meta {
        external_id: format!("tmdb:{path}:{id}"),
        year,
        creator: creator.filter(|c| !c.is_empty()),
        genres: genres.filter(|g| !g.is_empty()),
        runtime,
        overview: s(&d["overview"]),
        poster_url: s(&d["poster_path"]).map(|p| format!("https://image.tmdb.org/t/p/w500{p}")),
    }))
}

async fn ol_search(client: &reqwest::Client, q: &str) -> anyhow::Result<Vec<Value>> {
    let search: Value = client
        .get("https://openlibrary.org/search.json")
        .query(&[
            ("q", q),
            ("limit", "10"),
            (
                "fields",
                "key,title,author_name,first_publish_year,cover_i,subject,readinglog_count",
            ),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(search["docs"].as_array().cloned().unwrap_or_default())
}

async fn open_library(
    client: &reqwest::Client,
    title: &str,
    query: &str,
) -> anyhow::Result<Option<Meta>> {
    let mut docs = ol_search(client, query).await?;
    if docs.is_empty() && query != title {
        docs = ol_search(client, title).await?;
    }
    let Some(hit) = best(&docs, title, "title", "readinglog_count") else {
        return Ok(None);
    };
    let key = s(&hit["key"]).unwrap_or_default();
    let mut overview = None;
    if key.starts_with("/works/") {
        let w: Value = client
            .get(format!("https://openlibrary.org{key}.json"))
            .send()
            .await?
            .json()
            .await
            .unwrap_or_default();
        overview = s(&w["description"]).or_else(|| s(&w["description"]["value"]));
        // strip Open Library's trailing source notes
        overview = overview.map(|o| {
            o.split("\n----------")
                .next()
                .unwrap_or(&o)
                .split("([source]")
                .next()
                .unwrap_or(&o)
                .trim()
                .to_string()
        });
    }
    let subjects = hit["subject"].as_array().map(|a| {
        a.iter()
            .take(3)
            .filter_map(s)
            .collect::<Vec<_>>()
            .join(", ")
    });
    Ok(Some(Meta {
        external_id: format!("openlibrary:{key}"),
        year: hit["first_publish_year"].as_i64().map(|y| y.to_string()),
        creator: hit["author_name"].as_array().map(|a| {
            a.iter()
                .take(3)
                .filter_map(s)
                .collect::<Vec<_>>()
                .join(", ")
        }),
        genres: subjects.filter(|g| !g.is_empty()),
        runtime: None,
        overview,
        poster_url: hit["cover_i"]
            .as_i64()
            .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg")),
    }))
}

/// Fetch metadata for items that need it. Returns (updated, warnings).
pub async fn fetch_metadata(
    cfg: &Config,
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<(usize, Vec<String>)> {
    let rows = sqlx::query(
        "SELECT id, kind, title, COALESCE(lookup_override, lookup_title, title) AS query FROM library_items
          WHERE meta_status = 'pending'
             OR (meta_status = 'no_key' AND kind <> 'book' AND ? = 1)
             OR (meta_status = 'error' AND (meta_fetched_at IS NULL OR meta_fetched_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 day')))
          ORDER BY id LIMIT ?",
    )
    .bind(cfg.tmdb_api_key.is_some())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok((0, vec![]));
    }
    let client = client()?;
    let mut updated = 0;
    let mut warnings = Vec::new();
    for r in rows {
        let id: i64 = r.get("id");
        let kind: String = r.get("kind");
        let title: String = r.get("title");
        let query: String = r.get("query");
        // With a manual correction, match results against the correction.
        let match_title = split_title(&query).0;
        let res = if kind == "book" {
            open_library(&client, &match_title, &query).await
        } else if let Some(key) = &cfg.tmdb_api_key {
            tmdb(&client, key, &kind, &match_title, &query).await
        } else {
            sqlx::query("UPDATE library_items SET meta_status = 'no_key' WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
            continue;
        };
        match res {
            Ok(Some(m)) => {
                let poster = match &m.poster_url {
                    Some(u) => match download(&client, u).await {
                        Ok(bytes) => {
                            let name = format!("{id}.jpg");
                            std::fs::write(cfg.posters_dir().join(&name), bytes)?;
                            Some(format!("/posters/{name}"))
                        }
                        Err(e) => {
                            warnings.push(format!("poster for {title}: {e}"));
                            None
                        }
                    },
                    None => None,
                };
                sqlx::query(
                    "UPDATE library_items SET external_id=?, year=?, creator=?, genres=?, runtime=?, overview=?,
                            poster_path=COALESCE(?, poster_path), meta_status='ok', meta_fetched_at=? WHERE id=?",
                )
                .bind(&m.external_id)
                .bind(&m.year)
                .bind(&m.creator)
                .bind(&m.genres)
                .bind(&m.runtime)
                .bind(&m.overview)
                .bind(poster)
                .bind(now_iso())
                .bind(id)
                .execute(pool)
                .await?;
                updated += 1;
            }
            Ok(None) => {
                sqlx::query("UPDATE library_items SET meta_status='not_found', meta_fetched_at=? WHERE id=?")
                    .bind(now_iso())
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            Err(e) => {
                warnings.push(format!("metadata for {title}: {e}"));
                sqlx::query(
                    "UPDATE library_items SET meta_status='error', meta_fetched_at=? WHERE id=?",
                )
                .bind(now_iso())
                .bind(id)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok((updated, warnings))
}

async fn download(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    if bytes.len() < 1024 {
        anyhow::bail!("image too small ({} bytes)", bytes.len());
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders() {
        assert_eq!(kind_for_folder("Interests/Books"), Some("book"));
        assert_eq!(kind_for_folder("Interests/TV Shows"), Some("tv"));
        assert_eq!(kind_for_folder("Interests/Movies"), Some("film"));
        assert_eq!(kind_for_folder("Interests/CS"), None);
        assert_eq!(kind_for_folder("Learning/Books"), None);
    }

    #[test]
    fn list_status() {
        assert_eq!(
            status_from_list_title("Books I read in 2023"),
            Some("finished")
        );
        assert_eq!(status_from_list_title("Books to check"), Some("want"));
        assert_eq!(status_from_list_title("Books to read"), Some("want"));
        assert_eq!(status_from_list_title("Books"), None);
    }

    #[test]
    fn scores() {
        assert_eq!(parse_score("Great.\nScore: 8/10\n"), Some(8.0));
        assert_eq!(parse_score("Rating - 4/5"), Some(8.0));
        assert_eq!(parse_score("my score: 7,5"), Some(7.5));
        assert_eq!(parse_score("No score here"), None);
        assert_eq!(parse_score("Score: 15/10"), None);
    }

    #[test]
    fn split_titles() {
        assert_eq!(
            split_title("Late Bloomers: The Hidden Strengths"),
            (
                "Late Bloomers".into(),
                Some("Late Bloomers: The Hidden Strengths".into())
            )
        );
        assert_eq!(split_title("The Alchemist"), ("The Alchemist".into(), None));
    }

    #[test]
    fn items_from_list() {
        let md = "A man’s search for meaning\\\nThe Alchemist\n\n[System Design Interview](https://amzn.eu/x)\n\nhttps://www.amazon.nl/x\n\n- The Alchemist\n";
        let items = list_items(md);
        assert_eq!(
            items,
            vec![
                ("A man’s search for meaning".into(), None),
                ("The Alchemist".into(), None),
                (
                    "System Design Interview".into(),
                    Some("https://amzn.eu/x".into())
                ),
            ]
        );
    }
}
