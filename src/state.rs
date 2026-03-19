use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

/// A single resource instance extracted from state.
#[derive(Debug, Clone)]
pub struct ResourceInstance {
    /// "managed" or "data"
    pub mode: String,
    pub resource_type: String,
    pub name: String,
    /// Module address prefix, e.g. "module.vpc" (None for root module).
    pub module_path: Option<String>,
    /// Index key for count/for_each, e.g. "0" or "\"prod\""
    pub index: Option<String>,
    /// Full provider source, e.g. "hashicorp/aws"
    pub provider_source: String,
    /// Local provider name, e.g. "aws"
    pub provider_local_name: String,
    /// Value of the `id` attribute (used in import blocks).
    pub resource_id: Option<String>,
    /// Top-level attribute keys that are marked sensitive in state.
    pub sensitive_top_keys: HashSet<String>,
    /// All attributes from state (id excluded for rendering, kept for id extraction).
    pub attributes: Value,
}

impl ResourceInstance {
    /// The Terraform address for this resource, e.g. `module.vpc.aws_instance.web[0]`
    pub fn address(&self) -> String {
        let base = format!("{}.{}", self.resource_type, self.name);
        let with_module = match &self.module_path {
            Some(m) => format!("{m}.{base}"),
            None => base,
        };
        match &self.index {
            Some(idx) => format!("{with_module}[{idx}]"),
            None => with_module,
        }
    }
}

// ── internal deserialization types ──────────────────────────────────────────

#[derive(Deserialize)]
struct StateFile {
    #[serde(default)]
    resources: Vec<StateResource>,
}

#[derive(Deserialize)]
struct StateResource {
    #[serde(default = "default_managed")]
    mode: String,
    #[serde(rename = "type")]
    resource_type: String,
    name: String,
    /// Present for resources inside a module, e.g. "module.vpc"
    module: Option<String>,
    provider: String,
    #[serde(default)]
    instances: Vec<StateInstance>,
}

fn default_managed() -> String {
    "managed".to_owned()
}

#[derive(Deserialize)]
struct StateInstance {
    index_key: Option<Value>,
    attributes: Option<Value>,
    /// Paths to sensitive values, e.g. `[["password"], ["config", 0, "secret"]]`
    #[serde(default)]
    sensitive_attributes: Vec<Value>,
}

// ── public API ───────────────────────────────────────────────────────────────

pub fn parse_state(state_json: &Value) -> Result<Vec<ResourceInstance>> {
    let state: StateFile =
        serde_json::from_value(state_json.clone()).context("Failed to deserialize state file")?;

    let mut out = Vec::new();
    for res in state.resources {
        let (provider_source, provider_local_name) = parse_provider(&res.provider);

        for inst in res.instances {
            let index = inst.index_key.as_ref().map(|k| match k {
                Value::String(s) => format!("\"{}\"", s),
                other => other.to_string(),
            });

            let sensitive_top_keys = extract_sensitive_top_keys(&inst.sensitive_attributes);
            let attrs = inst.attributes.unwrap_or(Value::Object(Default::default()));
            let resource_id = extract_id(&attrs);

            out.push(ResourceInstance {
                mode: res.mode.clone(),
                resource_type: res.resource_type.clone(),
                name: res.name.clone(),
                module_path: res.module.clone(),
                index,
                provider_source: provider_source.clone(),
                provider_local_name: provider_local_name.clone(),
                resource_id,
                sensitive_top_keys,
                attributes: attrs,
            });
        }
    }
    Ok(out)
}

/// Parse `provider["registry.terraform.io/hashicorp/aws"]`
/// or    `provider["registry.opentofu.org/hashicorp/aws"]`
/// → source = "hashicorp/aws", local_name = "aws"
fn parse_provider(raw: &str) -> (String, String) {
    // Extract the part inside the quotes
    let source = raw
        .trim_start_matches("provider[\"")
        .trim_end_matches("\"]");

    // Drop the registry hostname prefix so the source is the short
    // `namespace/type` form, which works for both Terraform and OpenTofu.
    let source = source
        .strip_prefix("registry.terraform.io/")
        .or_else(|| source.strip_prefix("registry.opentofu.org/"))
        .unwrap_or(source);

    let local_name = source.rsplit('/').next().unwrap_or(source).to_owned();
    (source.to_owned(), local_name)
}

fn extract_id(attrs: &Value) -> Option<String> {
    attrs
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Collect top-level sensitive field names from the sensitive_attributes array.
/// Each entry is an array-of-path-segments; we only grab the first segment when
/// it's a string (direct top-level key).
fn extract_sensitive_top_keys(sensitive_attributes: &[Value]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for path in sensitive_attributes {
        if let Value::Array(segments) = path {
            if let Some(Value::String(top_key)) = segments.first() {
                keys.insert(top_key.clone());
            }
        }
    }
    keys
}
