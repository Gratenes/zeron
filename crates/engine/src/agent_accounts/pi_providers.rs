//! Pi's model providers: the `providers` map in `{pi_agent_dir}/models.json`.
//!
//! Pi reads custom (usually OpenAI-compatible) endpoints from that file —
//! `baseUrl`, `api`, `apiKey`, `models: [{id, name?}]` — and its model list
//! (and so zeron's Pi model catalog, which hashes the file) follows every
//! edit. Zeron edits one provider at a time and keeps everything else in
//! the file, including fields it doesn't model; the file is the only source
//! of truth (no slots, no snapshot).
//!
//! The URL here is one the user typed, so — unlike the credential-defined
//! endpoints [`super::stores::trusted_base`] guards — any host is allowed,
//! `http` included (LAN and local servers). A stored key is only ever sent
//! to its own provider's stored `baseUrl`, and never returned to a client.
//!
//! Pi never writes or locks `models.json`; [`StoreLock::File`] only
//! serialises zeron's own writers.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value, json};
use zeron_proto::{PiKeySource, PiProvider, PiProviderModel, SavePiProvider};

use super::AgentAccounts;
use super::stores::{StoreLock, merge_json};
use crate::EngineError;

/// The wire protocols the add/edit form offers. An existing entry may keep
/// any other value Pi understands.
const APIS: &[&str] = &[
    "openai-completions",
    "openai-responses",
    "anthropic-messages",
    "google-generative-ai",
];

/// Pi's built-in providers (pi-ai's `KnownProvider`, plus its OAuth-only
/// ones). An entry with one of these names *overrides* the built-in rather
/// than adding a provider, so new entries may not use them.
const BUILT_IN: &[&str] = &[
    "amazon-bedrock",
    "ant-ling",
    "anthropic",
    "azure-openai-responses",
    "baseten",
    "cerebras",
    "cloudflare-ai-gateway",
    "cloudflare-workers-ai",
    "deepseek",
    "fireworks",
    "github-copilot",
    "google",
    "google-antigravity",
    "google-gemini-cli",
    "google-vertex",
    "groq",
    "huggingface",
    "kimi-coding",
    "minimax",
    "minimax-cn",
    "mistral",
    "moonshotai",
    "moonshotai-cn",
    "nvidia",
    "openai",
    "openai-codex",
    "opencode",
    "opencode-go",
    "openrouter",
    "qwen-token-plan",
    "qwen-token-plan-cn",
    "qwen-token-plan-individual",
    "radius",
    "together",
    "vercel-ai-gateway",
    "xai",
    "xiaomi",
    "xiaomi-token-plan-ams",
    "xiaomi-token-plan-cn",
    "xiaomi-token-plan-sgp",
    "zai",
    "zai-coding-cn",
];

/// Most models one provider may list. Pi's catalog probe visits each model,
/// so a router's full list (hundreds) makes the model picker crawl.
const MAX_MODELS: usize = 500;
/// Most ids model discovery returns.
const MAX_DISCOVERED: usize = 5000;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

impl AgentAccounts {
    fn pi_models_file(&self) -> PathBuf {
        self.inner.config.pi_agent_dir.join("models.json")
    }

    fn pi_models_lock(&self) -> StoreLock {
        StoreLock::File(self.inner.config.pi_agent_dir.join("models.json.zeron-lock"))
    }

    /// Every provider in Pi's `models.json`, by name. A missing file is an
    /// empty list; one that doesn't parse is an error (and is never
    /// rewritten).
    pub fn list_pi_providers(&self) -> Result<Vec<PiProvider>, EngineError> {
        let providers = self.read_pi_providers()?;
        Ok(providers
            .iter()
            .filter_map(|(name, entry)| provider_view(name, entry))
            .collect())
    }

    /// Create, edit or rename one provider; returns the new list.
    pub fn save_pi_provider(&self, input: SavePiProvider) -> Result<Vec<PiProvider>, EngineError> {
        let name = input.name.trim().to_string();
        let previous = input
            .previous_name
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let base_url = normalize_base_url(&input.base_url)?;
        let api = input.api.trim().to_string();
        let models = normalize_models(&input.models)?;
        let api_key = match input.api_key.as_deref().map(str::trim) {
            Some("") => return Err(invalid("The API key is empty.")),
            Some(key) if key.chars().any(char::is_whitespace) => {
                return Err(invalid("The API key contains spaces."));
            }
            other => other.map(str::to_string),
        };
        merge_json(&self.pi_models_file(), self.pi_models_lock(), |root| {
            let providers = providers_mut(root)?;
            let existing = previous.and_then(|p| providers.get(p)).cloned();
            if previous.is_some() && existing.is_none() {
                return Err(invalid(&format!(
                    "Provider \"{}\" no longer exists — it may have been removed.",
                    previous.unwrap_or_default()
                )));
            }
            if previous != Some(name.as_str()) {
                validate_new_name(&name)?;
                if providers.contains_key(&name) {
                    return Err(invalid(&format!("A provider named \"{name}\" already exists.")));
                }
            }
            let kept_api = existing
                .as_ref()
                .and_then(|e| e.get("api"))
                .and_then(Value::as_str);
            if !APIS.contains(&api.as_str()) && kept_api != Some(api.as_str()) {
                return Err(invalid(&format!("Unsupported API type \"{api}\".")));
            }
            let mut entry = match existing {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            };
            match &api_key {
                Some(key) => {
                    entry.insert("apiKey".into(), json!(key));
                }
                None if !entry.contains_key("apiKey") => {
                    return Err(invalid("Enter an API key."));
                }
                None => {}
            }
            entry.insert("baseUrl".into(), json!(base_url));
            entry.insert("api".into(), json!(api));
            let old_models = entry.remove("models");
            entry.insert("models".into(), merge_models(old_models.as_ref(), &models));
            if let Some(previous) = previous {
                providers.remove(previous);
            }
            providers.insert(name.clone(), Value::Object(entry));
            Ok(true)
        })?;
        self.list_pi_providers()
    }

    /// Remove one provider; returns the new list. Already gone is fine.
    pub fn remove_pi_provider(&self, name: &str) -> Result<Vec<PiProvider>, EngineError> {
        merge_json(&self.pi_models_file(), self.pi_models_lock(), |root| {
            Ok(match root.get_mut("providers") {
                Some(Value::Object(providers)) => providers.remove(name).is_some(),
                _ => false,
            })
        })?;
        self.list_pi_providers()
    }

    /// The model ids an OpenAI-compatible endpoint lists at
    /// `GET {base_url}/models`, sorted. Authenticates with `api_key`, or —
    /// so a client never needs the secret back — with the stored literal
    /// key of provider `name`, which is only sent to that provider's own
    /// stored `baseUrl`.
    pub async fn discover_pi_models(
        &self,
        base_url: &str,
        api_key: Option<&str>,
        name: Option<&str>,
    ) -> Result<Vec<String>, EngineError> {
        let base_url = normalize_base_url(base_url)?;
        let key = match api_key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(key) => key.to_string(),
            None => self.stored_key_for(name, &base_url)?,
        };
        let url = format!("{base_url}/models");
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(&key)
            .timeout(DISCOVERY_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                EngineError::Other(format!("Couldn't reach {url}: {}", without_url(&e)))
            })?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(EngineError::Other(format!(
                "The endpoint rejected the API key (HTTP {}).",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(EngineError::Other(format!(
                "{url} answered HTTP {} — enter the models by hand.",
                status.as_u16()
            )));
        }
        let body: Value = response.json().await.map_err(|_| {
            EngineError::Other(format!(
                "{url} didn't return an OpenAI-style model list — enter the models by hand."
            ))
        })?;
        let listed = body
            .get("data")
            .or_else(|| body.get("models"))
            .unwrap_or(&body)
            .as_array()
            .ok_or_else(|| {
                EngineError::Other(format!(
                    "{url} didn't return an OpenAI-style model list — enter the models by hand."
                ))
            })?;
        let mut ids: Vec<String> = listed
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str).or_else(|| m.as_str()))
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        ids.sort();
        ids.dedup();
        ids.truncate(MAX_DISCOVERED);
        Ok(ids)
    }

    fn read_pi_providers(&self) -> Result<Map<String, Value>, EngineError> {
        let file = self.pi_models_file();
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(err) => return Err(err.into()),
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(mut root)) => match root.remove("providers") {
                Some(Value::Object(providers)) => Ok(providers),
                None => Ok(Map::new()),
                Some(_) => Err(EngineError::Other(format!(
                    "\"providers\" in {} isn't an object — fix or move the file first.",
                    file.display()
                ))),
            },
            _ => Err(EngineError::Other(format!(
                "{} isn't valid JSON — fix or move it first.",
                file.display()
            ))),
        }
    }

    fn stored_key_for(&self, name: Option<&str>, base_url: &str) -> Result<String, EngineError> {
        let missing = || invalid("Enter the API key to fetch models.");
        let providers = self.read_pi_providers()?;
        let entry = name.and_then(|n| providers.get(n)).ok_or_else(missing)?;
        let stored_url = entry
            .get("baseUrl")
            .and_then(Value::as_str)
            .and_then(|u| normalize_base_url(u).ok());
        if stored_url.as_deref() != Some(base_url) {
            return Err(invalid("Enter the API key to fetch models from a new URL."));
        }
        match entry.get("apiKey").and_then(Value::as_str) {
            Some(key) if key_source(key) == PiKeySource::Literal => Ok(key.to_string()),
            _ => Err(missing()),
        }
    }
}

fn invalid(message: &str) -> EngineError {
    EngineError::Other(message.to_string())
}

/// `root.providers`, created if missing; refuses a non-object rather than
/// replacing it.
fn providers_mut(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, EngineError> {
    root.entry("providers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("\"providers\" in models.json isn't an object — fix or move the file first."))
}

fn provider_view(name: &str, entry: &Value) -> Option<PiProvider> {
    let entry = entry.as_object()?;
    let str_of = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default();
    let key = entry.get("apiKey").and_then(Value::as_str);
    let key_source = key.map_or(PiKeySource::None, key_source);
    let key_tail = key
        .filter(|k| key_source == PiKeySource::Literal && k.chars().count() >= 8)
        .map(|k| {
            let chars: Vec<char> = k.chars().collect();
            chars[chars.len() - 4..].iter().collect()
        });
    let models = entry
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(PiProviderModel {
                id: m.get("id")?.as_str()?.to_string(),
                name: m.get("name").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect();
    Some(PiProvider {
        name: name.to_string(),
        base_url: str_of("baseUrl").to_string(),
        api: str_of("api").to_string(),
        key_source,
        key_tail,
        models,
    })
}

fn key_source(key: &str) -> PiKeySource {
    match key.trim().chars().next() {
        Some('!') => PiKeySource::Command,
        Some('$') => PiKeySource::Env,
        _ => PiKeySource::Literal,
    }
}

fn validate_new_name(name: &str) -> Result<(), EngineError> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(invalid(
            "Use a short name of lowercase letters, digits, '.', '_' or '-' (e.g. \"omni\").",
        ));
    }
    if BUILT_IN.contains(&name) {
        return Err(invalid(&format!(
            "\"{name}\" is one of Pi's built-in providers — pick another name."
        )));
    }
    Ok(())
}

/// `raw` as Pi's `baseUrl`: `http(s)`, a host, no credentials, query or
/// fragment; any trailing `/` dropped (Pi appends `/chat/completions`).
pub(super) fn normalize_base_url(raw: &str) -> Result<String, EngineError> {
    let bad = || invalid("Enter a base URL like https://api.example.com/v1.");
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(bad());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_models(models: &[PiProviderModel]) -> Result<Vec<PiProviderModel>, EngineError> {
    let mut out: Vec<PiProviderModel> = Vec::new();
    for model in models {
        let id = model.id.trim();
        if id.is_empty() || out.iter().any(|m| m.id == id) {
            continue;
        }
        out.push(PiProviderModel {
            id: id.to_string(),
            name: model
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
        });
    }
    if out.is_empty() {
        return Err(invalid("Pick at least one model."));
    }
    if out.len() > MAX_MODELS {
        return Err(invalid(&format!(
            "Pick at most {MAX_MODELS} models — Pi checks each one when listing models."
        )));
    }
    Ok(out)
}

/// The new `models` array: `models` in order, each keeping any extra fields
/// (`contextWindow`, `cost`, …) its old entry had.
fn merge_models(old: Option<&Value>, models: &[PiProviderModel]) -> Value {
    let old: Vec<&Map<String, Value>> = old
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect();
    Value::Array(
        models
            .iter()
            .map(|model| {
                let mut entry = old
                    .iter()
                    .find(|m| m.get("id").and_then(Value::as_str) == Some(model.id.as_str()))
                    .map(|m| (*m).clone())
                    .unwrap_or_default();
                entry.insert("id".into(), json!(model.id));
                if let Some(name) = &model.name {
                    entry.insert("name".into(), json!(name));
                }
                Value::Object(entry)
            })
            .collect(),
    )
}

/// A short reason for a failed request (the URL is already in our message).
fn without_url(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timed out"
    } else if err.is_connect() {
        "connection failed"
    } else {
        "request failed"
    }
}

#[cfg(test)]
mod tests;
