// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Management API for the SQLite rule database.
//!
//! Handlers are registered twice: once under the global `/rules` namespace used
//! by the WebUI rule page, and once under `/plugins/<tag>/rules` for
//! `sqlite_rule_set` provider instances. Both expose the same store.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, StatusCode};
use serde::{Deserialize, Serialize};

use super::{
    CategoryCount, DeleteOutcome, ListQuery, MAX_LIST_LIMIT, NewRule, RuleKind, RulePatch,
    RuleRecord, RuleStats, RuleStore,
};
use crate::api::query::{parse_usize_param, visit_query_params};
use crate::api::{ApiHandler, json_error, json_ok};
use crate::infra::error::Result as DnsResult;

#[derive(Debug, Clone, Copy)]
enum RuleAction {
    List,
    Add,
    Update,
    Delete,
    Clear,
}

#[derive(Debug)]
struct RuleApiHandler {
    store: Arc<RuleStore>,
    action: RuleAction,
}

impl RuleApiHandler {
    fn new(store: &Arc<RuleStore>, action: RuleAction) -> Self {
        Self {
            store: store.clone(),
            action,
        }
    }
}

#[async_trait]
impl ApiHandler for RuleApiHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        match self.action {
            RuleAction::List => {
                let query = match parse_list_query(request.uri().query()) {
                    Ok(query) => query,
                    Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
                };
                let store = self.store.clone();
                match run_blocking(move || store.list(&query)).await {
                    Ok(page) => json_ok(
                        StatusCode::OK,
                        &ListResponse {
                            ok: true,
                            database: self.store.path().display().to_string(),
                            total: page.total,
                            next_cursor: page.next_cursor,
                            stats: page.stats,
                            categories: page.categories,
                            rules: page.rules,
                        },
                    ),
                    Err(err) => {
                        json_error(StatusCode::INTERNAL_SERVER_ERROR, "rule_list_failed", err)
                    }
                }
            }
            RuleAction::Add => {
                let body = match parse_add_request(request.body()) {
                    Ok(body) => body,
                    Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_body", err),
                };
                let source = body.source.unwrap_or_default();
                let enabled = body.enabled.unwrap_or(true);
                let category = body.category.unwrap_or_else(|| "default".to_string());
                let rules = body.rules.into_iter().map(RuleInput::into_rule).collect();
                let store = self.store.clone();
                match run_blocking(move || store.add_rules(rules, &source, enabled, &category))
                    .await
                {
                    Ok(outcome) => json_ok(
                        StatusCode::OK,
                        &AddResponse {
                            ok: true,
                            added: outcome.added,
                            skipped: outcome.skipped,
                            total: outcome.total,
                            stats: outcome.stats,
                            categories: outcome.categories,
                        },
                    ),
                    Err(err) => json_error(StatusCode::BAD_REQUEST, "rule_add_failed", err),
                }
            }
            RuleAction::Update => {
                let patch = match parse_update_request(request.body()) {
                    Ok(patch) => patch,
                    Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_body", err),
                };
                let store = self.store.clone();
                match run_blocking(move || store.update_rule(patch)).await {
                    Ok(Some(outcome)) => json_ok(
                        StatusCode::OK,
                        &UpdateResponse {
                            ok: true,
                            rule: outcome.rule,
                            total: outcome.total,
                            stats: outcome.stats,
                            categories: outcome.categories,
                        },
                    ),
                    Ok(None) => json_error(
                        StatusCode::NOT_FOUND,
                        "rule_not_found",
                        "rule does not exist",
                    ),
                    Err(err) => json_error(StatusCode::BAD_REQUEST, "rule_update_failed", err),
                }
            }
            RuleAction::Delete => {
                let request = match parse_delete_request(request.uri().query(), request.body()) {
                    Ok(request) => request,
                    Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_body", err),
                };
                let store = self.store.clone();
                match run_blocking(move || delete_rules(&store, request)).await {
                    Ok(outcome) => json_ok(
                        StatusCode::OK,
                        &DeleteResponse {
                            ok: true,
                            removed: outcome.removed,
                            total: outcome.total,
                            stats: outcome.stats,
                            categories: outcome.categories,
                        },
                    ),
                    Err(err) => {
                        json_error(StatusCode::INTERNAL_SERVER_ERROR, "rule_delete_failed", err)
                    }
                }
            }
            RuleAction::Clear => {
                let store = self.store.clone();
                match run_blocking(move || store.clear()).await {
                    Ok(outcome) => json_ok(
                        StatusCode::OK,
                        &DeleteResponse {
                            ok: true,
                            removed: outcome.removed,
                            total: outcome.total,
                            stats: outcome.stats,
                            categories: outcome.categories,
                        },
                    ),
                    Err(err) => {
                        json_error(StatusCode::INTERNAL_SERVER_ERROR, "rule_clear_failed", err)
                    }
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct ListResponse {
    ok: bool,
    database: String,
    total: usize,
    next_cursor: Option<i64>,
    stats: RuleStats,
    categories: Vec<CategoryCount>,
    rules: Vec<RuleRecord>,
}

#[derive(Debug, Serialize)]
struct AddResponse {
    ok: bool,
    added: usize,
    skipped: usize,
    total: usize,
    stats: RuleStats,
    categories: Vec<CategoryCount>,
}

#[derive(Debug, Serialize)]
struct UpdateResponse {
    ok: bool,
    rule: RuleRecord,
    total: usize,
    stats: RuleStats,
    categories: Vec<CategoryCount>,
}

#[derive(Debug, Serialize)]
struct DeleteResponse {
    ok: bool,
    removed: usize,
    total: usize,
    stats: RuleStats,
    categories: Vec<CategoryCount>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RuleInput {
    Plain(String),
    Detailed {
        rule: String,
        #[serde(default)]
        kind: Option<RuleKind>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        enabled: Option<bool>,
    },
}

impl RuleInput {
    fn into_rule(self) -> NewRule {
        match self {
            Self::Plain(rule) => NewRule {
                rule,
                kind: None,
                category: None,
                enabled: None,
            },
            Self::Detailed {
                rule,
                kind,
                category,
                enabled,
            } => NewRule {
                rule,
                kind,
                category,
                enabled,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct AddRequest {
    rules: Vec<RuleInput>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateRequest {
    id: i64,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    kind: Option<RuleKind>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct DeleteRequest {
    #[serde(default)]
    ids: Vec<i64>,
    #[serde(default)]
    rules: Vec<RuleInput>,
}

fn parse_list_query(query: Option<&str>) -> std::result::Result<ListQuery, String> {
    let mut parsed = ListQuery::default();
    visit_query_params(query, |key, value| {
        match key {
            "cursor" => {
                let cursor = value
                    .trim()
                    .parse::<i64>()
                    .map_err(|err| format!("invalid cursor query parameter: {err}"))?;
                if cursor < 0 {
                    return Err("cursor must not be negative".to_string());
                }
                parsed.cursor = Some(cursor);
            }
            "limit" => {
                let limit = parse_usize_param(value, |err| {
                    format!("invalid limit query parameter: {err}")
                })?;
                if limit == 0 {
                    return Err("limit must be greater than 0".to_string());
                }
                parsed.limit = limit.min(MAX_LIST_LIMIT);
            }
            "q" => {
                let keyword = value.trim();
                if !keyword.is_empty() {
                    parsed.keyword = Some(keyword.to_string());
                }
            }
            "kind" => {
                let kind = value.trim();
                if !kind.is_empty() && !kind.eq_ignore_ascii_case("all") {
                    parsed.kind = Some(
                        RuleKind::parse(kind)
                            .ok_or_else(|| format!("unsupported rule kind '{kind}'"))?,
                    );
                }
            }
            "category" => {
                let category = value.trim();
                if !category.is_empty() && !category.eq_ignore_ascii_case("all") {
                    parsed.category = Some(category.to_ascii_lowercase());
                }
            }
            "enabled" => {
                let enabled = parse_bool(value)?;
                parsed.enabled = enabled;
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(parsed)
}

fn parse_bool(value: &str) -> std::result::Result<Option<bool>, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(Some(true)),
        "0" | "false" | "no" => Ok(Some(false)),
        "all" | "" => Ok(None),
        other => Err(format!("invalid boolean value '{other}'")),
    }
}

fn parse_add_request(body: &Bytes) -> std::result::Result<AddRequest, String> {
    let request = serde_json::from_slice::<AddRequest>(body)
        .map_err(|err| format!("invalid json body: {err}"))?;
    if request.rules.is_empty() {
        return Err("rules must contain at least one rule".to_string());
    }
    Ok(request)
}

fn parse_update_request(body: &Bytes) -> std::result::Result<RulePatch, String> {
    let request = serde_json::from_slice::<UpdateRequest>(body)
        .map_err(|err| format!("invalid json body: {err}"))?;
    if request.id <= 0 {
        return Err("id must be a positive integer".to_string());
    }
    if request.rule.is_none()
        && request.kind.is_none()
        && request.category.is_none()
        && request.source.is_none()
        && request.enabled.is_none()
    {
        return Err("request must change at least one field".to_string());
    }
    Ok(RulePatch {
        id: request.id,
        rule: request.rule,
        kind: request.kind,
        category: request.category,
        source: request.source,
        enabled: request.enabled,
    })
}

fn parse_delete_request(
    query: Option<&str>,
    body: &Bytes,
) -> std::result::Result<DeleteTarget, String> {
    let mut ids = Vec::new();
    visit_query_params(query, |key, value| {
        if key == "id" {
            let id = value
                .trim()
                .parse::<i64>()
                .map_err(|err| format!("invalid id query parameter: {err}"))?;
            if id <= 0 {
                return Err("id must be a positive integer".to_string());
            }
            ids.push(id);
        }
        Ok(())
    })?;

    let request = if body.is_empty() {
        DeleteRequest::default()
    } else {
        serde_json::from_slice::<DeleteRequest>(body)
            .map_err(|err| format!("invalid json body: {err}"))?
    };
    ids.extend(request.ids);
    let rules: Vec<NewRule> = request
        .rules
        .into_iter()
        .map(RuleInput::into_rule)
        .collect();
    if ids.is_empty() && rules.is_empty() {
        return Err("request must name rules by id or rule text".to_string());
    }
    Ok(DeleteTarget { ids, rules })
}

struct DeleteTarget {
    ids: Vec<i64>,
    rules: Vec<NewRule>,
}

fn delete_rules(store: &RuleStore, target: DeleteTarget) -> DnsResult<DeleteOutcome> {
    if !target.ids.is_empty() {
        let outcome = store.delete_ids(&target.ids)?;
        if target.rules.is_empty() {
            return Ok(outcome);
        }
        let by_rule = store.delete_rules(&target.rules)?;
        return Ok(DeleteOutcome {
            removed: outcome.removed + by_rule.removed,
            total: by_rule.total,
            stats: by_rule.stats,
            categories: by_rule.categories,
        });
    }
    store.delete_rules(&target.rules)
}

async fn run_blocking<T, F>(job: F) -> std::result::Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> DnsResult<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(job).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.to_string()),
        Err(err) => Err(format!("rule_db task failed: {err}")),
    }
}

/// Register the WebUI-facing `/rules` routes.
pub fn register_global(store: &Arc<RuleStore>) -> DnsResult<()> {
    crate::register_api_route!(GET "/rules" => RuleApiHandler::new(store, RuleAction::List))?;
    crate::register_api_route!(POST "/rules" => RuleApiHandler::new(store, RuleAction::Add))?;
    crate::register_api_route!(PUT "/rules" => RuleApiHandler::new(store, RuleAction::Update))?;
    crate::register_api_route!(DELETE "/rules" => RuleApiHandler::new(store, RuleAction::Delete))?;
    crate::register_api_route!(
        POST "/rules/clear" => RuleApiHandler::new(store, RuleAction::Clear)
    )?;
    Ok(())
}

/// Register the same routes under `/plugins/<tag>/rules` so a rule store can
/// also be managed from a provider instance page or external tooling.
pub fn register_plugin(tag: &str, store: &Arc<RuleStore>) -> DnsResult<()> {
    crate::register_plugin_api!(
        tag,
        GET "/rules" => RuleApiHandler::new(store, RuleAction::List),
        POST "/rules" => RuleApiHandler::new(store, RuleAction::Add),
        PUT "/rules" => RuleApiHandler::new(store, RuleAction::Update),
        DELETE "/rules" => RuleApiHandler::new(store, RuleAction::Delete),
        POST "/rules/clear" => RuleApiHandler::new(store, RuleAction::Clear),
    )
}
