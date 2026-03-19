use anyhow::{bail, Context, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;

pub struct TfcClient {
    http: reqwest::Client,
    token: String,
    base_url: String,
}

#[derive(Deserialize)]
struct WorkspaceResponse {
    data: WorkspaceData,
}

#[derive(Deserialize)]
struct WorkspaceData {
    id: String,
}

#[derive(Deserialize)]
struct CurrentStateVersionResponse {
    data: StateVersionData,
}

#[derive(Deserialize)]
struct StateVersionData {
    attributes: StateVersionAttributes,
}

#[derive(Deserialize)]
struct StateVersionAttributes {
    #[serde(rename = "hosted-state-download-url")]
    hosted_state_download_url: String,
}

impl TfcClient {
    pub fn new(token: &str, base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("terraform-settings-mapper/0.1")
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            http,
            token: token.to_owned(),
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Fetch the current state JSON for a given org/workspace.
    pub async fn fetch_state(&self, org: &str, workspace: &str) -> Result<serde_json::Value> {
        // Step 1: resolve workspace ID
        let ws_url = format!(
            "{}/api/v2/organizations/{}/workspaces/{}",
            self.base_url, org, workspace
        );
        let ws_resp: WorkspaceResponse = self
            .get_json(&ws_url)
            .await
            .context("Failed to fetch workspace")?;
        let workspace_id = ws_resp.data.id;

        // Step 2: get current state version
        let sv_url = format!(
            "{}/api/v2/workspaces/{}/current-state-version",
            self.base_url, workspace_id
        );
        let sv_resp: CurrentStateVersionResponse = self
            .get_json(&sv_url)
            .await
            .context("Failed to fetch current state version")?;
        let download_url = sv_resp.data.attributes.hosted_state_download_url;

        // Step 3: download raw state JSON
        let state_resp = self
            .http
            .get(&download_url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/vnd.api+json")
            .send()
            .await
            .context("Failed to download state file")?;

        if !state_resp.status().is_success() {
            bail!(
                "State download failed with status {}",
                state_resp.status()
            );
        }

        state_resp
            .json::<serde_json::Value>()
            .await
            .context("Failed to parse state JSON")
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/vnd.api+json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        if !resp.status().is_success() {
            bail!("HTTP {} for {url}", resp.status());
        }

        resp.json::<T>()
            .await
            .with_context(|| format!("Parsing JSON from {url}"))
    }
}
