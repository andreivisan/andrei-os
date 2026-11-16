//! NoteStore.sqlite reader. Core Data merges all entities into
//! ZICCLOUDSYNCINGOBJECT and suffixes clashing column names with numbers that
//! change between macOS versions (ZTITLE1, ZCREATIONDATE3, ...), so columns are
//! discovered by prefix at startup instead of hardcoded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sqlx::{Row, SqlitePool};

use crate::apple::{columns, entity_id};

#[derive(Debug, Clone)]
pub struct RawNote {
    pub pk: i64,
    pub identifier: String,
    pub title_hint: String,
    /// Folder path from the account root, e.g. "AndreiOS/Learning/Rust".
    pub folder_path: String,
    pub created: Option<f64>,
    pub modified: Option<f64>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RawAttachment {
    pub identifier: String,
    pub uti: String,
    pub title: Option<String>,
    pub url: Option<String>,
    /// Gzipped table CRDT or (not gzipped) audio transcript.
    pub mergeable: Option<Vec<u8>>,
    /// Resolved file on disk, if the attachment has media and it exists.
    pub file: Option<PathBuf>,
}

pub struct NoteStore {
    pool: SqlitePool,
    container: PathBuf,
    col_title: String,
    col_created: String,
    col_modified: String,
    col_mergeable: String,
    col_generation: Option<String>,
    col_account: Option<String>,
    note_ent: i64,
    folder_ent: i64,
}

/// Among `cols`, the ones equal to `base` or `base` + digits.
fn variants<'a>(cols: &'a [String], base: &str) -> Vec<&'a String> {
    cols.iter()
        .filter(|c| {
            c.strip_prefix(base)
                .is_some_and(|r| r.chars().all(|d| d.is_ascii_digit()))
        })
        .collect()
}

impl NoteStore {
    pub async fn open(db: &Path, container: &Path) -> anyhow::Result<Self> {
        let pool = crate::db::open_apple_readonly(db).await?;
        let cols = columns(&pool, "ZICCLOUDSYNCINGOBJECT").await?;
        let note_ent = entity_id(&pool, "ICNote")
            .await?
            .ok_or_else(|| anyhow::anyhow!("no ICNote entity"))?;
        let folder_ent = entity_id(&pool, "ICFolder")
            .await?
            .ok_or_else(|| anyhow::anyhow!("no ICFolder entity"))?;
        let media_ent = entity_id(&pool, "ICMedia").await?;
        let attachment_ent = entity_id(&pool, "ICAttachment").await?;

        // Pick, among numbered variants, the column populated for notes.
        async fn most_populated(
            pool: &SqlitePool,
            cands: Vec<&String>,
            ent: i64,
        ) -> anyhow::Result<Option<String>> {
            let mut best: Option<(String, i64)> = None;
            for c in cands {
                let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT = ? AND {c} IS NOT NULL"
                )))
                .bind(ent)
                .fetch_one(pool)
                .await?;
                if n > 0 && best.as_ref().is_none_or(|(_, b)| n > *b) {
                    best = Some((c.clone(), n));
                }
            }
            Ok(best.map(|b| b.0))
        }
        let need = |name: &str, v: Option<String>| {
            v.ok_or_else(|| anyhow::anyhow!("NoteStore: no populated {name} column"))
        };
        let col_title = need(
            "ZTITLE*",
            most_populated(&pool, variants(&cols, "ZTITLE"), note_ent).await?,
        )?;
        let col_created = need(
            "ZCREATIONDATE*",
            most_populated(&pool, variants(&cols, "ZCREATIONDATE"), note_ent).await?,
        )?;
        let col_modified = need(
            "ZMODIFICATIONDATE*",
            most_populated(&pool, variants(&cols, "ZMODIFICATIONDATE"), note_ent).await?,
        )?;
        let col_mergeable = match attachment_ent {
            Some(a) => most_populated(&pool, variants(&cols, "ZMERGEABLEDATA"), a).await?,
            None => None,
        }
        .unwrap_or_else(|| "ZMERGEABLEDATA1".into());
        let col_generation = match media_ent {
            Some(m) => most_populated(&pool, variants(&cols, "ZGENERATION"), m).await?,
            None => None,
        };
        let col_account = match attachment_ent {
            Some(a) => most_populated(&pool, variants(&cols, "ZACCOUNT"), a).await?,
            None => None,
        };
        Ok(Self {
            pool,
            container: container.to_path_buf(),
            col_title,
            col_created,
            col_modified,
            col_mergeable,
            col_generation,
            col_account,
            note_ent,
            folder_ent,
        })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    /// folder Z_PK -> full path ("AndreiOS/Learning/Rust"); excludes trash.
    pub async fn folder_paths(&self) -> anyhow::Result<HashMap<i64, String>> {
        let rows = sqlx::query(
            "SELECT Z_PK, ZTITLE2, CAST(ZPARENT AS INTEGER) AS ZPARENT, CAST(COALESCE(ZFOLDERTYPE,0) AS INTEGER) AS ftype,
                    CAST(COALESCE(ZMARKEDFORDELETION,0) AS INTEGER) AS deleted
               FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT = ?",
        )
        .bind(self.folder_ent)
        .fetch_all(&self.pool)
        .await?;
        struct F {
            name: String,
            parent: Option<i64>,
            skip: bool,
        }
        let folders: HashMap<i64, F> = rows
            .iter()
            .map(|r| {
                let ftype: i64 = r.get("ftype");
                let deleted: i64 = r.get("deleted");
                (
                    r.get::<i64, _>("Z_PK"),
                    F {
                        name: r
                            .get::<Option<String>, _>("ZTITLE2")
                            .unwrap_or_default()
                            .replace('/', "∕"),
                        parent: r.get::<Option<i64>, _>("ZPARENT").filter(|p| *p != 0),
                        skip: ftype == 1 || deleted != 0, // 1 = Recently Deleted
                    },
                )
            })
            .collect();
        let mut out = HashMap::new();
        for &pk in folders.keys() {
            let mut parts = Vec::new();
            let mut cur = Some(pk);
            let mut skip = false;
            let mut guard = 0;
            while let Some(c) = cur {
                let Some(f) = folders.get(&c) else { break };
                skip |= f.skip;
                parts.push(f.name.clone());
                cur = f.parent;
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            if !skip {
                parts.reverse();
                out.insert(pk, parts.join("/"));
            }
        }
        Ok(out)
    }

    /// All live notes (not deleted, not password protected, not in trash).
    pub async fn notes(&self) -> anyhow::Result<Vec<RawNote>> {
        let folders = self.folder_paths().await?;
        let sql = format!(
            "SELECT n.Z_PK AS pk, n.ZIDENTIFIER AS ident, n.{t} AS title, CAST(n.ZFOLDER AS INTEGER) AS folder,
                    CAST(n.{c} AS REAL) AS created, CAST(n.{m} AS REAL) AS modified, d.ZDATA AS data
               FROM ZICCLOUDSYNCINGOBJECT n
               JOIN ZICNOTEDATA d ON d.Z_PK = n.ZNOTEDATA
              WHERE n.Z_ENT = ? AND n.ZFOLDER IS NOT NULL
                AND COALESCE(n.ZMARKEDFORDELETION, 0) = 0
                AND COALESCE(n.ZISPASSWORDPROTECTED, 0) = 0
                AND d.ZDATA IS NOT NULL",
            t = self.col_title,
            c = self.col_created,
            m = self.col_modified,
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(self.note_ent)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for r in rows {
            let folder: i64 = r.get("folder");
            let Some(path) = folders.get(&folder) else {
                continue;
            }; // trash / deleted folder
            let Some(identifier) = r.get::<Option<String>, _>("ident") else {
                continue;
            };
            out.push(RawNote {
                pk: r.get("pk"),
                identifier,
                title_hint: r.get::<Option<String>, _>("title").unwrap_or_default(),
                folder_path: path.clone(),
                created: r.get("created"),
                modified: r.get("modified"),
                body: r.get("data"),
            });
        }
        Ok(out)
    }

    /// Attachments by identifier for the given ids.
    pub async fn attachments(
        &self,
        ids: &[String],
    ) -> anyhow::Result<HashMap<String, RawAttachment>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(200) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let gen_col = self
                .col_generation
                .as_deref()
                .map(|g| format!("m.{g}"))
                .unwrap_or_else(|| "NULL".into());
            let acc_join = match &self.col_account {
                Some(c) => format!("LEFT JOIN ZICCLOUDSYNCINGOBJECT acc ON acc.Z_PK = a.{c}"),
                None => "LEFT JOIN (SELECT NULL AS ZIDENTIFIER, NULL AS Z_PK) acc ON 0".into(),
            };
            let sql = format!(
                "SELECT a.ZIDENTIFIER AS ident, a.ZTYPEUTI AS uti, a.ZTITLE AS title, a.ZURLSTRING AS url,
                        a.{mg} AS mergeable, m.ZIDENTIFIER AS media_id, {gen_col} AS generation,
                        m.ZFILENAME AS filename, acc.ZIDENTIFIER AS account
                   FROM ZICCLOUDSYNCINGOBJECT a
                   LEFT JOIN ZICCLOUDSYNCINGOBJECT m ON m.Z_PK = a.ZMEDIA
                   {acc_join}
                  WHERE a.ZIDENTIFIER IN ({placeholders})",
                mg = self.col_mergeable,
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
            for id in chunk {
                q = q.bind(id);
            }
            for r in q.fetch_all(&self.pool).await? {
                let ident: String = r.get("ident");
                let file = match (
                    r.get::<Option<String>, _>("account"),
                    r.get::<Option<String>, _>("media_id"),
                    r.get::<Option<String>, _>("filename"),
                ) {
                    (account, Some(media), Some(name)) => {
                        let generation: Option<String> = r.get("generation");
                        self.media_file(account.as_deref(), &media, generation.as_deref(), &name)
                    }
                    _ => None,
                };
                out.insert(
                    ident.clone(),
                    RawAttachment {
                        identifier: ident,
                        uti: r.get::<Option<String>, _>("uti").unwrap_or_default(),
                        title: r
                            .get::<Option<String>, _>("title")
                            .filter(|t| !t.trim().is_empty()),
                        url: r
                            .get::<Option<String>, _>("url")
                            .filter(|t| !t.trim().is_empty()),
                        mergeable: r.get("mergeable"),
                        file,
                    },
                );
            }
        }
        Ok(out)
    }

    /// `Accounts/<account>/Media/<media id>/<generation>/<filename>`, with
    /// fallbacks for legacy layouts. Only returns paths that exist.
    fn media_file(
        &self,
        account: Option<&str>,
        media: &str,
        generation: Option<&str>,
        name: &str,
    ) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        let base = match account {
            Some(a) => self
                .container
                .join("Accounts")
                .join(a)
                .join("Media")
                .join(media),
            None => self.container.join("Media").join(media),
        };
        if let Some(g) = generation {
            candidates.push(base.join(g).join(name));
        }
        candidates.push(base.join(name));
        candidates.into_iter().find(|p| p.is_file())
    }
}
