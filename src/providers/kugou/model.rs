use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KugouLyricResp {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub contenttype: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KugouLyricSearchResp {
    #[serde(default)]
    candidates: Vec<KugouLyricCandidate>,
    #[serde(default)]
    data: Option<KugouLyricSearchData>,
}

#[derive(Deserialize)]
struct KugouLyricSearchData {
    #[serde(default)]
    candidates: Vec<KugouLyricCandidate>,
}

impl KugouLyricSearchResp {
    pub(super) fn first_candidate(&self) -> Option<&KugouLyricCandidate> {
        self.candidates
            .iter()
            .chain(self.data.iter().flat_map(|d| d.candidates.iter()))
            .find(|c| !c.id.is_empty() && !c.access_key.is_empty())
    }
}

#[derive(Deserialize)]
pub(super) struct KugouLyricCandidate {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "accesskey")]
    pub access_key: String,
}
