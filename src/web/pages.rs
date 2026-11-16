use std::collections::{BTreeMap, HashMap};

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::{Timelike, Utc};
use serde::Deserialize;
use sqlx::Row;

use super::AppState;
use super::error::{PageError, not_found};
use super::queries::{
    self, EventView, NoteCard, ProjectView, TaskFilter, TaskView, project_family, setting, tasks,
};
use crate::model::{AREAS, STATUSES, area_label};
use crate::render::{LinkIndex, MediaInfo, RenderCtx, TocEntry, render};
use crate::sync::notes::read_markdown;
use crate::text::{match_key, slugify};
use crate::time::today;

type PageResult = Result<Response, PageError>;

fn html<T: Template>(t: T) -> PageResult {
    Ok(Html(t.render()?).into_response())
}

// ------------------------------------------------------------------ layout

pub struct Layout {
    pub active: &'static str,
    pub title: String,
    pub projects: Vec<ProjectView>,
    pub areas: Vec<(&'static str, &'static str)>,
    pub last_sync: String,
    pub sync_ok: bool,
}

async fn layout(
    st: &AppState,
    active: &'static str,
    title: impl Into<String>,
) -> Result<Layout, PageError> {
    let projects = queries::projects(&st.pool).await?;
    let last: Option<(String, bool)> =
        sqlx::query_as("SELECT finished_at, ok FROM sync_runs ORDER BY id DESC LIMIT 1")
            .fetch_optional(&st.pool)
            .await?;
    let (last_sync, sync_ok) = match last {
        Some((at, ok)) => {
            let label = chrono::DateTime::parse_from_rfc3339(&at)
                .map(|d| {
                    let local = d.with_timezone(&st.cfg.tz);
                    if local.date_naive() == today(st.cfg.tz) {
                        format!("Synced {}", local.format("%H:%M"))
                    } else {
                        format!("Synced {}", local.format("%-d %b %H:%M"))
                    }
                })
                .unwrap_or_else(|_| "Synced".into());
            (label, ok)
        }
        None => ("Not synced yet".into(), true),
    };
    Ok(Layout {
        active,
        title: title.into(),
        projects,
        areas: AREAS
            .iter()
            .filter(|(k, _)| *k != "projects")
            .copied()
            .collect(),
        last_sync,
        sync_ok,
    })
}

// ------------------------------------------------------------------ static

pub async fn styles() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/styles.css"),
    )
}
pub async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/app.js"),
    )
}
pub async fn favicon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../../static/favicon.svg"),
    )
}

pub async fn fallback_404() -> PageError {
    super::error::not_found("Page")
}

// ------------------------------------------------------------------ dashboard

pub struct Focus {
    pub label: String,
    pub href: String,
    pub sub: String,
}

pub struct FocusOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTpl {
    layout: Layout,
    greeting: &'static str,
    date_label: String,
    tasks: Vec<TaskView>,
    events: Vec<EventView>,
    upcoming: Vec<EventView>,
    focus: Option<Focus>,
    focus_options: Vec<FocusOption>,
    focus_auto: bool,
    inbox_count: i64,
    article_count: i64,
}

pub async fn dashboard(State(st): State<AppState>) -> PageResult {
    let tz = st.cfg.tz;
    let now = Utc::now().with_timezone(&tz);
    let day = now.date_naive();
    let greeting = match now.hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    let tasks = tasks(
        &st.pool,
        &TaskFilter {
            today: Some(day),
            ..Default::default()
        },
        day,
    )
    .await?;
    let events = queries::events_between(&st.pool, day, day, now.naive_local(), 50).await?;
    let upcoming = if events.iter().all(|e| e.all_day) {
        let next: Vec<EventView> = queries::events_between(
            &st.pool,
            day + chrono::Duration::days(1),
            day + chrono::Duration::days(14),
            now.naive_local(),
            3,
        )
        .await?;
        next
    } else {
        vec![]
    };
    let (focus, focus_options, focus_auto) = learning_focus(&st).await?;
    let inbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tasks WHERE deleted_at IS NULL AND status <> 'done' AND project_id IS NULL AND area IS NULL",
    )
    .fetch_one(&st.pool)
    .await?;
    let article_count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes")
        .fetch_one(&st.pool)
        .await?;
    html(DashboardTpl {
        layout: layout(&st, "dashboard", "Dashboard").await?,
        greeting,
        date_label: now.format("%a %-d %b").to_string().to_uppercase(),
        tasks,
        events,
        upcoming,
        focus,
        focus_options,
        focus_auto,
        inbox_count,
        article_count,
    })
}

/// The current learning focus: set in the app, or (automatically) the
/// Learning topic with the most recently edited note.
async fn learning_focus(
    st: &AppState,
) -> Result<(Option<Focus>, Vec<FocusOption>, bool), PageError> {
    let topics = sqlx::query(
        "SELECT c.category, c.cat_key, count(*) AS n, max(n.modified_at) AS last
           FROM note_categories c JOIN notes n ON n.id = c.note_id
          WHERE n.area = 'learning' AND c.cat_key <> 'learning'
          GROUP BY c.cat_key ORDER BY last DESC",
    )
    .fetch_all(&st.pool)
    .await?;
    let notes = sqlx::query(
        "SELECT slug, title FROM notes WHERE area = 'learning' ORDER BY title COLLATE NOCASE",
    )
    .fetch_all(&st.pool)
    .await?;
    let chosen = setting(&st.pool, "learning_focus").await?;
    let auto = chosen.is_none();
    let value = chosen.clone().or_else(|| {
        topics
            .first()
            .map(|t| format!("category:{}", t.get::<String, _>("cat_key")))
    });

    let mut options = vec![FocusOption {
        value: String::new(),
        label: "Automatic (most active topic)".into(),
        selected: auto,
    }];
    for t in &topics {
        let v = format!("category:{}", t.get::<String, _>("cat_key"));
        options.push(FocusOption {
            selected: !auto && chosen.as_deref() == Some(v.as_str()),
            label: format!("Topic · {}", t.get::<String, _>("category")),
            value: v,
        });
    }
    for n in &notes {
        let v = format!("note:{}", n.get::<String, _>("slug"));
        options.push(FocusOption {
            selected: !auto && chosen.as_deref() == Some(v.as_str()),
            label: format!("Article · {}", n.get::<String, _>("title")),
            value: v,
        });
    }

    let focus = match value.as_deref() {
        Some(v) if v.starts_with("category:") => {
            let key = &v["category:".len()..];
            topics
                .iter()
                .find(|t| t.get::<String, _>("cat_key") == key)
                .map(|t| Focus {
                    label: t.get("category"),
                    href: format!("/wiki/category/{key}"),
                    sub: format!("{} articles", t.get::<i64, _>("n")),
                })
        }
        Some(v) if v.starts_with("note:") => {
            let slug = &v["note:".len()..];
            notes
                .iter()
                .find(|n| n.get::<String, _>("slug") == slug)
                .map(|n| Focus {
                    label: n.get("title"),
                    href: format!("/wiki/{slug}"),
                    sub: "Article".into(),
                })
        }
        _ => None,
    };
    Ok((focus, options, auto))
}

// ------------------------------------------------------------------ projects

pub struct Column {
    pub key: &'static str,
    pub label: &'static str,
    pub tasks: Vec<TaskView>,
}

#[derive(Template)]
#[template(path = "project.html")]
pub struct ProjectTpl {
    layout: Layout,
    this_project: Option<i64>,
    project: ProjectView,
    parent: Option<ProjectView>,
    children: Vec<ProjectView>,
    own_notes: Vec<NoteCard>,
    related: Vec<NoteCard>,
    columns: Vec<Column>,
}

pub async fn projects_index(State(st): State<AppState>) -> PageResult {
    let all = queries::projects(&st.pool).await?;
    match all.first() {
        Some(p) => Ok(Redirect::to(&format!("/projects/{}", p.slug)).into_response()),
        None => {
            #[derive(Template)]
            #[template(path = "empty.html")]
            struct EmptyTpl {
                layout: Layout,
                message: &'static str,
            }
            html(EmptyTpl {
                layout: layout(&st, "projects", "Projects").await?,
                message: "No projects yet. Create a folder under AndreiOS/Projects in Apple Notes, then sync.",
            })
        }
    }
}

pub async fn project(State(st): State<AppState>, Path(slug): Path<String>) -> PageResult {
    let lay = layout(&st, "projects", "").await?;
    let project = lay
        .projects
        .iter()
        .find(|p| p.slug == slug)
        .cloned()
        .ok_or_else(|| not_found("Project"))?;
    let family = project_family(&lay.projects, project.id);
    let parent = project
        .parent_id
        .and_then(|pid| lay.projects.iter().find(|p| p.id == pid).cloned());
    let children: Vec<ProjectView> = lay
        .projects
        .iter()
        .filter(|p| p.parent_id == Some(project.id))
        .cloned()
        .collect();
    let day = today(st.cfg.tz);
    let all = tasks(
        &st.pool,
        &TaskFilter {
            project_ids: Some(family.clone()),
            include_done: true,
            by_position: true,
            ..Default::default()
        },
        day,
    )
    .await?;
    let columns = STATUSES
        .iter()
        .map(|(k, label)| Column {
            key: k,
            label,
            tasks: all.iter().filter(|t| t.status == *k).cloned().collect(),
        })
        .collect();
    let own_notes = queries::project_notes(&st.pool, &family).await?;
    let related = queries::related_notes(&st.pool, &family, &project.name).await?;
    html(ProjectTpl {
        layout: Layout {
            title: project.name.clone(),
            ..lay
        },
        this_project: Some(project.id),
        project,
        parent,
        children,
        own_notes,
        related,
        columns,
    })
}

// ------------------------------------------------------------------ tasks table

#[derive(Deserialize, Default)]
pub struct TasksQ {
    view: Option<String>,
    status: Option<String>,
    project: Option<String>,
    area: Option<String>,
    source: Option<String>,
    q: Option<String>,
}

#[derive(Template)]
#[template(path = "tasks.html")]
pub struct TasksTpl {
    layout: Layout,
    tasks: Vec<TaskView>,
    view: String,
    status: String,
    project: String,
    area: String,
    source: String,
    q: String,
    statuses: Vec<(&'static str, &'static str)>,
    preset_project: Option<i64>,
    preset_area: String,
}

pub async fn tasks_page(State(st): State<AppState>, Query(q): Query<TasksQ>) -> PageResult {
    let lay = layout(&st, "tasks", "Tasks").await?;
    let day = today(st.cfg.tz);
    let view = q.view.clone().unwrap_or_else(|| "all".into());
    let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
    let project = nonempty(&q.project);
    let project_ids = project
        .as_ref()
        .and_then(|slug| lay.projects.iter().find(|p| p.slug == *slug))
        .map(|p| vec![p.id]);
    let status = nonempty(&q.status);
    let f = TaskFilter {
        project_ids: if project.is_some() {
            Some(project_ids.clone().unwrap_or_default())
        } else {
            None
        },
        area: nonempty(&q.area),
        inbox: view == "inbox",
        status: status.clone().filter(|s| s != "open" && s != "any"),
        include_done: status.as_deref() == Some("any"),
        source: nonempty(&q.source),
        q: nonempty(&q.q),
        today: (view == "today").then_some(day),
        limit: Some(1000),
        by_position: false,
    };
    let list = tasks(&st.pool, &f, day).await?;
    html(TasksTpl {
        preset_project: project_ids.and_then(|v| v.first().copied()),
        preset_area: nonempty(&q.area).unwrap_or_default(),
        layout: lay,
        tasks: list,
        view,
        status: status.unwrap_or_else(|| "open".into()),
        project: project.unwrap_or_default(),
        area: q.area.unwrap_or_default(),
        source: q.source.unwrap_or_default(),
        q: q.q.unwrap_or_default(),
        statuses: STATUSES.to_vec(),
    })
}

// ------------------------------------------------------------------ areas

pub struct NoteGroup {
    pub name: String,
    pub key: String,
    pub notes: Vec<NoteCard>,
}

#[derive(Template)]
#[template(path = "area.html")]
pub struct AreaTpl {
    layout: Layout,
    key: String,
    label: &'static str,
    groups: Vec<NoteGroup>,
    tasks: Vec<TaskView>,
    library_count: i64,
}

pub async fn area(State(st): State<AppState>, Path(key): Path<String>) -> PageResult {
    if key == "projects" {
        return Ok(Redirect::to("/projects").into_response());
    }
    let label = AREAS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| *l)
        .ok_or_else(|| not_found("Area"))?;
    let active: &'static str = AREAS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(k, _)| *k)
        .unwrap_or("");
    let notes = queries::notes_where(&st.pool, "area = ?", &key).await?;
    let mut groups: BTreeMap<String, Vec<NoteCard>> = BTreeMap::new();
    for n in notes {
        let sub = n.folder.split('/').nth(1).unwrap_or("").to_string();
        groups.entry(sub).or_default().push(n);
    }
    let groups = groups
        .into_iter()
        .map(|(name, notes)| NoteGroup {
            key: slugify(if name.is_empty() { label } else { &name }),
            name: if name.is_empty() {
                "General".into()
            } else {
                name
            },
            notes,
        })
        .collect();
    let day = today(st.cfg.tz);
    let tasks = tasks(
        &st.pool,
        &TaskFilter {
            area: Some(key.clone()),
            ..Default::default()
        },
        day,
    )
    .await?;
    let library_count: i64 = if key == "interests" {
        sqlx::query_scalar("SELECT count(*) FROM library_items")
            .fetch_one(&st.pool)
            .await?
    } else {
        0
    };
    html(AreaTpl {
        layout: layout(&st, active, label).await?,
        key,
        label,
        groups,
        tasks,
        library_count,
    })
}

// ------------------------------------------------------------------ wiki

pub struct Link {
    pub href: String,
    pub title: String,
    pub missing: bool,
}

pub struct InfoRow {
    pub key: &'static str,
    pub links: Vec<Link>,
    pub text: String,
}

#[derive(Template)]
#[template(path = "wiki_article.html")]
pub struct ArticleTpl {
    layout: Layout,
    slug: String,
    title: String,
    html: String,
    toc: Vec<TocEntry>,
    info: Vec<InfoRow>,
    backlinks: Vec<Link>,
    categories: Vec<Link>,
    missing: bool,
    links_here_only: bool,
    can_focus: bool,
    is_focus: bool,
}

async fn link_index(st: &AppState) -> Result<LinkIndex, PageError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT title, slug FROM notes")
        .fetch_all(&st.pool)
        .await?;
    Ok(LinkIndex::new(&rows))
}

/// Notes linking to `key` (a title match key).
async fn backlinks(st: &AppState, key: &str) -> Result<Vec<Link>, PageError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT n.slug, n.title FROM note_links l JOIN notes n ON n.id = l.from_note_id
          WHERE l.target_key = ? ORDER BY n.title COLLATE NOCASE",
    )
    .bind(key)
    .fetch_all(&st.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(s, t)| Link {
            href: format!("/wiki/{s}"),
            title: t,
            missing: false,
        })
        .collect())
}

pub async fn wiki_article(State(st): State<AppState>, Path(slug): Path<String>) -> PageResult {
    article(st, slug, false).await
}

pub async fn wiki_links_here(State(st): State<AppState>, Path(slug): Path<String>) -> PageResult {
    article(st, slug, true).await
}

async fn article(st: AppState, slug: String, links_here_only: bool) -> PageResult {
    let row = sqlx::query(
        "SELECT n.id, n.apple_id, n.title, n.title_key, n.area, n.folder_path, n.rel_path, n.created_at, n.modified_at,
                p.name AS project, p.slug AS project_slug
           FROM notes n LEFT JOIN projects p ON p.id = n.project_id WHERE n.slug = ?",
    )
    .bind(&slug)
    .fetch_optional(&st.pool)
    .await?;
    let focus = setting(&st.pool, "learning_focus").await?;

    let Some(row) = row else {
        // "Wanted" page: not written yet, but maybe linked from elsewhere.
        let title = {
            let t: Option<String> = sqlx::query_scalar(
                "SELECT target_title FROM note_links WHERE target_key = ? LIMIT 1",
            )
            .bind(match_key(&slug))
            .fetch_optional(&st.pool)
            .await?;
            t.unwrap_or_else(|| {
                let s = slug.replace('-', " ");
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
        };
        let back = backlinks(&st, &match_key(&title)).await?;
        let mut resp = html(ArticleTpl {
            layout: layout(&st, "wiki", format!("Wiki: {title}")).await?,
            slug,
            title,
            html: "<p>This article has not been written yet. Create a note with this title in Apple Notes \
                   (under AndreiOS) and it will appear here after the next sync.</p>"
                .into(),
            toc: vec![],
            info: vec![],
            backlinks: back,
            categories: vec![],
            missing: true,
            links_here_only,
            can_focus: false,
            is_focus: false,
        })?;
        *resp.status_mut() = StatusCode::NOT_FOUND;
        return Ok(resp);
    };

    let id: i64 = row.get("id");
    let title: String = row.get("title");
    let area: String = row.get("area");
    let folder: String = row.get("folder_path");
    let md = read_markdown(&st.cfg.content_dir().join(row.get::<String, _>("rel_path")));

    let media_rows = sqlx::query("SELECT media_path, title, transcript FROM attachments WHERE note_id = ? AND media_path IS NOT NULL")
        .bind(id)
        .fetch_all(&st.pool)
        .await?;
    let media: HashMap<String, (Option<String>, Option<String>)> = media_rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("media_path"),
                (r.get("title"), r.get("transcript")),
            )
        })
        .collect();
    let media_fn = |src: &str| {
        media.get(src).map(|(t, tr)| MediaInfo {
            title: t.clone(),
            transcript: tr.clone(),
        })
    };
    let idx = link_index(&st).await?;
    let out = render(
        &md,
        &RenderCtx {
            links: &idx,
            self_slug: Some(&slug),
            media: &media_fn,
        },
    );

    // infobox
    let mut info = vec![InfoRow {
        key: "Area",
        links: vec![Link {
            href: format!("/area/{area}"),
            title: area_label(&area).into(),
            missing: false,
        }],
        text: String::new(),
    }];
    let parts: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() > 1 {
        info.push(InfoRow {
            key: "Topic",
            links: parts[1..]
                .iter()
                .map(|p| Link {
                    href: format!("/wiki/category/{}", slugify(p)),
                    title: p.to_string(),
                    missing: false,
                })
                .collect(),
            text: String::new(),
        });
    }
    if let (Some(name), Some(ps)) = (
        row.get::<Option<String>, _>("project"),
        row.get::<Option<String>, _>("project_slug"),
    ) {
        info.push(InfoRow {
            key: "Project",
            links: vec![Link {
                href: format!("/projects/{ps}"),
                title: name,
                missing: false,
            }],
            text: String::new(),
        });
    }
    let fmt_date = |s: String| s.get(..10).unwrap_or("").to_string();
    info.push(InfoRow {
        key: "Created",
        links: vec![],
        text: fmt_date(row.get("created_at")),
    });
    info.push(InfoRow {
        key: "Updated",
        links: vec![],
        text: fmt_date(row.get("modified_at")),
    });
    if !out.links.is_empty() {
        info.push(InfoRow {
            key: "Links to",
            links: out
                .links
                .iter()
                .take(8)
                .map(|(_, t)| match idx.resolve(t) {
                    Some(a) => Link {
                        href: format!("/wiki/{}", a.slug),
                        title: a.title.clone(),
                        missing: false,
                    },
                    None => Link {
                        href: format!("/wiki/{}", slugify(t)),
                        title: t.clone(),
                        missing: true,
                    },
                })
                .collect(),
            text: String::new(),
        });
    }
    info.push(InfoRow {
        key: "Source",
        links: vec![],
        text: "Apple Notes".into(),
    });

    let cats: Vec<(String, String)> = sqlx::query_as(
        "SELECT cat_key, category FROM note_categories WHERE note_id = ? ORDER BY rowid",
    )
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    let back = backlinks(&st, &row.get::<String, _>("title_key")).await?;

    html(ArticleTpl {
        layout: layout(&st, "wiki", format!("Wiki: {title}")).await?,
        is_focus: focus.as_deref() == Some(&format!("note:{slug}")),
        can_focus: area == "learning",
        slug,
        title,
        html: out.html,
        toc: if out.toc.len() >= 3 { out.toc } else { vec![] },
        info,
        backlinks: back,
        categories: cats
            .into_iter()
            .map(|(k, c)| Link {
                href: format!("/wiki/category/{k}"),
                title: c,
                missing: false,
            })
            .collect(),
        missing: false,
        links_here_only,
    })
}

pub struct ListSection {
    pub heading: String,
    pub href: String,
    pub items: Vec<Link>,
}

#[derive(Template)]
#[template(path = "wiki_list.html")]
pub struct WikiListTpl {
    layout: Layout,
    title: String,
    intro: String,
    sections: Vec<ListSection>,
    tab: &'static str,
    focus_value: String,
    is_focus: bool,
}

pub async fn wiki_home(State(st): State<AppState>) -> PageResult {
    // Main page: every top-level area with its topics, then wanted pages.
    let rows = sqlx::query(
        "SELECT slug, title, area, folder_path FROM notes ORDER BY title COLLATE NOCASE",
    )
    .fetch_all(&st.pool)
    .await?;
    let mut by_area: BTreeMap<usize, (String, String, Vec<Link>)> = BTreeMap::new();
    for r in &rows {
        let area: String = r.get("area");
        let order = AREAS.iter().position(|(k, _)| *k == area).unwrap_or(99);
        let e = by_area.entry(order).or_insert_with(|| {
            (
                area_label(&area).to_string(),
                format!("/area/{area}"),
                vec![],
            )
        });
        e.2.push(Link {
            href: format!("/wiki/{}", r.get::<String, _>("slug")),
            title: r.get("title"),
            missing: false,
        });
    }
    let mut sections: Vec<ListSection> = by_area
        .into_values()
        .map(|(heading, href, items)| ListSection {
            heading,
            href,
            items,
        })
        .collect();
    let wanted: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT target_key, min(target_title), count(*) AS n FROM note_links
          WHERE target_key NOT IN (SELECT title_key FROM notes) GROUP BY target_key ORDER BY n DESC LIMIT 30",
    )
    .fetch_all(&st.pool)
    .await?;
    if !wanted.is_empty() {
        sections.push(ListSection {
            heading: "Wanted articles".into(),
            href: String::new(),
            items: wanted
                .into_iter()
                .map(|(_, t, n)| Link {
                    href: format!("/wiki/{}", slugify(&t)),
                    title: format!("{t} ({n})"),
                    missing: true,
                })
                .collect(),
        });
    }
    let count = rows.len();
    html(WikiListTpl {
        layout: layout(&st, "wiki", "Wiki").await?,
        title: "Wiki".into(),
        intro: format!("{count} articles from Apple Notes, linked to each other. Search with /."),
        sections,
        tab: "home",
        focus_value: String::new(),
        is_focus: false,
    })
}

pub async fn wiki_all(State(st): State<AppState>) -> PageResult {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT slug, title FROM notes ORDER BY title COLLATE NOCASE")
            .fetch_all(&st.pool)
            .await?;
    let mut by_letter: BTreeMap<String, Vec<Link>> = BTreeMap::new();
    for (s, t) in rows {
        let letter = t
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let letter = if letter.chars().all(char::is_alphabetic) {
            letter
        } else {
            "#".into()
        };
        by_letter.entry(letter).or_default().push(Link {
            href: format!("/wiki/{s}"),
            title: t,
            missing: false,
        });
    }
    let n: usize = by_letter.values().map(Vec::len).sum();
    html(WikiListTpl {
        layout: layout(&st, "wiki", "Wiki: All articles").await?,
        title: "All articles".into(),
        intro: format!("{n} articles."),
        sections: by_letter
            .into_iter()
            .map(|(heading, items)| ListSection {
                heading,
                href: String::new(),
                items,
            })
            .collect(),
        tab: "all",
        focus_value: String::new(),
        is_focus: false,
    })
}

pub async fn wiki_random(State(st): State<AppState>) -> PageResult {
    let slug: Option<String> =
        sqlx::query_scalar("SELECT slug FROM notes ORDER BY random() LIMIT 1")
            .fetch_optional(&st.pool)
            .await?;
    Ok(Redirect::to(
        &slug
            .map(|s| format!("/wiki/{s}"))
            .unwrap_or_else(|| "/wiki".into()),
    )
    .into_response())
}

pub async fn wiki_category(State(st): State<AppState>, Path(key): Path<String>) -> PageResult {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT n.slug, n.title, c.category, n.folder_path FROM note_categories c JOIN notes n ON n.id = c.note_id
          WHERE c.cat_key = ? ORDER BY n.title COLLATE NOCASE",
    )
    .bind(&key)
    .fetch_all(&st.pool)
    .await?;
    let name = rows
        .first()
        .map(|r| r.2.clone())
        .ok_or_else(|| not_found("Category"))?;
    // subcategories: folder segments below this one
    let mut sub: BTreeMap<String, Vec<Link>> = BTreeMap::new();
    for (slug, title, _, folder) in &rows {
        let parts: Vec<&str> = folder.split('/').collect();
        let below = parts
            .iter()
            .position(|p| slugify(p) == key)
            .and_then(|i| parts.get(i + 1))
            .map(|s| s.to_string());
        sub.entry(below.unwrap_or_default())
            .or_default()
            .push(Link {
                href: format!("/wiki/{slug}"),
                title: title.clone(),
                missing: false,
            });
    }
    let sections = sub
        .into_iter()
        .map(|(s, items)| ListSection {
            href: if s.is_empty() {
                String::new()
            } else {
                format!("/wiki/category/{}", slugify(&s))
            },
            heading: if s.is_empty() { "Articles".into() } else { s },
            items,
        })
        .collect();
    let focus = setting(&st.pool, "learning_focus").await?;
    let is_learning: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM note_categories c JOIN notes n ON n.id=c.note_id WHERE c.cat_key=? AND n.area='learning')",
    )
    .bind(&key)
    .fetch_one(&st.pool)
    .await?;
    html(WikiListTpl {
        layout: layout(&st, "wiki", format!("Category: {name}")).await?,
        title: format!("Category: {name}"),
        intro: format!("{} articles in this category.", rows.len()),
        sections,
        tab: "category",
        focus_value: if is_learning && key != "learning" {
            format!("category:{key}")
        } else {
            String::new()
        },
        is_focus: focus.as_deref() == Some(&format!("category:{key}")),
    })
}

// ------------------------------------------------------------------ library

pub struct Tile {
    pub id: i64,
    pub title: String,
    pub kind_label: &'static str,
    pub meta: String,
    pub poster: Option<String>,
    pub score: Option<String>,
    pub status: Option<String>,
    pub n: usize,
}

pub struct Shelf {
    pub title: String,
    pub tiles: Vec<Tile>,
    pub add_kind: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "library.html")]
pub struct LibraryTpl {
    layout: Layout,
    kind: String,
    shelves: Vec<Shelf>,
    total: usize,
    tmdb_missing: bool,
}

#[derive(Deserialize)]
pub struct LibraryQ {
    kind: Option<String>,
}

fn kind_label(k: &str) -> &'static str {
    match k {
        "film" => "FILM",
        "tv" => "TV",
        _ => "BOOK",
    }
}

fn fmt_score(s: f64) -> String {
    if (s - s.round()).abs() < f64::EPSILON {
        format!("{s:.0}")
    } else {
        format!("{s:.1}")
    }
}

pub async fn library(State(st): State<AppState>, Query(q): Query<LibraryQ>) -> PageResult {
    let kind = q
        .kind
        .filter(|k| matches!(k.as_str(), "film" | "tv" | "book"))
        .unwrap_or_default();
    let rows = sqlx::query(
        "SELECT id, kind, title, year, genres, creator, poster_path, status,
                COALESCE(user_score, note_score) AS score
           FROM library_items WHERE (? = '' OR kind = ?) ORDER BY created_at DESC, id DESC",
    )
    .bind(&kind)
    .bind(&kind)
    .fetch_all(&st.pool)
    .await?;
    let total = rows.len();
    let tiles: Vec<(String, Tile)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let k: String = r.get("kind");
            let year: Option<String> = r.get("year");
            let genre: Option<String> = r
                .get::<Option<String>, _>("genres")
                .map(|g| g.split(", ").next().unwrap_or("").to_string());
            let meta = [year, genre]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            (
                k.clone(),
                Tile {
                    id: r.get("id"),
                    title: r.get("title"),
                    kind_label: kind_label(&k),
                    meta,
                    poster: r.get("poster_path"),
                    score: r.get::<Option<f64>, _>("score").map(fmt_score),
                    status: r.get("status"),
                    n: i + 1,
                },
            )
        })
        .collect();

    let mut shelves = Vec::new();
    let mut tiles = tiles;
    // Netflix-style rows: what you're in the middle of, your list, then by kind.
    let take = |tiles: &mut Vec<(String, Tile)>, f: &dyn Fn(&(String, Tile)) -> bool| {
        let (a, b): (Vec<_>, Vec<_>) = std::mem::take(tiles).into_iter().partition(|t| f(t));
        *tiles = b;
        a.into_iter().map(|(_, t)| t).collect::<Vec<_>>()
    };
    let cont = take(&mut tiles, &|t| {
        t.1.status.as_deref() == Some("in_progress")
    });
    if !cont.is_empty() {
        shelves.push(Shelf {
            title: "Continue".into(),
            tiles: cont,
            add_kind: None,
        });
    }
    let want = take(&mut tiles, &|t| t.1.status.as_deref() == Some("want"));
    if !want.is_empty() {
        shelves.push(Shelf {
            title: "My list".into(),
            tiles: want,
            add_kind: None,
        });
    }
    for (k, label) in [("film", "Films"), ("tv", "TV shows"), ("book", "Books")] {
        if !kind.is_empty() && kind != k {
            continue;
        }
        let row = take(&mut tiles, &|t| t.0 == k);
        let title = match (kind.is_empty(), k) {
            (false, "book") => "Read".to_string(),
            (false, _) => "Watched".to_string(),
            _ => label.to_string(),
        };
        shelves.push(Shelf {
            title,
            tiles: row,
            add_kind: Some(k),
        });
    }
    html(LibraryTpl {
        layout: layout(&st, "library", "Library").await?,
        kind,
        shelves,
        total,
        tmdb_missing: st.cfg.tmdb_api_key.is_none(),
    })
}

#[derive(Template)]
#[template(path = "library_item.html")]
pub struct ItemTpl {
    layout: Layout,
    id: i64,
    kind: String,
    kind_label: &'static str,
    title: String,
    year: String,
    creator: String,
    creator_label: &'static str,
    genres: Vec<String>,
    runtime: String,
    overview: String,
    overview_source: &'static str,
    poster: Option<String>,
    score: Option<String>,
    score_value: String,
    score_from_note: bool,
    status: String,
    note_html: Option<String>,
    note_slug: Option<String>,
    summary: String,
    link_url: Option<String>,
    lookup: String,
    meta_status: String,
    from_notes: bool,
    tmdb_missing: bool,
}

pub async fn library_item(State(st): State<AppState>, Path(id): Path<i64>) -> PageResult {
    let r = sqlx::query(
        "SELECT i.*, n.slug AS note_slug, n.rel_path FROM library_items i LEFT JOIN notes n ON n.id = i.note_id WHERE i.id = ?",
    )
    .bind(id)
    .fetch_optional(&st.pool)
    .await?
    .ok_or_else(|| not_found("Title"))?;
    let kind: String = r.get("kind");
    let user: Option<f64> = r.get("user_score");
    let note: Option<f64> = r.get("note_score");
    let score = user.or(note);
    let external: Option<String> = r.get("external_id");
    let note_html = match r.get::<Option<String>, _>("rel_path") {
        Some(p) => {
            let md = read_markdown(&st.cfg.content_dir().join(p));
            let idx = link_index(&st).await?;
            let none = |_: &str| None;
            let slug: Option<String> = r.get("note_slug");
            let out = render(
                &md,
                &RenderCtx {
                    links: &idx,
                    self_slug: slug.as_deref(),
                    media: &none,
                },
            );
            Some(out.html).filter(|h| !h.trim().is_empty())
        }
        None => None,
    };
    let title: String = r.get("title");
    html(ItemTpl {
        layout: layout(&st, "library", title.clone()).await?,
        id,
        kind_label: match kind.as_str() {
            "film" => "Film",
            "tv" => "TV series",
            _ => "Book",
        },
        creator_label: match kind.as_str() {
            "film" => "Director",
            "tv" => "Created by",
            _ => "Author",
        },
        title,
        year: r.get::<Option<String>, _>("year").unwrap_or_default(),
        creator: r.get::<Option<String>, _>("creator").unwrap_or_default(),
        genres: r
            .get::<Option<String>, _>("genres")
            .map(|g| {
                g.split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        runtime: r.get::<Option<String>, _>("runtime").unwrap_or_default(),
        overview: r.get::<Option<String>, _>("overview").unwrap_or_default(),
        overview_source: match external.as_deref() {
            Some(e) if e.starts_with("tmdb") => "TMDB",
            Some(e) if e.starts_with("openlibrary") => "Open Library",
            _ => "",
        },
        poster: r.get("poster_path"),
        score: score.map(fmt_score),
        score_value: user.map(fmt_score).unwrap_or_default(),
        score_from_note: user.is_none() && note.is_some(),
        status: r.get::<Option<String>, _>("status").unwrap_or_default(),
        note_slug: r.get("note_slug"),
        note_html,
        summary: r.get::<Option<String>, _>("summary").unwrap_or_default(),
        link_url: r.get("link_url"),
        lookup: r
            .get::<Option<String>, _>("lookup_override")
            .unwrap_or_default(),
        meta_status: r.get("meta_status"),
        from_notes: r.get::<Option<String>, _>("source_key").is_some(),
        tmdb_missing: st.cfg.tmdb_api_key.is_none() && kind != "book",
        kind,
    })
}

// ------------------------------------------------------------------ sync log

pub struct RunView {
    pub when: String,
    pub ok: bool,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Template)]
#[template(path = "sync.html")]
pub struct SyncTpl {
    layout: Layout,
    runs: Vec<RunView>,
    checks: Vec<(String, bool, String)>,
}

pub async fn sync_log(State(st): State<AppState>) -> PageResult {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT report FROM sync_runs ORDER BY id DESC LIMIT 20")
            .fetch_all(&st.pool)
            .await?;
    let runs = rows
        .into_iter()
        .filter_map(|(r,)| serde_json::from_str::<crate::sync::SyncReport>(&r).ok())
        .map(|r| RunView {
            when: chrono::DateTime::parse_from_rfc3339(&r.finished_at)
                .map(|d| {
                    d.with_timezone(&st.cfg.tz)
                        .format("%a %-d %b %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default(),
            ok: r.ok,
            lines: r.lines,
            warnings: r.warnings,
            errors: r.errors,
        })
        .collect();
    let cfg = &st.cfg;
    let readable = |p: &std::path::Path| std::fs::File::open(p).is_ok();
    let whisper = crate::sync::audio::whisper_ready(cfg);
    let checks = vec![
        (
            "Apple Notes database".to_string(),
            readable(&cfg.notes_db),
            cfg.notes_db.display().to_string(),
        ),
        (
            "Reminders stores".to_string(),
            std::fs::read_dir(&cfg.reminders_stores).is_ok(),
            cfg.reminders_stores.display().to_string(),
        ),
        (
            "Calendar database".to_string(),
            readable(&cfg.calendar_db),
            cfg.calendar_db.display().to_string(),
        ),
        (
            "Transcription (Whisper model)".to_string(),
            whisper.is_ok(),
            match whisper {
                Ok(b) => b.display().to_string(),
                Err(e) => e,
            },
        ),
        (
            "TMDB key (films & TV metadata)".to_string(),
            cfg.tmdb_api_key.is_some(),
            if cfg.tmdb_api_key.is_some() {
                "set".into()
            } else {
                "TMDB_API_KEY not set".into()
            },
        ),
        (
            "Data folder".to_string(),
            true,
            cfg.data_dir.display().to_string(),
        ),
    ];
    html(SyncTpl {
        layout: layout(&st, "sync", "Sync").await?,
        runs,
        checks,
    })
}
