// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite persistence and compiled snapshots for the rule database.
//!
//! Rules are grouped into named categories (for example `ad`, `gfw`, `china`,
//! and `cdn`). The provider can select a subset of categories, while the
//! management API and WebUI can filter and count by category.

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use arc_swap::ArcSwap;
use jiff::Timestamp;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::config::types::{RuleDbConfig, RuleImportEntry, RuleImportMode};
use crate::core::rule_matcher::{
    DomainRuleKind, DomainRuleMatcher, IpPrefixMatcher, split_domain_rule_expression,
};
use crate::infra::error::{DnsError, Result as DnsResult};
use crate::infra::io::{LineClassifier, TextSource};
use crate::proto::Name;

const MAX_CATEGORY_LEN: usize = 32;
pub const DEFAULT_CATEGORY: &str = "default";
pub const DEFAULT_LIST_LIMIT: usize = 200;
pub const MAX_LIST_LIMIT: usize = 5_000;

/// Rule family stored in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Domain,
    Ip,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Ip => "ip",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "domain" => Some(Self::Domain),
            "ip" => Some(Self::Ip),
            _ => None,
        }
    }
}

/// One persisted rule row.
#[derive(Debug, Clone, Serialize)]
pub struct RuleRecord {
    pub id: i64,
    pub kind: RuleKind,
    pub rule: String,
    pub category: String,
    pub source: String,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Aggregate counts for the WebUI summary cards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RuleStats {
    pub total: usize,
    pub domain: usize,
    pub ip: usize,
    pub enabled: usize,
    pub disabled: usize,
}

/// Per-category aggregate counts.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CategoryCount {
    pub category: String,
    pub total: usize,
    pub domain: usize,
    pub ip: usize,
    pub enabled: usize,
}

/// List request after query-string parsing.
#[derive(Debug, Clone)]
pub struct ListQuery {
    pub cursor: Option<i64>,
    pub limit: usize,
    pub keyword: Option<String>,
    pub kind: Option<RuleKind>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
}

impl Default for ListQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: DEFAULT_LIST_LIMIT,
            keyword: None,
            kind: None,
            category: None,
            enabled: None,
        }
    }
}

/// One page of rules plus unfiltered summary counts.
#[derive(Debug, Clone, Serialize)]
pub struct ListPage {
    pub rules: Vec<RuleRecord>,
    pub total: usize,
    pub next_cursor: Option<i64>,
    pub stats: RuleStats,
    pub categories: Vec<CategoryCount>,
}

/// Rule input used by add/delete requests.
#[derive(Debug, Clone)]
pub struct NewRule {
    pub rule: String,
    pub kind: Option<RuleKind>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
}

/// Patch applied to one existing rule.
#[derive(Debug, Clone)]
pub struct RulePatch {
    pub id: i64,
    pub rule: Option<String>,
    pub kind: Option<RuleKind>,
    pub category: Option<String>,
    pub source: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    pub added: usize,
    pub skipped: usize,
    pub total: usize,
    pub stats: RuleStats,
    pub categories: Vec<CategoryCount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    pub rule: RuleRecord,
    pub total: usize,
    pub stats: RuleStats,
    pub categories: Vec<CategoryCount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteOutcome {
    pub removed: usize,
    pub total: usize,
    pub stats: RuleStats,
    pub categories: Vec<CategoryCount>,
}

/// Compiled matchers for one category.
#[derive(Debug, Default)]
struct CategoryMatcher {
    domain: DomainRuleMatcher,
    ip: IpPrefixMatcher,
}

/// Immutable matching state published to providers.
#[derive(Debug, Default)]
struct RuleSnapshot {
    categories: BTreeMap<String, CategoryMatcher>,
    stats: RuleStats,
}

impl RuleSnapshot {
    fn matches_name(&self, name: &Name, filter: Option<&[String]>) -> bool {
        match filter {
            Some(categories) => categories.iter().any(|category| {
                self.categories
                    .get(category)
                    .is_some_and(|matcher| matcher.domain.is_match_name(name))
            }),
            None => self
                .categories
                .values()
                .any(|matcher| matcher.domain.is_match_name(name)),
        }
    }

    fn matches_ip(&self, ip: IpAddr, filter: Option<&[String]>) -> bool {
        match filter {
            Some(categories) => categories.iter().any(|category| {
                self.categories
                    .get(category)
                    .is_some_and(|matcher| matcher.ip.contains_ip(ip))
            }),
            None => self
                .categories
                .values()
                .any(|matcher| matcher.ip.contains_ip(ip)),
        }
    }
}

/// Shared rule database handle.
///
/// Reads on the DNS hot path only load the snapshot. Every mutation takes the
/// connection mutex, commits to SQLite, and then publishes a fully rebuilt
/// snapshot, so readers never observe a partially updated rule set.
pub struct RuleStore {
    path: PathBuf,
    db: Mutex<Connection>,
    snapshot: ArcSwap<RuleSnapshot>,
}

impl std::fmt::Debug for RuleStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuleStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl RuleStore {
    pub(super) fn open(config: &RuleDbConfig) -> DnsResult<Self> {
        let path = resolve_path(&config.path)?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = open_database(&path)?;
        create_schema(&mut conn)?;
        if should_import(&conn, config)? {
            import_rules(&mut conn, &config.import, config.import_mode)?;
        }

        let store = Self {
            path,
            db: Mutex::new(conn),
            snapshot: ArcSwap::from_pointee(RuleSnapshot::default()),
        };
        {
            let conn = store.connection()?;
            store.reload_locked(&conn)?;
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rebuild the compiled snapshot from the current database contents.
    pub fn reload(&self) -> DnsResult<()> {
        let conn = self.connection()?;
        self.reload_locked(&conn)
    }

    #[inline]
    pub fn contains_name(&self, name: &Name) -> bool {
        self.contains_name_in(name, None)
    }

    #[inline]
    pub fn contains_name_in(&self, name: &Name, categories: Option<&[String]>) -> bool {
        self.snapshot.load().matches_name(name, categories)
    }

    #[inline]
    pub fn contains_ip(&self, ip: IpAddr) -> bool {
        self.contains_ip_in(ip, None)
    }

    #[inline]
    pub fn contains_ip_in(&self, ip: IpAddr, categories: Option<&[String]>) -> bool {
        self.snapshot.load().matches_ip(ip, categories)
    }

    pub fn stats(&self) -> DnsResult<RuleStats> {
        let conn = self.connection()?;
        stats_locked(&conn).map_err(Into::into)
    }

    pub fn snapshot_stats(&self) -> RuleStats {
        self.snapshot.load().stats
    }

    pub fn categories(&self) -> DnsResult<Vec<CategoryCount>> {
        let conn = self.connection()?;
        category_counts_locked(&conn).map_err(Into::into)
    }

    pub fn list(&self, query: &ListQuery) -> DnsResult<ListPage> {
        let conn = self.connection()?;
        let mut filters = Vec::new();
        let mut params = Vec::new();

        if let Some(kind) = query.kind {
            filters.push("kind = ?");
            params.push(SqlValue::Text(kind.as_str().to_string()));
        }
        if let Some(category) = query
            .category
            .as_deref()
            .map(str::trim)
            .filter(|category| !category.is_empty() && !category.eq_ignore_ascii_case("all"))
        {
            filters.push("category = ?");
            params.push(SqlValue::Text(category.to_ascii_lowercase()));
        }
        if let Some(enabled) = query.enabled {
            filters.push("enabled = ?");
            params.push(SqlValue::Integer(bool_int(enabled)));
        }
        if let Some(keyword) = query
            .keyword
            .as_deref()
            .map(str::trim)
            .filter(|keyword| !keyword.is_empty())
        {
            filters.push("LOWER(rule) LIKE ? ESCAPE '\\'");
            params.push(SqlValue::Text(like_pattern(&keyword.to_lowercase())));
        }

        let where_clause = where_sql(&filters);
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM rules{where_clause}"),
            params_from_iter(params.iter()),
            |row| row.get(0),
        )?;

        let limit = query.limit.clamp(1, MAX_LIST_LIMIT);
        let mut page_filters = filters;
        let mut page_params = params;
        if let Some(cursor) = query.cursor {
            page_filters.push("id > ?");
            page_params.push(SqlValue::Integer(cursor));
        }
        let page_where = where_sql(&page_filters);
        page_params.push(SqlValue::Integer(limit as i64 + 1));

        let mut stmt = conn.prepare(&format!(
            "SELECT id, kind, rule, category, source, enabled, created_at_ms, updated_at_ms \
             FROM rules{page_where} ORDER BY id ASC LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(page_params.iter()), map_record)?;
        let mut rules = Vec::with_capacity(limit.min(256));
        for row in rows {
            rules.push(row?);
        }

        let next_cursor = if rules.len() > limit {
            rules.truncate(limit);
            rules.last().map(|rule| rule.id)
        } else {
            None
        };

        Ok(ListPage {
            rules,
            total: total as usize,
            next_cursor,
            stats: stats_locked(&conn)?,
            categories: category_counts_locked(&conn)?,
        })
    }

    pub fn add_rules(
        &self,
        rules: Vec<NewRule>,
        source: &str,
        enabled: bool,
        default_category: &str,
    ) -> DnsResult<AddOutcome> {
        let conn = self.connection()?;
        let source = normalize_source(source);
        let default_category = normalize_category(default_category)?;
        let now = now_ms();
        let mut added = 0usize;
        let mut skipped = 0usize;

        {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR IGNORE INTO rules(kind, rule, category, source, enabled, created_at_ms, updated_at_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                )?;
                for entry in rules {
                    let (kind, rule) = canonicalize_rule(&entry.rule, entry.kind)?;
                    let category = match entry.category.as_deref() {
                        Some(category) => normalize_category(category)?,
                        None => default_category.clone(),
                    };
                    let row_enabled = entry.enabled.unwrap_or(enabled);
                    let changed = stmt.execute(params![
                        kind.as_str(),
                        rule,
                        category,
                        source,
                        bool_int(row_enabled),
                        now
                    ])?;
                    if changed == 0 {
                        skipped += 1;
                    } else {
                        added += 1;
                    }
                }
            }
            tx.commit()?;
        }

        self.reload_locked(&conn)?;
        let stats = stats_locked(&conn)?;
        Ok(AddOutcome {
            added,
            skipped,
            total: stats.total,
            stats,
            categories: category_counts_locked(&conn)?,
        })
    }

    pub fn update_rule(&self, patch: RulePatch) -> DnsResult<Option<UpdateOutcome>> {
        let conn = self.connection()?;
        let existing: Option<RuleRecord> = conn
            .query_row(
                "SELECT id, kind, rule, category, source, enabled, created_at_ms, updated_at_ms \
                 FROM rules WHERE id = ?1",
                [patch.id],
                map_record,
            )
            .optional()?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let raw_rule = patch.rule.as_deref().unwrap_or(&existing.rule);
        let (kind, rule) = canonicalize_rule(raw_rule, patch.kind)?;
        let category = match patch.category.as_deref() {
            Some(category) => normalize_category(category)?,
            None => existing.category,
        };
        let source = patch
            .source
            .as_deref()
            .map(normalize_source)
            .unwrap_or(existing.source);
        let enabled = patch.enabled.unwrap_or(existing.enabled);

        let duplicate: Option<i64> = conn
            .query_row(
                "SELECT id FROM rules WHERE kind = ?1 AND rule = ?2 AND id <> ?3",
                params![kind.as_str(), rule, patch.id],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate.is_some() {
            return Err(DnsError::plugin(format!(
                "rule '{rule}' already exists in rule_db"
            )));
        }

        conn.execute(
            "UPDATE rules SET kind = ?1, rule = ?2, category = ?3, source = ?4, enabled = ?5, \
             updated_at_ms = ?6 WHERE id = ?7",
            params![
                kind.as_str(),
                rule,
                category,
                source,
                bool_int(enabled),
                now_ms(),
                patch.id
            ],
        )?;
        self.reload_locked(&conn)?;

        let record = conn.query_row(
            "SELECT id, kind, rule, category, source, enabled, created_at_ms, updated_at_ms \
             FROM rules WHERE id = ?1",
            [patch.id],
            map_record,
        )?;
        let stats = stats_locked(&conn)?;
        Ok(Some(UpdateOutcome {
            rule: record,
            total: stats.total,
            stats,
            categories: category_counts_locked(&conn)?,
        }))
    }

    pub fn delete_ids(&self, ids: &[i64]) -> DnsResult<DeleteOutcome> {
        let conn = self.connection()?;
        let mut removed = 0usize;
        if !ids.is_empty() {
            let placeholders = vec!["?"; ids.len()].join(", ");
            let params = ids
                .iter()
                .copied()
                .map(SqlValue::Integer)
                .collect::<Vec<_>>();
            removed = conn.execute(
                &format!("DELETE FROM rules WHERE id IN ({placeholders})"),
                params_from_iter(params.iter()),
            )?;
        }
        self.reload_locked(&conn)?;
        let stats = stats_locked(&conn)?;
        Ok(DeleteOutcome {
            removed,
            total: stats.total,
            stats,
            categories: category_counts_locked(&conn)?,
        })
    }

    pub fn delete_rules(&self, rules: &[NewRule]) -> DnsResult<DeleteOutcome> {
        let conn = self.connection()?;
        let mut removed = 0usize;

        {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare("DELETE FROM rules WHERE kind = ?1 AND rule = ?2")?;
                for entry in rules {
                    let (kind, rule) = canonicalize_rule(&entry.rule, entry.kind)?;
                    removed += stmt.execute(params![kind.as_str(), rule])?;
                }
            }
            tx.commit()?;
        }

        self.reload_locked(&conn)?;
        let stats = stats_locked(&conn)?;
        Ok(DeleteOutcome {
            removed,
            total: stats.total,
            stats,
            categories: category_counts_locked(&conn)?,
        })
    }

    pub fn clear(&self) -> DnsResult<DeleteOutcome> {
        let conn = self.connection()?;
        let removed = conn.execute("DELETE FROM rules", [])?;
        self.reload_locked(&conn)?;
        let stats = stats_locked(&conn)?;
        Ok(DeleteOutcome {
            removed,
            total: stats.total,
            stats,
            categories: category_counts_locked(&conn)?,
        })
    }

    fn connection(&self) -> DnsResult<std::sync::MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| DnsError::runtime("rule_db connection lock poisoned"))
    }

    fn reload_locked(&self, conn: &Connection) -> DnsResult<()> {
        let mut snapshot = RuleSnapshot::default();
        let source = format!("rule_db:{}", self.path.display());
        {
            let mut stmt = conn.prepare(
                "SELECT category, kind, rule FROM rules WHERE enabled = 1 ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (category, kind, rule) = row?;
                let matcher = snapshot.categories.entry(category.clone()).or_default();
                match RuleKind::parse(&kind) {
                    Some(RuleKind::Domain) => matcher
                        .domain
                        .add_expression(&rule, &source)
                        .map_err(|err| {
                            DnsError::plugin(format!("{err} in {source} (category '{category}')"))
                        })?,
                    Some(RuleKind::Ip) => matcher.ip.add_rule(&rule).map_err(|err| {
                        DnsError::plugin(format!(
                            "invalid rule_db IP rule '{rule}' in category '{category}': {err}"
                        ))
                    })?,
                    None => {
                        return Err(DnsError::plugin(format!(
                            "rule_db contains unsupported rule kind '{kind}'"
                        )));
                    }
                }
            }
        }
        for (category, matcher) in &mut snapshot.categories {
            matcher.domain.finalize().map_err(|err| {
                DnsError::plugin(format!(
                    "failed to compile rule_db domains in category '{category}': {err}"
                ))
            })?;
            matcher.ip.finalize_compact();
        }
        snapshot.stats = stats_locked(conn)?;
        self.snapshot.store(std::sync::Arc::new(snapshot));
        Ok(())
    }
}

pub(super) fn normalize_category(raw: &str) -> DnsResult<String> {
    let category = raw.trim().to_ascii_lowercase();
    if category.is_empty() {
        return Err(DnsError::config("rule category cannot be empty"));
    }
    if category.len() > MAX_CATEGORY_LEN {
        return Err(DnsError::config(format!(
            "rule category '{category}' is longer than {MAX_CATEGORY_LEN} bytes"
        )));
    }
    if !category
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
    {
        return Err(DnsError::config(format!(
            "rule category '{category}' may only contain ASCII letters, digits, '_' and '-'"
        )));
    }
    Ok(category)
}

pub(super) fn resolve_path(path: &str) -> DnsResult<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(DnsError::config("rule_db.path cannot be empty"));
    }
    let raw = PathBuf::from(trimmed);
    Ok(std::path::absolute(&raw).unwrap_or(raw))
}

fn open_database(path: &Path) -> DnsResult<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA auto_vacuum=INCREMENTAL;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=OFF;
         PRAGMA temp_store=DEFAULT;
         PRAGMA cache_size=-4096;
         PRAGMA mmap_size=0;",
    )?;
    Ok(conn)
}

fn create_schema(conn: &mut Connection) -> DnsResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let revision: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_revision'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    match revision.as_deref() {
        Some("1") => migrate_v1(conn)?,
        Some("2") => {}
        Some(other) => {
            return Err(DnsError::config(format!(
                "rule_db {} uses unsupported schema revision '{other}'",
                conn.path().unwrap_or_default()
            )));
        }
        None => {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS rules (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL,
                    rule TEXT NOT NULL,
                    category TEXT NOT NULL DEFAULT 'default',
                    source TEXT NOT NULL DEFAULT '',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    UNIQUE(kind, rule)
                );
                INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_revision', '2');",
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS rules_kind_idx ON rules(kind, id);
         CREATE INDEX IF NOT EXISTS rules_category_idx ON rules(category, id);
         CREATE INDEX IF NOT EXISTS rules_enabled_idx ON rules(enabled, id);",
    )?;
    Ok(())
}

/// Upgrade the original v1 schema. Existing rows keep the `default` category
/// and are re-imported with their configured categories on the next startup.
fn migrate_v1(conn: &mut Connection) -> DnsResult<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "ALTER TABLE rules ADD COLUMN category TEXT NOT NULL DEFAULT 'default'",
        [],
    )?;
    tx.execute_batch(
        "UPDATE meta SET value = '2' WHERE key = 'schema_revision';
         INSERT OR REPLACE INTO meta(key, value) VALUES ('category_backfill_pending', '1');",
    )?;
    tx.commit()?;
    Ok(())
}

fn backfill_pending(conn: &Connection) -> DnsResult<bool> {
    let pending: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'category_backfill_pending'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(pending.as_deref() == Some("1"))
}

fn should_import(conn: &Connection, config: &RuleDbConfig) -> DnsResult<bool> {
    if config.import.is_empty() {
        return Ok(false);
    }
    if backfill_pending(conn)? {
        return Ok(true);
    }
    match config.import_mode {
        RuleImportMode::Merge | RuleImportMode::Replace => Ok(true),
        RuleImportMode::IfEmpty => {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM rules", [], |row| row.get(0))?;
            Ok(count == 0)
        }
    }
}

fn import_rules(
    conn: &mut Connection,
    import: &[RuleImportEntry],
    mode: RuleImportMode,
) -> DnsResult<()> {
    let rules = load_import_rules(import)?;
    let now = now_ms();
    let tx = conn.transaction()?;
    if mode == RuleImportMode::Replace {
        tx.execute("DELETE FROM rules", [])?;
    }
    {
        // First import wins when the same rule appears in multiple categories;
        // rows added or edited through the API are never overwritten.
        let mut stmt = tx.prepare(
            "INSERT INTO rules(kind, rule, category, source, enabled, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, 'import', 1, ?4, ?4) \
             ON CONFLICT(kind, rule) DO UPDATE SET \
                category = excluded.category, updated_at_ms = excluded.updated_at_ms \
             WHERE rules.source = 'import' AND rules.category = 'default'",
        )?;
        for (category, kind, rule) in &rules {
            stmt.execute(params![kind.as_str(), rule, category, now])?;
        }
    }
    tx.execute(
        "DELETE FROM meta WHERE key = 'category_backfill_pending'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn load_import_rules(import: &[RuleImportEntry]) -> DnsResult<Vec<(String, RuleKind, String)>> {
    let mut rules = Vec::new();
    let mut seen = HashSet::new();

    for entry in import {
        let (category, sources) = match entry {
            RuleImportEntry::Plain(source) => {
                (DEFAULT_CATEGORY.to_string(), std::slice::from_ref(source))
            }
            RuleImportEntry::Group(group) => (
                normalize_category(&group.category)?,
                group.sources.as_slice(),
            ),
        };

        let mut inline = Vec::new();
        let mut files = Vec::new();
        for source in sources {
            let token = source.trim();
            if token.is_empty() {
                continue;
            }
            if token.starts_with('$') {
                return Err(DnsError::config(
                    "rule_db.import does not support $provider references; use inline rules or &file sources",
                ));
            }
            if let Some(path) = token.strip_prefix('&') {
                let path = path.trim();
                if path.is_empty() {
                    return Err(DnsError::config("rule_db.import file path cannot be empty"));
                }
                files.push(path.to_string());
            } else {
                inline.push(token.to_string());
            }
        }

        TextSource::new("rule_db.import", &inline, &files)
            .scan(&LineClassifier::new(&["#"]), |line| -> Result<(), String> {
                if line.annotations().blank || line.annotations().leading_comment.is_some() {
                    return Ok(());
                }
                let (kind, rule) = canonicalize_rule(line.trimmed(), None)
                    .map_err(|err| format!("{err} ({})", line.location()))?;
                let key = (kind, rule.clone());
                if seen.insert(key) {
                    rules.push((category.clone(), kind, rule));
                }
                Ok(())
            })
            .map_err(|err| {
                DnsError::config(format!("failed to load rule_db.import sources: {err}"))
            })?;
    }

    Ok(rules)
}

fn canonicalize_rule(raw: &str, requested: Option<RuleKind>) -> Result<(RuleKind, String), String> {
    let rule = raw.trim();
    if rule.is_empty() {
        return Err("rule cannot be empty".to_string());
    }
    if rule.starts_with('$') || rule.starts_with('&') {
        return Err(format!(
            "rule '{rule}' looks like a source reference; rule_db stores concrete rules only"
        ));
    }
    if rule.chars().any(char::is_whitespace) {
        return Err(format!("rule '{rule}' must not contain whitespace"));
    }

    let looks_like_ip = IpPrefixMatcher::classify_rule(rule).is_ok();
    let kind = match requested {
        Some(RuleKind::Ip) if !looks_like_ip => {
            return Err(format!("rule '{rule}' is not an IP address or CIDR prefix"));
        }
        Some(kind) => kind,
        None if looks_like_ip => RuleKind::Ip,
        None => RuleKind::Domain,
    };

    let stored = match kind {
        RuleKind::Ip => {
            let mut matcher = IpPrefixMatcher::default();
            matcher.add_rule(rule)?;
            rule.to_string()
        }
        RuleKind::Domain => {
            let mut matcher = DomainRuleMatcher::default();
            matcher.add_expression(rule, "rule_db")?;
            matcher.finalize()?;
            if split_domain_rule_expression(rule).0 == DomainRuleKind::Regexp {
                // Regexp expressions are case-sensitive by user intent.
                rule.to_string()
            } else {
                rule.to_lowercase()
            }
        }
    };
    Ok((kind, stored))
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuleRecord> {
    let kind: String = row.get(1)?;
    Ok(RuleRecord {
        id: row.get(0)?,
        kind: RuleKind::parse(&kind).unwrap_or(RuleKind::Domain),
        rule: row.get(2)?,
        category: row.get(3)?,
        source: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn stats_locked(conn: &Connection) -> rusqlite::Result<RuleStats> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN kind = 'domain' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN kind = 'ip' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN enabled = 1 THEN 1 ELSE 0 END), 0)
         FROM rules",
        [],
        |row| {
            let total: i64 = row.get(0)?;
            let domain: i64 = row.get(1)?;
            let ip: i64 = row.get(2)?;
            let enabled: i64 = row.get(3)?;
            Ok(RuleStats {
                total: total as usize,
                domain: domain as usize,
                ip: ip as usize,
                enabled: enabled as usize,
                disabled: total.saturating_sub(enabled) as usize,
            })
        },
    )
}

fn category_counts_locked(conn: &Connection) -> rusqlite::Result<Vec<CategoryCount>> {
    let mut stmt = conn.prepare(
        "SELECT category,
                COUNT(*),
                COALESCE(SUM(CASE WHEN kind = 'domain' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN kind = 'ip' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN enabled = 1 THEN 1 ELSE 0 END), 0)
         FROM rules GROUP BY category ORDER BY category ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CategoryCount {
            category: row.get(0)?,
            total: row.get::<_, i64>(1)? as usize,
            domain: row.get::<_, i64>(2)? as usize,
            ip: row.get::<_, i64>(3)? as usize,
            enabled: row.get::<_, i64>(4)? as usize,
        })
    })?;
    rows.collect()
}

fn where_sql(filters: &[&str]) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    }
}

fn like_pattern(keyword: &str) -> String {
    let mut pattern = String::with_capacity(keyword.len() + 2);
    pattern.push('%');
    for character in keyword.chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn normalize_source(source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        "api".to_string()
    } else {
        source.to_string()
    }
}

fn bool_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}
