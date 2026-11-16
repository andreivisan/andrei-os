# AndreiOS

A second brain that compiles itself from Apple Notes, Reminders and Calendar.
Rust + Axum + SQLite; notes become Markdown files rendered as a wiki.

You keep writing where you already write:

| You write in…      | It becomes…                                                                 |
|--------------------|-----------------------------------------------------------------------------|
| **Apple Notes**, folder `AndreiOS/` | wiki articles (Markdown), project notes, library titles, checklist tasks |
| **Reminders**      | tasks; a tag like `#HudAgents` puts it on that project's board, `#Admin` / `#Health` in that area |
| **Calendar**       | today's appointments                                                        |

## Folder conventions (Apple Notes)

```
AndreiOS/
  Learning/<topic>/…        wiki articles; topics become categories ("Rust")
  Projects/<project>/…      one project per folder (nested folders = sub-projects)
  Health/  Resources/  Admin/
  Interests/Books|Movies|TV Shows/
        one note per title  → library item (write "Score: 8/10" or "Status: reading" in it)
        a note whose title starts with the folder name ("Books I read in 2023")
                            → one library item per line or link
```

Notes outside the `AndreiOS` folder are ignored (change with `ANDREIOS_NOTES_ROOT`).

## What it does

- **Dashboard**: greeting, today's tasks (due today, overdue, in the weekday
  Reminders list for today, or in progress), today's appointments, current
  learning focus, link to the wiki.
- **Tasks**: one table, many views — dashboard, per-project Kanban (drag and
  drop), the Tasks table with filters, area pages. Edits made in the app stick
  until you edit the same item at the source. Completing a task that came from
  Reminders also completes it in Reminders (the only thing ever written back).
- **Projects**: Kanban, the project's own notes, and related notes from
  Learning / Resources / Admin (linked, mentioning the project, or pinned).
- **Wiki**: Wikipedia-style articles with contents box, infobox, "what links
  here", categories, wanted pages, full-text search (`/`). Links come from
  `[[Wiki links]]` and automatically from mentions of other articles' titles.
  Native Apple Notes tables, checklists, headings, code blocks and the literal
  Markdown you typed (backticks, `**bold**`) are rendered properly.
- **Library**: Netflix-style shelves; IMDB-style title pages with your score,
  your summary, synopsis, credits. Metadata from TMDB (films, TV) and Open
  Library (books); posters are cached locally.
- **Voice notes**: audio recordings in notes get a player and a transcript —
  Apple's own when available, otherwise whisper.cpp (English and Romanian).

## Run it

```sh
cargo run --release            # http://127.0.0.1:4000, syncs every 15 min
cargo run --release -- sync    # one sync, then exit
cargo run --release -- transcribe [--force]
```

The terminal you run it from needs **Full Disk Access** (System Settings →
Privacy & Security) to read the Apple databases. The first time you tick a
Reminders task, macOS asks to let AndreiOS control Reminders.

Data lives in `~/Library/Application Support/AndreiOS/` (`andrei-os.db`,
`content/` Markdown, `media/`, `posters/`).

### Configuration (environment variables)

| Variable | Default |
|---|---|
| `ANDREIOS_DATA_DIR` | `~/Library/Application Support/AndreiOS` |
| `ANDREIOS_BIND` | `127.0.0.1:4000` |
| `ANDREIOS_TZ` | `Europe/Bucharest` |
| `ANDREIOS_SYNC_INTERVAL_MINS` | `15` (0 = off) |
| `ANDREIOS_NOTES_ROOT` | `AndreiOS` |
| `ANDREIOS_CALENDARS` | all except Birthdays, Siri suggestions, Scheduled Reminders; e.g. `Business,Life,Health & Fitness` |
| `TMDB_API_KEY` | none — get a free key at themoviedb.org (v3 key or v4 read token) |
| `ANDREIOS_WHISPER_BIN` / `ANDREIOS_WHISPER_MODEL` | `/opt/homebrew/bin/whisper-cli` / `<data>/models/ggml-large-v3-turbo.bin` |

### Transcription

```sh
brew install whisper.cpp
mkdir -p ~/Library/Application\ Support/AndreiOS/models
curl -L -o ~/Library/Application\ Support/AndreiOS/models/ggml-large-v3-turbo.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin
andrei-os transcribe --force   # replace Apple's English-only transcripts too
```

### Run it in the background (launchd)

```sh
cargo install --path .
sed "s/YOUR_USER/$USER/g" deploy/com.andreios.server.plist > ~/Library/LaunchAgents/com.andreios.server.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.andreios.server.plist
```

Then add `~/.cargo/bin/andrei-os` to **Full Disk Access** (launchd processes
don't inherit your terminal's permission). Logs: `~/Library/Logs/AndreiOS.log`.
The **Sync log** page in the app shows which sources are readable.

## Security

The server only answers on localhost, refuses requests whose Host header is
not local (DNS rebinding) and only accepts same-origin JSON for changes
(CSRF). Raw HTML inside notes is shown as text, never executed. Apple
databases are opened read-only.

## Code map

```
src/apple/        readers for Apple's stores (read-only) + Reminders write-back
  notes/          protobuf body → Markdown, native tables (CRDT), transcripts
src/sync/         sync engine: notes → Markdown files + index, reminders → tasks,
                  calendar → events, library metadata, whisper transcription
src/render.rs     Markdown → HTML (wiki links, auto-links, TOC, audio players)
src/web/          Axum routes, pages (Askama templates), JSON API
templates/ static/ migrations/
docs/apple-formats.md   the verified Apple storage formats
```

Tests: `cargo test`. Verification against real Notes data (kept outside the
repo because it is personal): `ANDREIOS_FIXTURES=<dir> cargo test --test apple_fixtures`.

The `index.html`, `project.html`, `wiki.html`, `library.html` and `assets/`
files are the original static design POC.
