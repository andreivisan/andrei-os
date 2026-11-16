//! Minimal protobuf wire-format reader for Apple's dynamic CRDT documents
//! (tables, audio transcripts), where structure is index-addressed and a
//! generated schema would not help much.

#[derive(Debug, Clone, Copy)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(&'a [u8]),
    Bytes(&'a [u8]),
    Fixed32(&'a [u8]),
}

#[derive(Debug, Clone, Default)]
pub struct Fields<'a>(pub Vec<(u32, Value<'a>)>);

fn varint(b: &[u8], i: &mut usize) -> anyhow::Result<u64> {
    let mut r: u64 = 0;
    let mut shift = 0;
    loop {
        let c = *b
            .get(*i)
            .ok_or_else(|| anyhow::anyhow!("truncated varint"))?;
        *i += 1;
        if shift >= 64 {
            anyhow::bail!("varint too long");
        }
        r |= u64::from(c & 0x7f) << shift;
        shift += 7;
        if c < 0x80 {
            return Ok(r);
        }
    }
}

fn take<'a>(b: &'a [u8], i: &mut usize, n: usize) -> anyhow::Result<&'a [u8]> {
    let end = i
        .checked_add(n)
        .filter(|&e| e <= b.len())
        .ok_or_else(|| anyhow::anyhow!("truncated field"))?;
    let s = &b[*i..end];
    *i = end;
    Ok(s)
}

pub fn parse(b: &[u8]) -> anyhow::Result<Fields<'_>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let field = (key >> 3) as u32;
        let v = match key & 7 {
            0 => Value::Varint(varint(b, &mut i)?),
            1 => Value::Fixed64(take(b, &mut i, 8)?),
            2 => {
                let len = varint(b, &mut i)? as usize;
                Value::Bytes(take(b, &mut i, len)?)
            }
            5 => Value::Fixed32(take(b, &mut i, 4)?),
            w => anyhow::bail!("unsupported wire type {w}"),
        };
        out.push((field, v));
    }
    Ok(Fields(out))
}

impl<'a> Fields<'a> {
    pub fn all(&self, field: u32) -> impl Iterator<Item = &Value<'a>> {
        self.0
            .iter()
            .filter(move |(f, _)| *f == field)
            .map(|(_, v)| v)
    }
    pub fn has(&self, field: u32) -> bool {
        self.0.iter().any(|(f, _)| *f == field)
    }
    pub fn bytes(&self, field: u32) -> Option<&'a [u8]> {
        self.all(field).find_map(|v| match v {
            Value::Bytes(b) => Some(*b),
            _ => None,
        })
    }
    pub fn all_bytes(&self, field: u32) -> Vec<&'a [u8]> {
        self.all(field)
            .filter_map(|v| match v {
                Value::Bytes(b) => Some(*b),
                _ => None,
            })
            .collect()
    }
    pub fn varint(&self, field: u32) -> Option<u64> {
        self.all(field).find_map(|v| match v {
            Value::Varint(n) => Some(*n),
            _ => None,
        })
    }
    pub fn double(&self, field: u32) -> Option<f64> {
        self.all(field).find_map(|v| match v {
            Value::Fixed64(b) => Some(f64::from_le_bytes((*b).try_into().ok()?)),
            _ => None,
        })
    }
    pub fn string(&self, field: u32) -> Option<String> {
        self.bytes(field)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }
    pub fn message(&self, field: u32) -> anyhow::Result<Option<Fields<'a>>> {
        self.bytes(field).map(parse).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_wire_types() {
        // field 1 varint 150, field 2 bytes "hi", field 3 fixed64 1.5
        let mut b = vec![0x08, 0x96, 0x01, 0x12, 0x02, b'h', b'i', 0x19];
        b.extend_from_slice(&1.5f64.to_le_bytes());
        let f = parse(&b).unwrap();
        assert_eq!(f.varint(1), Some(150));
        assert_eq!(f.string(2).as_deref(), Some("hi"));
        assert_eq!(f.double(3), Some(1.5));
        assert!(!f.has(4));
    }

    #[test]
    fn rejects_truncated() {
        assert!(parse(&[0x12, 0x05, b'a']).is_err());
        assert!(parse(&[0x08, 0xff]).is_err());
    }
}
