//! Desktop contact RPCs and local contact persistence.

use anyhow::{Context, Result};
use puffer_config::ConfigPaths;
use puffer_provider_registry::{AuthStore, StoredCredential};
use puffer_subscriptions::{
    connector_slug_accepts_contact_id, contact_id_prefix, contact_ids_from_payload,
    normalize_contact_id, normalize_contact_ids, ConnectorContact, ContactContext, ContactProposal,
    SavedContact,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[path = "daemon_contacts_telegram.rs"]
mod daemon_contacts_telegram;
use daemon_contacts_telegram::collect_telegram_candidates;

const DEFAULT_LIMIT: usize = 30;
const MAX_LIMIT: usize = 120;
const TELEGRAM_CONTEXT_LIMIT: usize = 100;
const GOOGLE_CONTEXT_LIMIT: usize = 10;
const DAY_MS: i128 = 86_400_000;

#[derive(Debug, Default, Deserialize, Serialize)]
struct ContactStoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    contacts: Vec<SavedContact>,
}

#[derive(Debug, Deserialize)]
struct ContactListParams {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContactSaveParams {
    #[serde(default)]
    id: Option<String>,
    name: String,
    description: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    contact_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ContactDeleteParams {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ContactContextParams {
    #[serde(default)]
    contact_ids: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ContactInferParams {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    name: Option<String>,
    avatar: Option<String>,
    score: f64,
    context: Vec<ContactContext>,
}

/// Lists saved contacts plus ranked connector candidates.
pub(crate) fn handle_contacts_list(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactListParams =
        serde_json::from_value(params.clone()).unwrap_or(ContactListParams {
            limit: Some(DEFAULT_LIMIT),
            query: None,
        });
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let saved = filtered_saved_contacts(load_store(paths)?.contacts, query);
    let candidates = filtered_candidates(paths, limit, query)?;
    Ok(json!({
        "contacts": saved,
        "candidates": candidates,
    }))
}

/// Searches connector contact ids for autocomplete.
pub(crate) fn handle_contacts_search(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactListParams =
        serde_json::from_value(params.clone()).unwrap_or(ContactListParams {
            limit: Some(DEFAULT_LIMIT),
            query: None,
        });
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let saved = filtered_saved_contacts(load_store(paths)?.contacts, query);
    let candidates = searched_candidates(paths, limit, query)?;
    Ok(json!({
        "contacts": saved,
        "candidates": candidates,
    }))
}

/// Saves a user-curated contact and returns the refreshed contact snapshot.
pub(crate) fn handle_contacts_save(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactSaveParams =
        serde_json::from_value(params.clone()).context("invalid contact save params")?;
    let name = params.name.trim();
    if name.is_empty() {
        anyhow::bail!("contact name must not be empty");
    }
    let contact_ids = normalize_contact_ids(params.contact_ids);
    if contact_ids.is_empty() {
        anyhow::bail!("contact must contain at least one valid contact id");
    }
    let mut store = load_store(paths)?;
    let id = params
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let saved = SavedContact {
        id: id.clone(),
        name: name.to_string(),
        description: params.description.trim().to_string(),
        avatar: params
            .avatar
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        contact_ids,
    };
    store.contacts.retain(|contact| contact.id != id);
    store.contacts.push(saved);
    store.contacts.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    save_store(paths, &store)?;
    handle_contacts_list(paths, &json!({ "limit": DEFAULT_LIMIT }))
}

/// Deletes a user-curated contact and returns the refreshed contact snapshot.
pub(crate) fn handle_contacts_delete(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactDeleteParams =
        serde_json::from_value(params.clone()).context("invalid contact delete params")?;
    let mut store = load_store(paths)?;
    let before = store.contacts.len();
    store.contacts.retain(|contact| contact.id != params.id);
    if store.contacts.len() == before {
        anyhow::bail!("contact `{}` not found", params.id);
    }
    save_store(paths, &store)?;
    handle_contacts_list(paths, &json!({ "limit": DEFAULT_LIMIT }))
}

/// Returns recent context for one or more connector contact ids.
pub(crate) fn handle_contacts_context(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactContextParams =
        serde_json::from_value(params.clone()).context("invalid contact context params")?;
    let contact_ids = normalize_contact_ids(params.contact_ids);
    let limit = params
        .limit
        .unwrap_or(TELEGRAM_CONTEXT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let contexts = contact_contexts(paths, &contact_ids, limit)?;
    Ok(json!({ "contact_ids": contact_ids, "context": contexts }))
}

/// Infers contact group proposals from top connector candidates.
pub(crate) fn handle_contacts_infer(paths: &ConfigPaths, params: &Value) -> Result<Value> {
    let params: ContactInferParams =
        serde_json::from_value(params.clone()).unwrap_or(ContactInferParams {
            limit: Some(DEFAULT_LIMIT),
            model: None,
        });
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, DEFAULT_LIMIT);
    let candidates = inference_candidates(paths)?;
    let proposals = infer_proposals(paths, &candidates, limit, params.model.as_deref())?;
    Ok(json!({ "proposals": proposals, "candidates": candidates }))
}

fn filtered_candidates(
    paths: &ConfigPaths,
    limit: usize,
    query: Option<&str>,
) -> Result<Vec<ConnectorContact>> {
    let mut candidates = ranked_candidates(paths)?;
    if let Some(query) = query {
        let query = query.to_ascii_lowercase();
        candidates.retain(|candidate| {
            candidate.id.to_ascii_lowercase().contains(&query)
                || candidate
                    .name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&query)
        });
    }
    candidates.truncate(limit);
    Ok(candidates
        .into_iter()
        .map(Candidate::into_contact)
        .collect())
}

fn searched_candidates(
    paths: &ConfigPaths,
    limit: usize,
    query: Option<&str>,
) -> Result<Vec<ConnectorContact>> {
    let Some(query) = query else {
        return filtered_candidates(paths, limit, None);
    };
    let mut by_id: HashMap<String, Candidate> = HashMap::new();
    collect_telegram_candidates(paths, &mut by_id)?;
    collect_connector_method_search_candidates(&mut by_id, query, Some(limit));
    collect_history_candidates(&mut by_id);
    let query = query.to_ascii_lowercase();
    let mut candidates = by_id
        .into_values()
        .filter(|candidate| {
            candidate.id.to_ascii_lowercase().contains(&query)
                || candidate
                    .name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
    candidates.truncate(limit);
    Ok(candidates
        .into_iter()
        .map(Candidate::into_contact)
        .collect())
}

fn filtered_saved_contacts(
    mut contacts: Vec<SavedContact>,
    query: Option<&str>,
) -> Vec<SavedContact> {
    let Some(query) = query else {
        return contacts;
    };
    let query = query.to_ascii_lowercase();
    contacts.retain(|contact| {
        contact.name.to_ascii_lowercase().contains(&query)
            || contact.description.to_ascii_lowercase().contains(&query)
            || contact
                .contact_ids
                .iter()
                .any(|id| id.to_ascii_lowercase().contains(&query))
    });
    contacts
}

fn ranked_candidates(paths: &ConfigPaths) -> Result<Vec<Candidate>> {
    let mut by_id: HashMap<String, Candidate> = HashMap::new();
    collect_telegram_candidates(paths, &mut by_id)?;
    collect_connector_method_candidates(&mut by_id);
    collect_history_candidates(&mut by_id);
    let mut candidates = by_id.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(candidates)
}

fn inference_candidates(paths: &ConfigPaths) -> Result<Vec<ConnectorContact>> {
    Ok(
        sample_inference_candidates(ranked_candidates(paths)?, DEFAULT_LIMIT)
            .into_iter()
            .map(Candidate::into_contact)
            .collect(),
    )
}

fn sample_inference_candidates(candidates: Vec<Candidate>, per_connector: usize) -> Vec<Candidate> {
    let mut buckets: HashMap<String, Vec<Candidate>> = HashMap::new();
    for candidate in candidates {
        let prefix = contact_id_prefix(&candidate.id).unwrap_or("unknown");
        buckets
            .entry(prefix.to_string())
            .or_default()
            .push(candidate);
    }
    let mut prefixes = buckets.keys().cloned().collect::<Vec<_>>();
    prefixes.sort();
    let mut sampled = Vec::new();
    for prefix in prefixes {
        if let Some(mut bucket) = buckets.remove(&prefix) {
            bucket.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.id.cmp(&right.id))
            });
            sampled.extend(bucket.into_iter().take(per_connector));
        }
    }
    sampled
}

fn collect_connector_method_search_candidates(
    by_id: &mut HashMap<String, Candidate>,
    query: &str,
    limit: Option<usize>,
) {
    let Ok(manager) = puffer_core::subscription_manager() else {
        return;
    };
    for connection in manager.connection_store().list() {
        let Ok(Some(contacts)) =
            manager.search_connector_contacts(&connection.slug, query.to_string(), limit)
        else {
            continue;
        };
        merge_connector_contacts(by_id, &connection.connector_slug, contacts);
    }
}

fn collect_connector_method_candidates(by_id: &mut HashMap<String, Candidate>) {
    let Ok(manager) = puffer_core::subscription_manager() else {
        return;
    };
    for connection in manager.connection_store().list() {
        let Ok(Some(contacts)) =
            manager.list_connector_contacts(&connection.slug, None, Some(DEFAULT_LIMIT))
        else {
            continue;
        };
        merge_connector_contacts(by_id, &connection.connector_slug, contacts);
    }
}

fn merge_connector_contacts(
    by_id: &mut HashMap<String, Candidate>,
    connector_slug: &str,
    contacts: Vec<ConnectorContact>,
) {
    for contact in contacts {
        let Some(id) = normalize_contact_id(&contact.id) else {
            continue;
        };
        if !connector_slug_accepts_contact_id(connector_slug, &id) {
            continue;
        }
        let entry = by_id.entry(id.clone()).or_insert_with(|| Candidate {
            id: id.clone(),
            name: contact.name.clone().or_else(|| Some(id.clone())),
            avatar: contact.avatar.clone(),
            score: 0.0,
            context: Vec::new(),
        });
        entry.score += contact.score.max(0.01);
        if entry.name.is_none() {
            entry.name = contact.name;
        }
        if entry.avatar.is_none() {
            entry.avatar = contact.avatar;
        }
        for context in contact.context {
            push_context(
                entry,
                context,
                GOOGLE_CONTEXT_LIMIT.max(TELEGRAM_CONTEXT_LIMIT),
            );
        }
    }
}

fn collect_history_candidates(by_id: &mut HashMap<String, Candidate>) {
    let Ok(manager) = puffer_core::subscription_manager() else {
        return;
    };
    for run in manager.history_store().list() {
        let payload = run
            .trigger_info
            .get("payload")
            .cloned()
            .unwrap_or(Value::Null);
        let text = run
            .trigger_info
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            continue;
        }
        let timestamp_ms = run
            .trigger_info
            .get("received_at_ms")
            .and_then(Value::as_i64)
            .map(i128::from);
        let score = entropy_score(&text) / days_since(timestamp_ms.unwrap_or(run.started_at_ms));
        for id in contact_ids_from_payload(&payload) {
            let entry = by_id.entry(id.clone()).or_insert_with(|| Candidate {
                id: id.clone(),
                name: name_from_payload(&payload).or_else(|| Some(id.clone())),
                avatar: None,
                score: 0.0,
                context: Vec::new(),
            });
            entry.score += score.max(0.01);
            push_context(
                entry,
                ContactContext {
                    kind: "message".to_string(),
                    text: text.clone(),
                    timestamp_ms,
                    payload: payload.clone(),
                },
                GOOGLE_CONTEXT_LIMIT,
            );
        }
    }
}

fn contact_contexts(
    paths: &ConfigPaths,
    contact_ids: &[String],
    limit: usize,
) -> Result<Vec<ContactContext>> {
    let wanted = contact_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut contexts = connector_method_contexts(contact_ids, limit);
    for candidate in ranked_candidates(paths)? {
        if wanted.is_empty() || wanted.contains(&candidate.id) {
            let cap = if candidate.id.starts_with("telegram@") {
                TELEGRAM_CONTEXT_LIMIT.min(limit)
            } else {
                GOOGLE_CONTEXT_LIMIT.min(limit)
            };
            contexts.extend(candidate.context.into_iter().take(cap));
        }
    }
    contexts.sort_by(|left, right| right.timestamp_ms.cmp(&left.timestamp_ms));
    contexts.truncate(limit);
    Ok(contexts)
}

fn connector_method_contexts(contact_ids: &[String], limit: usize) -> Vec<ContactContext> {
    if contact_ids.is_empty() {
        return Vec::new();
    }
    let Ok(manager) = puffer_core::subscription_manager() else {
        return Vec::new();
    };
    let mut contexts = Vec::new();
    for connection in manager.connection_store().list() {
        let owned_ids = contact_ids
            .iter()
            .filter(|id| connector_slug_accepts_contact_id(&connection.connector_slug, id))
            .cloned()
            .collect::<Vec<_>>();
        if owned_ids.is_empty() {
            continue;
        }
        let Ok(Some((_ids, items))) =
            manager.connector_contact_context(&connection.slug, owned_ids, Some(limit))
        else {
            continue;
        };
        contexts.extend(items);
    }
    contexts
}

fn infer_proposals(
    paths: &ConfigPaths,
    candidates: &[ConnectorContact],
    limit: usize,
    model: Option<&str>,
) -> Result<Vec<ContactProposal>> {
    match openai_key(paths) {
        Some(key) => {
            let model = model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("gpt-5.4-mini");
            infer_with_openai(&key, model, candidates, limit)
                .or_else(|_| Ok(heuristic_proposals(candidates, limit)))
        }
        None => Ok(heuristic_proposals(candidates, limit)),
    }
}

fn infer_with_openai(
    api_key: &str,
    model: &str,
    candidates: &[ConnectorContact],
    limit: usize,
) -> Result<Vec<ContactProposal>> {
    let compact = candidates
        .iter()
        .map(|candidate| {
            json!({
                "id": candidate.id,
                "name": candidate.name,
                "score": candidate.score,
                "context": candidate.context.iter().take(3).map(|ctx| &ctx.text).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let body = json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You infer the user's important contacts from connector candidates. You may only call CreateContact JSON tool calls. Propose at most 30 contacts. Each description must be exactly two sentences explaining why this contact matters and why it is not spam."
            },
            {
                "role": "user",
                "content": serde_json::to_string(&compact)?
            }
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "CreateContact",
                "description": "Create one grouped contact proposal.",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {"type": "string"},
                        "description": {"type": "string"},
                        "avatar": {"type": ["string", "null"]},
                        "contact_ids": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    },
                    "required": ["name", "description", "contact_ids"]
                }
            }
        }],
        "tool_choice": "required"
    });
    let response = reqwest::blocking::Client::new()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .context("send OpenAI contact inference request")?
        .json::<Value>()
        .context("parse OpenAI contact inference response")?;
    let proposals = parse_openai_proposals(&response, limit);
    if proposals.is_empty() {
        anyhow::bail!("contact inference returned no CreateContact calls");
    }
    Ok(proposals)
}

fn parse_openai_proposals(response: &Value, limit: usize) -> Vec<ContactProposal> {
    let mut proposals = Vec::new();
    if let Some(calls) = response
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    {
        for call in calls {
            let Some(args) = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            else {
                continue;
            };
            if let Some(proposal) = proposal_from_value(args) {
                proposals.push(proposal);
            }
        }
    }
    if proposals.is_empty() {
        if let Some(content) = response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .and_then(extract_json)
        {
            match content {
                Value::Array(values) => {
                    proposals.extend(values.into_iter().filter_map(proposal_from_value));
                }
                value => {
                    if let Some(proposal) = proposal_from_value(value) {
                        proposals.push(proposal);
                    }
                }
            }
        }
    }
    proposals.truncate(limit);
    proposals
}

fn proposal_from_value(value: Value) -> Option<ContactProposal> {
    let mut proposal = serde_json::from_value::<ContactProposal>(value).ok()?;
    proposal.contact_ids = normalize_contact_ids(proposal.contact_ids);
    (!proposal.name.trim().is_empty() && !proposal.contact_ids.is_empty()).then_some(proposal)
}

fn heuristic_proposals(candidates: &[ConnectorContact], limit: usize) -> Vec<ContactProposal> {
    candidates
        .iter()
        .take(limit)
        .map(|candidate| {
            let name = candidate
                .name
                .as_deref()
                .unwrap_or(candidate.id.as_str())
                .to_string();
            ContactProposal {
                name,
                description: format!(
                    "{} appears repeatedly in recent connector context. The messages include direct, high-signal exchanges rather than broad broadcast spam.",
                    candidate.id
                ),
                avatar: candidate.avatar.clone(),
                contact_ids: vec![candidate.id.clone()],
            }
        })
        .collect()
}

fn openai_key(paths: &ConfigPaths) -> Option<String> {
    let path = paths.user_config_dir.join("auth.json");
    let store = AuthStore::load(&path).ok()?;
    match store.providers.get("openai") {
        Some(StoredCredential::ApiKey { key }) => Some(key.clone()),
        _ => None,
    }
}

fn load_store(paths: &ConfigPaths) -> Result<ContactStoreFile> {
    let path = contacts_path(paths);
    if !path.exists() {
        return Ok(ContactStoreFile::default());
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(ContactStoreFile::default());
    }
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn save_store(paths: &ConfigPaths, store: &ContactStoreFile) -> Result<()> {
    let path = contacts_path(paths);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(store)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))
}

fn contacts_path(paths: &ConfigPaths) -> PathBuf {
    paths
        .workspace_config_dir
        .join("runtime")
        .join("contacts.json")
}

fn name_from_payload(payload: &Value) -> Option<String> {
    for path in [
        "/sender_name",
        "/chat_title",
        "/from",
        "/message/sender",
        "/event/title",
    ] {
        if let Some(value) = payload
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn push_context(candidate: &mut Candidate, context: ContactContext, limit: usize) {
    if context.text.trim().is_empty() {
        return;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    context.text.hash(&mut hasher);
    let hash = hasher.finish();
    if candidate.context.iter().any(|existing| {
        let mut existing_hasher = std::collections::hash_map::DefaultHasher::new();
        existing.text.hash(&mut existing_hasher);
        existing_hasher.finish() == hash
    }) {
        return;
    }
    candidate.context.push(context);
    sort_context(&mut candidate.context, limit);
}

fn sort_context(context: &mut Vec<ContactContext>, limit: usize) {
    context.sort_by(|left, right| {
        let entropy_order = entropy_score(&right.text)
            .partial_cmp(&entropy_score(&left.text))
            .unwrap_or(std::cmp::Ordering::Equal);
        entropy_order.then_with(|| right.timestamp_ms.cmp(&left.timestamp_ms))
    });
    context.truncate(limit);
}

fn entropy_score(text: &str) -> f64 {
    let tokens = text
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|token| token.len() > 1)
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return 0.1;
    }
    let unique = tokens.iter().collect::<HashSet<_>>().len() as f64;
    let length = tokens.len() as f64;
    (unique / length) * length.ln_1p().max(1.0)
}

fn days_since(timestamp_ms: i128) -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i128)
        .unwrap_or(timestamp_ms);
    let delta = (now - timestamp_ms).max(DAY_MS);
    (delta as f64) / (DAY_MS as f64)
}

fn extract_json(content: &str) -> Option<Value> {
    serde_json::from_str(content).ok().or_else(|| {
        let start = content.find(['[', '{'])?;
        let end = content.rfind([']', '}'])?;
        serde_json::from_str(&content[start..=end]).ok()
    })
}

impl Candidate {
    fn into_contact(self) -> ConnectorContact {
        ConnectorContact {
            id: self.id,
            avatar: self.avatar,
            name: self.name,
            context: self.context,
            score: self.score,
        }
    }
}

#[cfg(test)]
#[path = "daemon_contacts_tests.rs"]
mod tests;
