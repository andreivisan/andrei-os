//! Small string helpers shared by sync and web.

/// URL slug: lowercase ASCII alphanumerics separated by single dashes.
/// Non-ASCII letters are transliterated for common Latin diacritics (Romanian
/// included) and dropped otherwise.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.chars().flat_map(fold_diacritic) {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    out
}

/// Key used to match a `[[Link]]` to a note title, or a `#Tag` to a project:
/// case-insensitive, diacritics folded, and all whitespace/punctuation removed
/// so `[[perception device]]`, `#PerceptionDevice` and `#perception-device`
/// all match "Perception Device".
pub fn match_key(s: &str) -> String {
    s.chars()
        .flat_map(fold_diacritic)
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn fold_diacritic(c: char) -> impl Iterator<Item = char> {
    let mapped: &str = match c {
        'ă' | 'â' | 'á' | 'à' | 'ä' | 'ã' | 'å' => "a",
        'Ă' | 'Â' | 'Á' | 'À' | 'Ä' | 'Ã' | 'Å' => "A",
        'î' | 'í' | 'ì' | 'ï' => "i",
        'Î' | 'Í' | 'Ì' | 'Ï' => "I",
        'ș' | 'ş' | 'š' => "s",
        'Ș' | 'Ş' | 'Š' => "S",
        'ț' | 'ţ' => "t",
        'Ț' | 'Ţ' => "T",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'É' | 'È' | 'Ê' | 'Ë' => "E",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
        'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => "O",
        'ú' | 'ù' | 'û' | 'ü' => "u",
        'Ú' | 'Ù' | 'Û' | 'Ü' => "U",
        'ç' => "c",
        'Ç' => "C",
        'ñ' => "n",
        'Ñ' => "N",
        'ß' => "ss",
        _ => "",
    };
    let single = if mapped.is_empty() { Some(c) } else { None };
    single.into_iter().chain(mapped.chars())
}

/// Stable, dependency-free content hash (FNV-1a 64) as hex. Only used to detect
/// changes, never for security.
pub fn hash_hex(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            // separator so ("ab","c") != ("a","bc")
            h ^= 0x1f;
            h = h.wrapping_mul(0x100000001b3);
        }
        for b in p.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

/// Shorten to at most `max` chars on a word boundary, adding an ellipsis.
pub fn truncate_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => &cut[..i],
        _ => &cut[..],
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Ohm's law"), "ohm-s-law");
        assert_eq!(
            slugify("  The Future trait & the poll contract "),
            "the-future-trait-the-poll-contract"
        );
        assert_eq!(slugify("Învățare: Rust"), "invatare-rust");
        assert_eq!(slugify("C++"), "c");
        assert_eq!(slugify("🦀"), "");
    }

    #[test]
    fn match_keys() {
        assert_eq!(match_key("Perception Device"), "perceptiondevice");
        assert_eq!(match_key("#perception-device"), "perceptiondevice");
        assert_eq!(match_key("HudAgents"), "hudagents");
        assert_eq!(match_key("Știință"), "stiinta");
    }

    #[test]
    fn hashing_is_stable_and_separated() {
        assert_eq!(hash_hex(&["a"]), hash_hex(&["a"]));
        assert_ne!(hash_hex(&["ab", "c"]), hash_hex(&["a", "bc"]));
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate_words("short", 10), "short");
        assert_eq!(
            truncate_words("hello wonderful world", 17),
            "hello wonderful…"
        );
        assert_eq!(truncate_words("hello wonderful world", 12), "hello wonder…");
    }
}
