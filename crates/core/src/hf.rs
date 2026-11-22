//! Hugging Face lookups for the `hf-model` input type: model search and GGUF
//! file listing (host side, through an injected [`HttpClient`]).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind, Result};
use crate::http::{HttpClient, encode_query};
use crate::plugins::is_repo_id;
use crate::quant::{GgufFile, GgufListing, group};

pub const HF_BASE: &str = "https://huggingface.co";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfModel {
    /// Repo id (`owner/name`).
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    /// Gated (needs an accepted licence and a token).
    pub gated: bool,
}

#[derive(Clone)]
pub struct HfClient {
    http: Arc<dyn HttpClient>,
    base: String,
    token: Option<String>,
}

impl std::fmt::Debug for HfClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HfClient")
            .field("base", &self.base)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

pub fn search_url(base: &str, query: &str, limit: u32) -> String {
    format!(
        "{base}/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit={}",
        encode_query(query.trim()),
        limit.clamp(1, 100)
    )
}

/// Publishers whose GGUF files are made by the llama.cpp project itself, so they load in the
/// llama.cpp builds the app runs. Other publishers' files sometimes target other software.
pub const OFFICIAL_OWNERS: &[&str] = &["ggml-org"];

/// Official repositories put first in a search, at most.
pub const OFFICIAL_RESULTS: u32 = 3;

pub fn is_official(repo: &str) -> bool {
    repo.split('/')
        .next()
        .is_some_and(|owner| OFFICIAL_OWNERS.contains(&owner))
}

/// A search among `owner`'s GGUF repositories. A query written as `owner/name` searches for
/// `name`, so pasting another publisher's repository still finds the official one.
pub fn official_search_url(base: &str, owner: &str, query: &str, limit: u32) -> String {
    let name = query.trim().rsplit('/').next().unwrap_or_default();
    format!(
        "{base}/api/models?author={owner}&search={}&filter=gguf&sort=downloads&direction=-1&limit={}",
        encode_query(name),
        limit.clamp(1, 100)
    )
}

/// `official` first, then the rest of `general` without repeating any, at most `limit`.
pub fn official_first(official: Vec<HfModel>, general: Vec<HfModel>, limit: u32) -> Vec<HfModel> {
    let mut out: Vec<HfModel> = Vec::new();
    let (first, rest): (Vec<_>, Vec<_>) = general.into_iter().partition(|m| is_official(&m.id));
    for model in official.into_iter().chain(first).chain(rest) {
        if !out.iter().any(|seen| seen.id == model.id) {
            out.push(model);
        }
    }
    out.truncate(limit as usize);
    out
}

pub fn tree_url(base: &str, repo: &str) -> String {
    format!("{base}/api/models/{repo}/tree/main?recursive=true")
}

pub fn parse_search(body: &[u8]) -> Result<Vec<HfModel>> {
    let v: Vec<serde_json::Value> = serde_json::from_slice(body)?;
    Ok(v.iter()
        .filter_map(|m| {
            let id = m
                .get("id")
                .or_else(|| m.get("modelId"))?
                .as_str()?
                .to_owned();
            Some(HfModel {
                id,
                downloads: m.get("downloads").and_then(|d| d.as_u64()).unwrap_or(0),
                likes: m.get("likes").and_then(|d| d.as_u64()).unwrap_or(0),
                gated: m
                    .get("gated")
                    .is_some_and(|g| g.as_bool().unwrap_or(g.is_string())),
            })
        })
        .collect())
}

/// LFS size preferred.
pub fn parse_tree(body: &[u8]) -> Result<Vec<GgufFile>> {
    let v: Vec<serde_json::Value> = serde_json::from_slice(body)?;
    Ok(v.iter()
        .filter(|f| f.get("type").and_then(|t| t.as_str()) == Some("file"))
        .filter_map(|f| {
            let path = f.get("path")?.as_str()?.to_owned();
            if !path.to_ascii_lowercase().ends_with(".gguf") {
                return None;
            }
            let size = f
                .get("lfs")
                .and_then(|l| l.get("size"))
                .and_then(|s| s.as_u64())
                .or_else(|| f.get("size").and_then(|s| s.as_u64()))
                .unwrap_or(0);
            Some(GgufFile { path, size })
        })
        .collect())
}

impl HfClient {
    pub fn new(http: Arc<dyn HttpClient>, token: Option<String>) -> Self {
        HfClient {
            http,
            base: HF_BASE.to_owned(),
            token,
        }
    }

    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers = vec![("Accept", "application/json")];
        if let Some(a) = &auth {
            headers.push(("Authorization", a.as_str()));
        }
        let resp = self.http.get(url, &headers)?;
        if resp.status != 200 {
            return Err(Error::new(ErrorKind::Http)
                .with("status", resp.status)
                .with("url", url));
        }
        Ok(resp.body)
    }

    /// Searches GGUF models: official ones ([`OFFICIAL_OWNERS`]) first, then the rest, most
    /// downloaded first. The official ones are looked up on their own, since a popular model
    /// has more repackagings than fit in `limit`; when that lookup fails the rest still shows.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<HfModel>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let general = parse_search(&self.get(&search_url(&self.base, query, limit))?)?;
        let official = OFFICIAL_OWNERS
            .iter()
            .flat_map(|owner| {
                let url = official_search_url(&self.base, owner, query, OFFICIAL_RESULTS);
                self.get(&url)
                    .and_then(|body| parse_search(&body))
                    .unwrap_or_default()
            })
            .filter(|model| is_official(&model.id))
            .collect();
        Ok(official_first(official, general, limit))
    }

    pub fn gguf_files(&self, repo: &str) -> Result<GgufListing> {
        if !is_repo_id(repo) {
            return Err(Error::invalid("repo", "expected owner/name").with("value", repo));
        }
        Ok(group(&parse_tree(&self.get(&tree_url(&self.base, repo))?)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpResponse;
    use std::sync::Mutex;

    type Seen = Mutex<Vec<(String, Vec<(String, String)>)>>;
    struct Canned(Seen, u16, &'static str);

    impl HttpClient for Canned {
        fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
            self.0.lock().unwrap().push((
                url.into(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            Ok(HttpResponse {
                status: self.1,
                body: self.2.as_bytes().to_vec(),
            })
        }
    }

    #[test]
    fn search_and_tree() {
        let http = Arc::new(Canned(
            Mutex::new(vec![]),
            200,
            r#"[{"id":"unsloth/Qwen3-0.6B-GGUF","downloads":10,"likes":2,"gated":false},{"modelId":"x/y","gated":"auto"}]"#,
        ));
        let c = HfClient::new(http.clone(), Some("hf_x".into()));
        let r = c.search("qwen 3", 5).unwrap();
        assert_eq!(r[0].id, "unsloth/Qwen3-0.6B-GGUF");
        assert!(r[1].gated);
        let (url, headers) = http.0.lock().unwrap()[0].clone();
        assert!(
            url.contains("search=qwen%203")
                && url.contains("filter=gguf")
                && url.contains("limit=5")
        );
        assert!(headers.contains(&("Authorization".into(), "Bearer hf_x".into())));
        assert!(!format!("{c:?}").contains("hf_x"));

        let tree = br#"[{"type":"directory","path":"Q8_0"},
            {"type":"file","path":"Q8_0/M-Q8_0-00001-of-00002.gguf","size":135,"lfs":{"size":2000}},
            {"type":"file","path":"Q8_0/M-Q8_0-00002-of-00002.gguf","size":135,"lfs":{"size":1000}},
            {"type":"file","path":"README.md","size":5}]"#;
        let files = parse_tree(tree).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(group(&files).variants[0].size, 3000);
        assert!(c.gguf_files("not valid").is_err());
    }

    fn model(id: &str) -> HfModel {
        HfModel {
            id: id.into(),
            downloads: 0,
            likes: 0,
            gated: false,
        }
    }

    #[test]
    fn official_repositories_come_first_once() {
        let ids = |models: Vec<HfModel>| models.into_iter().map(|m| m.id).collect::<Vec<_>>();
        let general = vec![
            model("unsloth/M-GGUF"),
            model("ggml-org/M-GGUF"),
            model("a/M-GGUF"),
            model("b/M-GGUF"),
        ];
        let official = vec![model("ggml-org/M-GGUF"), model("ggml-org/M-Small-GGUF")];
        assert_eq!(
            ids(official_first(official, general.clone(), 4)),
            [
                "ggml-org/M-GGUF",
                "ggml-org/M-Small-GGUF",
                "unsloth/M-GGUF",
                "a/M-GGUF"
            ]
        );
        assert_eq!(
            ids(official_first(Vec::new(), general, 8))[..2],
            ["ggml-org/M-GGUF", "unsloth/M-GGUF"],
            "an official one in the general results moves up too"
        );
        assert!(
            is_official("ggml-org/x") && !is_official("ggml-orgx/y") && !is_official("x/ggml-org")
        );
        let url = official_search_url(HF_BASE, "ggml-org", " unsloth/Qwen3 8B ", 3);
        assert!(
            url.contains("author=ggml-org")
                && url.contains("search=Qwen3%208B")
                && url.contains("limit=3")
        );
    }

    #[test]
    fn http_errors_carry_status() {
        let c = HfClient::new(Arc::new(Canned(Mutex::new(vec![]), 401, "")), None);
        let e = c.gguf_files("org/private").unwrap_err();
        assert!(e.is(ErrorKind::Http));
        assert_eq!(e.param("status"), Some("401"));
    }
}
