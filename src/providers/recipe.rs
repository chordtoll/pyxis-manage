use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Recipe {
    pub version: String,
    pub depends: Vec<String>,
    pub actions: Option<String>,
    #[serde(default)]
    pub files:   BTreeMap<String, String>,
}
