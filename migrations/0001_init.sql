-- AndreiOS schema.
-- Content (notes) lives as Markdown files under <data>/content; this DB is the
-- index over that content plus all state (tasks, events, library, settings).

-- ---------------------------------------------------------------- projects
-- One row per folder under AndreiOS/Projects in Apple Notes (nested folders
-- become child projects). `tag_key` is the normalised Reminders tag that routes
-- tasks to this project (e.g. "hudagents" for #HudAgents).
CREATE TABLE projects (
    id           INTEGER PRIMARY KEY,
    slug         TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    parent_id    INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    folder_path  TEXT NOT NULL UNIQUE,
    tag_key      TEXT NOT NULL UNIQUE,
    description  TEXT,
    archived     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- ---------------------------------------------------------------- notes
-- area: learning | projects | health | interests | resources | admin | inbox
CREATE TABLE notes (
    id           INTEGER PRIMARY KEY,
    apple_id     TEXT UNIQUE,                 -- ZIDENTIFIER of the Apple note
    slug         TEXT NOT NULL UNIQUE,
    title        TEXT NOT NULL,
    title_key    TEXT NOT NULL,               -- normalised title for [[link]] resolution
    area         TEXT NOT NULL,
    folder_path  TEXT NOT NULL,               -- "Learning/Rust" (relative to AndreiOS)
    project_id   INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    rel_path     TEXT NOT NULL,               -- path of the .md file under content/
    lead         TEXT NOT NULL DEFAULT '',    -- first paragraph, plain text
    content_hash TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    modified_at  TEXT NOT NULL
);
CREATE INDEX notes_title_key ON notes(title_key);
CREATE INDEX notes_area ON notes(area);

-- Outgoing [[links]] from a note; target_key is the normalised target title,
-- resolved against notes.title_key at query time (so links to notes that do
-- not exist yet still show up as "wanted" pages).
CREATE TABLE note_links (
    from_note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    target_key   TEXT NOT NULL,
    target_title TEXT NOT NULL,
    PRIMARY KEY (from_note_id, target_key)
);
CREATE INDEX note_links_target ON note_links(target_key);

-- #tags and folder-derived categories.
CREATE TABLE note_categories (
    note_id  INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    category TEXT NOT NULL,                   -- display form, e.g. "Rust"
    cat_key  TEXT NOT NULL,                   -- normalised, e.g. "rust"
    PRIMARY KEY (note_id, cat_key)
);
CREATE INDEX note_categories_key ON note_categories(cat_key);

-- Full-text index (rowid = notes.id). Stores its own copy of the text so
-- snippet() can show search hits in context.
CREATE VIRTUAL TABLE notes_fts USING fts5(
    title, body,
    tokenize='unicode61 remove_diacritics 2'
);

-- Attachments referenced by notes (audio, images, links). Tables are inlined
-- into the Markdown and are not stored here.
CREATE TABLE attachments (
    id             INTEGER PRIMARY KEY,
    apple_id       TEXT NOT NULL UNIQUE,
    note_id        INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL,             -- audio | image | url | file
    uti            TEXT NOT NULL,
    source_path    TEXT,                      -- original file inside the Notes container
    media_path     TEXT,                      -- copy under <data>/media, served at /media/
    url            TEXT,
    title          TEXT,
    duration_secs  REAL,
    transcript     TEXT,
    transcript_source TEXT,                   -- apple | whisper
    transcribed_at TEXT
);

-- Manually pinned notes on a project page (in addition to the automatic
-- links/tags/backlinks relation).
CREATE TABLE project_notes (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    note_id    INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, note_id)
);

-- ---------------------------------------------------------------- tasks
-- THE task database. Dashboard, Kanban boards and the task table are views
-- over this one table (Notion-style).
--
-- Sync rule: a field coming from a source (Reminders / Notes) only overwrites
-- the app's value when the source value itself changed since the last sync
-- (tracked by source_hash / source_done). So edits made in the app stick until
-- the item is edited again at the source.
CREATE TABLE tasks (
    id             INTEGER PRIMARY KEY,
    title          TEXT NOT NULL,
    notes          TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
    position       REAL NOT NULL DEFAULT 0,   -- order within a Kanban column
    project_id     INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    area           TEXT,                      -- admin | health | learning | ... when not a project
    due_date       TEXT,                      -- YYYY-MM-DD (local)
    due_time       TEXT,                      -- HH:MM (local), NULL for all-day
    source         TEXT NOT NULL CHECK (source IN ('app','reminders','notes')),
    source_id      TEXT,                      -- reminder UUID / note-id#checklist-uuid
    source_list    TEXT,                      -- Reminders list name (weekday lists drive "today")
    source_note_id INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    source_hash    TEXT,                      -- hash of source fields at last sync
    source_done    INTEGER,                   -- completion state at source at last sync
    writeback      TEXT,                      -- pending write-back: 'complete' | 'uncomplete'
    completed_at   TEXT,
    deleted_at     TEXT,                      -- soft delete so sync never resurrects it
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE (source, source_id)
);
CREATE INDEX tasks_project ON tasks(project_id, status, position);
CREATE INDEX tasks_due ON tasks(due_date);

-- ---------------------------------------------------------------- events
CREATE TABLE events (
    id            INTEGER PRIMARY KEY,
    occurrence_id TEXT NOT NULL UNIQUE,       -- event uid + occurrence start
    event_uid     TEXT NOT NULL,
    calendar      TEXT NOT NULL,
    title         TEXT NOT NULL,
    start_at      TEXT NOT NULL,              -- local, "YYYY-MM-DDTHH:MM:SS"
    end_at        TEXT NOT NULL,
    all_day       INTEGER NOT NULL DEFAULT 0,
    location      TEXT
);
CREATE INDEX events_start ON events(start_at);

-- ---------------------------------------------------------------- library
-- One item per note in Interests/{Books,Movies,TV Shows}; a "list note"
-- (title starting with the folder name, e.g. "Books I read in 2023")
-- contributes one item per line instead. Items can also be added in the app.
CREATE TABLE library_items (
    id            INTEGER PRIMARY KEY,
    source_key    TEXT UNIQUE,                -- note:<apple id> | list:<apple id>:<title key> | NULL (app)
    note_id       INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('film','tv','book')),
    title         TEXT NOT NULL,
    status        TEXT,                       -- want | in_progress | finished
    note_score    REAL,                       -- score written in the note ("Score: 8/10")
    user_score    REAL,                       -- score set in the app (wins)
    summary       TEXT,                       -- written in the app (note-backed items use the note body)
    lookup_title  TEXT,                       -- full title used for the metadata search (from the source)
    lookup_override TEXT,                     -- correction typed in the app (wins)
    link_url      TEXT,                       -- e.g. the shop link a list item came from
    -- external metadata (TMDB / Open Library)
    external_id   TEXT,
    year          TEXT,
    creator       TEXT,                       -- director / showrunner / author
    genres        TEXT,
    runtime       TEXT,
    overview      TEXT,                       -- official synopsis
    poster_path   TEXT,                       -- cached under <data>/posters, served at /posters/
    meta_status   TEXT NOT NULL DEFAULT 'pending', -- pending | ok | not_found | error | no_key
    meta_fetched_at TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- ---------------------------------------------------------------- misc
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE sync_runs (
    id          INTEGER PRIMARY KEY,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    ok          INTEGER,
    report      TEXT
);
