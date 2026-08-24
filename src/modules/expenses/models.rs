use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Chf,
    Eur,
    Ron,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryGroup {
    Housing,
    Food,
    Transportation,
    Health,
    Insurance,
    Lifestyle,
    Financial,
    Education,
    Travel,
    Giving,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: u64,
    pub name: String,
    pub group: CategoryGroup,
}
