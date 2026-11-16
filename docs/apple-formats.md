# Apple data formats (as verified on macOS, November 2026)

AndreiOS reads Apple's local databases directly, read-only. Apple does not
document these formats; everything below was verified against real data on
this Mac. When a macOS update breaks something, start here.

All timestamps are **Core Data seconds**: `unix = value + 978307200`.
Core Data merges entities into shared tables and suffixes clashing column
names with numbers that change between OS versions (`ZTITLE1`, `ZREMINDER3`);
the code discovers them at startup instead of hardcoding them. Entity numbers
(`Z_ENT`) are resolved by name through `Z_PRIMARYKEY`.

Open the stores with SQLite `mode=ro`, never `immutable=1`: the apps keep the
newest data in the `-wal` file, which `immutable` ignores.

## Apple Notes — `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite`

- Notes, folders, attachments, media and accounts all live in
  `ZICCLOUDSYNCINGOBJECT` (entities `ICNote`, `ICFolder`, `ICAttachment`,
  `ICMedia`, `ICAccount`). Bodies are in `ZICNOTEDATA.ZDATA` (join on
  `ZNOTEDATA`).
- Live notes: `ZFOLDER IS NOT NULL`, not `ZMARKEDFORDELETION`, not
  `ZISPASSWORDPROTECTED`, folder not of `ZFOLDERTYPE = 1` (Recently Deleted).
- Folder path: walk `ZPARENT` up from `ZTITLE2`.

### Body: gzip → protobuf

```
NoteStoreProto { 2: Document { 3: Note { 2: note_text, 5: AttributeRun[] } } }
AttributeRun   { 1: length (UTF-16 units), 2: ParagraphStyle, 5: font_weight (1 bold, 2 italic, 3 both),
                 6: underlined, 7: strikethrough, 9: link (URL), 12: AttachmentInfo }
ParagraphStyle { 1: style_type, 4: indent_amount, 5: Checklist { 1: uuid, 2: done }, 7: list start, 8: block quote }
AttachmentInfo { 1: attachment identifier, 2: type UTI }
```

`style_type`: absent = body, 0 title, 1 heading, 2 subheading, 4 monospaced,
100 bullet, 101 dashed, 102 numbered, 103 checklist.

- Run lengths are **UTF-16 code units** (verified with emoji).
- `\n` ends a paragraph; **U+2028 is a soft line break inside a paragraph**.
- An attachment is a single U+FFFC with `attachment_info` on its run.
- A monospaced *font* on body text is common in pasted text and does not mean
  code; only `style_type = 4` does.

### Tables (`com.apple.notes.table`): `ZMERGEABLEDATA1`, gzip → CRDT protobuf

```
{2: {3: MergeableDataObjectData { 3: entry[], 4: key[], 5: type[], 6: uuid[] }}}
entry { 1: RegisterLatest{2: ObjectID}, 6: Dictionary, 10: Note (cell), 13: CustomMap{1: type idx, 3: {1: key idx, 2: ObjectID}}, 16: OrderedSet }
ObjectID { 2: uint, 3: double, 4: string, 6: object index }
```

The `com.apple.notes.ICTable` map holds `crRows`, `crColumns` (ordered sets),
`cellColumns` (column → {row → cell note}) and `crTableColumnDirection`.
Order: the ordered set's array text is N × U+FFFC; the attachment whose
`index` equals a placeholder's position carries the UUID of that slot; the
ordering dictionary maps UUID objects (`UUIDIndex` → `uuid[]`) to the element.
Missing cells are simply absent. Verified on 35 tables (all left-to-right;
right-to-left handling follows the schema but is untested).

### Audio
- Body references only `com.apple.m4a-audio` attachments; the
  `public.mpeg-4-audio` rows are byte-identical child copies — ignore them.
- File: `Accounts/<account ZIDENTIFIER>/Media/<media ZIDENTIFIER>/<media ZGENERATION1>/<ZFILENAME>`
  (account from the attachment's `ZACCOUNT*`, media via `ZMEDIA`).
- Apple's own transcript: the audio attachment's `ZMERGEABLEDATA1` (not
  gzipped) is a bare MergeableDataObjectData with `ICTTTranscriptSegment`
  objects (`text`, `timestamp`), stored out of order: sort by timestamp.

### Link cards (`public.url`)
No file; `ZURLSTRING`, `ZTITLE`, `ZSUMMARY` on the attachment row.

## Reminders — `~/Library/Group Containers/group.com.apple.reminders/Container_v1/Stores/Data-*.sqlite`

- One store per account; read them all (some are empty shells).
- Reminders: `ZREMCDREMINDER` joined to lists in `ZREMCDBASELIST`
  (entity `REMCDList`, excluding smart lists).
- Tags: `ZREMCDOBJECT` rows of entity `REMCDHashtag`; a numbered
  `ZREMINDER*` column points at the reminder, `ZHASHTAGLABEL` at
  `ZREMCDHASHTAGLABEL.ZNAME`. Tags are not in the AppleScript dictionary.
- Due dates: timed = UTC instant; **all-day = floating date stored at 00:00 UTC**
  (take the UTC date, do not shift it).
- Identity: `ZCKIDENTIFIER` = AppleScript id `x-apple-reminder://<UUID>`.
- Write-back (the only write we do): AppleScript
  `set completed of reminder id "<id>" to <bool>`. Never write the SQLite
  file directly (it would break iCloud sync state).

## Calendar — `~/Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb`

(`~/Library/Calendars` is empty on current macOS.)

- Events are `CalendarItem` rows with `entity_type = 2`; skip `status = 3`
  (cancelled) and `hidden`.
- `OccurrenceCache` holds pre-expanded occurrences (recurring events
  included) for about ±2 years: real start =
  `COALESCE(occurrence_start_date, occurrence_date)`; multi-day events have
  one row per day (dedupe on event + start).
- In the cache, timed occurrences are UTC instants; all-day ones are shifted to
  device-local midnight.
- "Scheduled Reminders" mirrors Reminders items (`x-apple-reminderkit://` ids)
  and is skipped because Reminders are synced directly.
