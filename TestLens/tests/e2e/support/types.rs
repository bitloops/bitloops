use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct ListedArtefact {
    #[allow(dead_code)]
    pub artefact_id: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    #[allow(dead_code)]
    pub kind: String,
}
