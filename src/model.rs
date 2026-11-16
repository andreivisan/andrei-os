//! Shared domain vocabulary.

use crate::text::match_key;

/// Top-level areas of the second brain = first folder level under the
/// AndreiOS folder in Apple Notes.
pub const AREAS: &[(&str, &str)] = &[
    ("learning", "Learning"),
    ("projects", "Projects"),
    ("health", "Health"),
    ("interests", "Interests"),
    ("resources", "Resources"),
    ("admin", "Admin"),
];

pub fn area_from_folder(rel_folder: &str) -> &'static str {
    let first = rel_folder.split('/').next().unwrap_or("");
    let key = match_key(first);
    AREAS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(k, _)| *k)
        .unwrap_or("inbox")
}

pub fn area_label(area: &str) -> &'static str {
    AREAS
        .iter()
        .find(|(k, _)| *k == area)
        .map(|(_, l)| *l)
        .unwrap_or("Inbox")
}

/// Area for a Reminders tag such as "#Admin", if it names an area.
pub fn area_from_tag(tag: &str) -> Option<&'static str> {
    let key = match_key(tag);
    AREAS.iter().find(|(k, _)| *k == key).map(|(k, _)| *k)
}

pub const STATUSES: &[(&str, &str)] = &[("todo", "To do"), ("doing", "Doing"), ("done", "Done")];

pub fn valid_status(s: &str) -> bool {
    STATUSES.iter().any(|(k, _)| *k == s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas() {
        assert_eq!(area_from_folder("Learning/Rust"), "learning");
        assert_eq!(area_from_folder("Projects/Hudward/HudAgents"), "projects");
        assert_eq!(area_from_folder(""), "inbox");
        assert_eq!(area_from_tag("#admin"), Some("admin"));
        assert_eq!(area_from_tag("PLWebsite"), None);
    }
}
