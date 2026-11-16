//! Apple Notes "mergeable data" CRDT documents: native tables and audio
//! transcripts. Layout (verified, see docs/apple-formats.md):
//!
//! ```text
//! MergeableDataObjectData { 3: entry[], 4: key_item[] (str), 5: type_item[] (str), 6: uuid_item[] (16 bytes) }
//! entry { 1: RegisterLatest{2: ObjectID}, 6: Dictionary, 10: Note, 13: CustomMap{1: type idx, 3: MapEntry{1: key idx, 2: ObjectID}}, 16: OrderedSet }
//! ObjectID { 2: uint, 3: double, 4: string, 6: object index }
//! ```
//! Tables wrap this in `{2: {3: data}}` and are gzipped; transcripts are the
//! bare object data.

use std::collections::HashMap;

use super::wire::{Fields, parse};

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectId {
    Uint(u64),
    Double(f64),
    Str(String),
    Obj(usize),
    None,
}

impl ObjectId {
    fn from_fields(f: &Fields<'_>) -> Self {
        // Precedence mirrors the reference decoder: object index first.
        if let Some(v) = f.varint(6) {
            return ObjectId::Obj(v as usize);
        }
        if let Some(s) = f.string(4) {
            return ObjectId::Str(s);
        }
        if let Some(d) = f.double(3) {
            return ObjectId::Double(d);
        }
        if let Some(v) = f.varint(2) {
            return ObjectId::Uint(v);
        }
        ObjectId::None
    }
    pub fn obj(&self) -> Option<usize> {
        match self {
            ObjectId::Obj(i) => Some(*i),
            _ => None,
        }
    }
}

pub struct Doc<'a> {
    entries: Vec<&'a [u8]>,
    keys: Vec<String>,
    types: Vec<String>,
    uuids: Vec<&'a [u8]>,
}

pub struct CustomMap {
    pub type_name: String,
    pub map: HashMap<String, ObjectId>,
}

impl<'a> Doc<'a> {
    /// From bare MergeableDataObjectData bytes (audio transcripts).
    pub fn from_object_data(b: &'a [u8]) -> anyhow::Result<Self> {
        let data = parse(b)?;
        Ok(Self {
            entries: data.all_bytes(3),
            keys: data
                .all_bytes(4)
                .into_iter()
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect(),
            types: data
                .all_bytes(5)
                .into_iter()
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect(),
            uuids: data.all_bytes(6),
        })
    }

    /// From a MergableDataProto (gunzipped table blob): `{2: {3: data}}`.
    pub fn from_proto(b: &'a [u8]) -> anyhow::Result<Self> {
        let top = parse(b)?;
        let obj = top
            .bytes(2)
            .ok_or_else(|| anyhow::anyhow!("no mergable_data_object"))?;
        let obj = parse(obj)?;
        let data = obj
            .bytes(3)
            .ok_or_else(|| anyhow::anyhow!("no object data"))?;
        Self::from_object_data(data)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry(&self, i: usize) -> anyhow::Result<Fields<'a>> {
        let b = self
            .entries
            .get(i)
            .ok_or_else(|| anyhow::anyhow!("entry {i} out of range"))?;
        parse(b)
    }

    pub fn custom_map(&self, i: usize) -> anyhow::Result<Option<CustomMap>> {
        let e = self.entry(i)?;
        let Some(cm) = e.message(13)? else {
            return Ok(None);
        };
        let ty = cm
            .varint(1)
            .ok_or_else(|| anyhow::anyhow!("custom map without type"))? as usize;
        let type_name = self.types.get(ty).cloned().unwrap_or_default();
        let mut map = HashMap::new();
        for me in cm.all_bytes(3) {
            let me = parse(me)?;
            let Some(k) = me.varint(1).and_then(|k| self.keys.get(k as usize)) else {
                continue;
            };
            let v = me
                .message(2)?
                .map(|f| ObjectId::from_fields(&f))
                .unwrap_or(ObjectId::None);
            map.insert(k.clone(), v);
        }
        Ok(Some(CustomMap { type_name, map }))
    }

    pub fn find_custom_map_of_type(
        &self,
        type_name: &str,
    ) -> anyhow::Result<Option<(usize, CustomMap)>> {
        for i in 0..self.len() {
            if let Some(cm) = self.custom_map(i)?
                && cm.type_name == type_name
            {
                return Ok(Some((i, cm)));
            }
        }
        Ok(None)
    }

    fn uuid_of(&self, i: usize) -> anyhow::Result<Option<&'a [u8]>> {
        let Some(cm) = self.custom_map(i)? else {
            return Ok(None);
        };
        Ok(match cm.map.get("UUIDIndex") {
            Some(ObjectId::Uint(idx)) => self.uuids.get(*idx as usize).copied(),
            _ => None,
        })
    }

    fn dict_pairs(d: &Fields<'_>) -> anyhow::Result<Vec<(ObjectId, ObjectId)>> {
        let mut out = Vec::new();
        for el in d.all_bytes(1) {
            let el = parse(el)?;
            let k = el
                .message(1)?
                .map(|f| ObjectId::from_fields(&f))
                .unwrap_or(ObjectId::None);
            let v = el
                .message(2)?
                .map(|f| ObjectId::from_fields(&f))
                .unwrap_or(ObjectId::None);
            out.push((k, v));
        }
        Ok(out)
    }

    /// Dictionary entry as object-index -> object-index.
    pub fn dictionary(&self, i: usize) -> anyhow::Result<HashMap<usize, usize>> {
        let e = self.entry(i)?;
        let Some(d) = e.message(6)? else {
            return Ok(HashMap::new());
        };
        Ok(Self::dict_pairs(&d)?
            .into_iter()
            .filter_map(|(k, v)| Some((k.obj()?, v.obj()?)))
            .collect())
    }

    /// Ordered set entry -> element object indices in display order.
    ///
    /// The ordering array's text is N x U+FFFC; the attachment whose `index`
    /// equals a placeholder's char position carries the UUID of that slot. The
    /// ordering dictionary maps UUID-objects to element objects.
    pub fn ordered_set(&self, i: usize) -> anyhow::Result<Vec<usize>> {
        let e = self.entry(i)?;
        let os = e
            .message(16)?
            .ok_or_else(|| anyhow::anyhow!("entry {i} is not an ordered set"))?;
        let ordering = os
            .message(1)?
            .ok_or_else(|| anyhow::anyhow!("ordered set without ordering"))?;
        let array = ordering
            .message(1)?
            .ok_or_else(|| anyhow::anyhow!("ordering without array"))?;
        let text = array
            .message(1)?
            .and_then(|n| n.string(2))
            .unwrap_or_default();
        let mut att: HashMap<u64, &[u8]> = HashMap::new();
        for a in array.all_bytes(2) {
            let a = parse(a)?;
            if let (Some(idx), Some(uuid)) = (a.varint(1), a.bytes(2)) {
                att.insert(idx, uuid);
            }
        }
        let contents = match ordering.message(2)? {
            Some(d) => Self::dict_pairs(&d)?,
            None => vec![],
        };
        let mut by_uuid: HashMap<&[u8], usize> = HashMap::new();
        for (k, v) in &contents {
            if let (Some(k), Some(v)) = (k.obj(), v.obj())
                && let Some(u) = self.uuid_of(k)?
            {
                by_uuid.insert(u, v);
            }
        }
        // Positions are counted in characters of the ordering text. Every
        // character there is U+FFFC (one UTF-16 unit, one scalar), so scalar
        // and UTF-16 positions coincide.
        let mut order = Vec::new();
        for (p, ch) in text.chars().enumerate() {
            if ch != '\u{FFFC}' {
                continue;
            }
            if let Some(u) = att.get(&(p as u64))
                && let Some(v) = by_uuid.get(u)
            {
                order.push(*v);
            }
        }
        Ok(order)
    }

    /// Text of a Note entry (table cell).
    pub fn note_text(&self, i: usize) -> anyhow::Result<String> {
        let e = self.entry(i)?;
        Ok(e.message(10)?.and_then(|n| n.string(2)).unwrap_or_default())
    }

    pub fn register_value(&self, i: usize) -> anyhow::Result<Option<ObjectId>> {
        let e = self.entry(i)?;
        let Some(reg) = e.message(1)? else {
            return Ok(None);
        };
        Ok(reg.message(2)?.map(|f| ObjectId::from_fields(&f)))
    }

    /// Resolve an object to a scalar: follows registers, unwraps NSString /
    /// NSNumber custom maps ("self", "doubleValue", "integerValue") and Notes.
    pub fn scalar(&self, i: usize) -> anyhow::Result<Option<ObjectId>> {
        let mut cur = i;
        for _ in 0..8 {
            if let Some(v) = self.register_value(cur)? {
                match v {
                    ObjectId::Obj(next) => {
                        cur = next;
                        continue;
                    }
                    other => return Ok(Some(other)),
                }
            }
            if let Some(cm) = self.custom_map(cur)? {
                for k in ["self", "doubleValue", "integerValue"] {
                    if let Some(v) = cm.map.get(k) {
                        return Ok(Some(v.clone()));
                    }
                }
                return Ok(None);
            }
            let e = self.entry(cur)?;
            if e.has(10) {
                return Ok(Some(ObjectId::Str(self.note_text(cur)?)));
            }
            return Ok(None);
        }
        Ok(None)
    }
}
