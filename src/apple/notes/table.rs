//! Native Apple Notes tables (`com.apple.notes.table`) -> Markdown pipe tables.

use super::crdt::{Doc, ObjectId};
use super::proto::gunzip_if_needed;

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
    pub right_to_left: bool,
}

/// Decode a table blob (`ZMERGEABLEDATA1`, gzipped or not).
pub fn decode(blob: &[u8]) -> anyhow::Result<Table> {
    let raw = gunzip_if_needed(blob)?;
    let doc = Doc::from_proto(&raw)?;
    let (_, root) = doc
        .find_custom_map_of_type("com.apple.notes.ICTable")?
        .ok_or_else(|| anyhow::anyhow!("no ICTable object"))?;
    let obj = |k: &str| -> anyhow::Result<usize> {
        root.map
            .get(k)
            .and_then(ObjectId::obj)
            .ok_or_else(|| anyhow::anyhow!("table without {k}"))
    };
    let rows = doc.ordered_set(obj("crRows")?)?;
    let cols = doc.ordered_set(obj("crColumns")?)?;
    let cell_columns = doc.dictionary(obj("cellColumns")?)?;

    let mut right_to_left = false;
    if let Ok(dir) = obj("crTableColumnDirection")
        && let Some(ObjectId::Str(s)) = doc.scalar(dir)?
    {
        right_to_left = s.contains("RightToLeft");
    }

    let mut grid = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut line = Vec::with_capacity(cols.len());
        for c in &cols {
            let text = match cell_columns.get(c) {
                Some(col_dict) => match doc.dictionary(*col_dict)?.get(r) {
                    Some(cell) => doc.note_text(*cell)?,
                    None => String::new(),
                },
                None => String::new(),
            };
            line.push(text);
        }
        if right_to_left {
            line.reverse(); // column 0 is drawn at the right edge
        }
        grid.push(line);
    }
    Ok(Table {
        rows: grid,
        right_to_left,
    })
}

fn md_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\r', "")
        .replace(['\n', '\u{2028}'], "<br>")
        .trim()
        .to_string()
}

/// Render with the first row as header (Apple tables have no header concept;
/// in practice the first row always holds the headings).
pub fn to_markdown(t: &Table) -> String {
    let Some(first) = t.rows.first().filter(|r| !r.is_empty()) else {
        return String::new();
    };
    let n = first.len();
    let line = |r: &Vec<String>| {
        format!(
            "| {} |",
            r.iter().map(|c| md_cell(c)).collect::<Vec<_>>().join(" | ")
        )
    };
    let mut out = vec![line(first), format!("|{}|", vec!["---"; n].join("|"))];
    out.extend(t.rows[1..].iter().map(line));
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_and_header() {
        let t = Table {
            rows: vec![
                vec!["A".into(), "B|C".into()],
                vec!["x\ny".into(), " \\ ".into()],
            ],
            right_to_left: false,
        };
        assert_eq!(
            to_markdown(&t),
            "| A | B\\|C |\n|---|---|\n| x<br>y | \\\\ |\n"
        );
    }

    #[test]
    fn empty_table() {
        assert_eq!(
            to_markdown(&Table {
                rows: vec![],
                right_to_left: false
            }),
            ""
        );
    }
}
