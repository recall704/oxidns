// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite-backed rule database.
//!
//! The rule database is the mutable counterpart of the read-only provider
//! plugins: rules are imported from configuration sources at startup, then
//! managed at runtime through the management API and the WebUI rule page.
//!
//! Design constraints:
//!
//! - the request path never touches SQLite; providers read an immutable
//!   [`arc_swap::ArcSwap`] snapshot compiled from the enabled rules;
//! - mutations are serialized by one connection mutex, persist before the
//!   snapshot is replaced, and rebuild the snapshot exactly once per write;
//! - stores are shared per canonical database path, so a top-level `rule_db`
//!   and a `sqlite_rule_set` provider configured with the same file observe the
//!   same data and snapshots.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock, Weak};

use crate::config::types::{RuleDbConfig, RuleImportMode};
use crate::infra::error::Result as DnsResult;

#[cfg(feature = "api")]
pub mod api;
mod store;

#[cfg(test)]
mod tests;

pub use store::{
    AddOutcome, CategoryCount, DEFAULT_CATEGORY, DEFAULT_LIST_LIMIT, DeleteOutcome, ListPage,
    ListQuery, MAX_LIST_LIMIT, NewRule, RuleKind, RulePatch, RuleRecord, RuleStats, RuleStore,
    UpdateOutcome,
};

/// Validate and normalize a category identifier (`ad`, `gfw`, `china`, ...).
pub fn normalize_category(raw: &str) -> DnsResult<String> {
    store::normalize_category(raw)
}

static STORES: LazyLock<Mutex<HashMap<PathBuf, Weak<RuleStore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static GLOBAL: LazyLock<RwLock<Option<Arc<RuleStore>>>> = LazyLock::new(|| RwLock::new(None));

/// Open the process-wide rule database configured by the top-level `rule_db`
/// section and remember it as the default store for `sqlite_rule_set`
/// providers that do not set a path.
pub fn init_global(config: &RuleDbConfig) -> DnsResult<Option<Arc<RuleStore>>> {
    if !config.is_enabled() {
        close_global();
        return Ok(None);
    }
    let store = open(config)?;
    set_global(store.clone())?;
    let stats = store.snapshot_stats();
    let categories = store.categories().map(|entries| entries.len()).unwrap_or(0);
    tracing::info!(
        database = %store.path().display(),
        rules = stats.total,
        enabled = stats.enabled,
        categories,
        "rule_db initialized"
    );
    Ok(Some(store))
}

/// Return the store opened by [`init_global`], if any.
pub fn global() -> Option<Arc<RuleStore>> {
    GLOBAL.read().ok().and_then(|store| store.clone())
}

/// Forget the global store without closing databases that are still shared by
/// provider instances.
pub fn close_global() {
    if let Ok(mut store) = GLOBAL.write() {
        *store = None;
    }
}

/// Open (or reuse) a rule database for `config.path`.
pub fn open(config: &RuleDbConfig) -> DnsResult<Arc<RuleStore>> {
    let key = store::resolve_path(&config.path)?;
    if let Some(existing) = lookup(&key) {
        return Ok(existing);
    }
    let store = RuleStore::open(config)?;
    shared_store(store)
}

/// Open (or reuse) a rule database by path without importing configuration
/// rules.
pub fn open_path(path: &str) -> DnsResult<Arc<RuleStore>> {
    open(&RuleDbConfig {
        path: path.to_string(),
        import: Vec::new(),
        import_mode: RuleImportMode::IfEmpty,
    })
}

fn set_global(store: Arc<RuleStore>) -> DnsResult<()> {
    let mut slot = GLOBAL
        .write()
        .map_err(|_| crate::infra::error::DnsError::runtime("rule_db global lock poisoned"))?;
    *slot = Some(store);
    Ok(())
}

fn lookup(key: &PathBuf) -> Option<Arc<RuleStore>> {
    STORES
        .lock()
        .ok()
        .and_then(|stores| stores.get(key).and_then(Weak::upgrade))
}

fn shared_store(store: RuleStore) -> DnsResult<Arc<RuleStore>> {
    let key = store.path().to_path_buf();
    let mut stores = STORES
        .lock()
        .map_err(|_| crate::infra::error::DnsError::runtime("rule_db registry lock poisoned"))?;

    if let Some(existing) = stores.get(&key).and_then(Weak::upgrade) {
        return Ok(existing);
    }

    let store = Arc::new(store);
    stores.insert(key, Arc::downgrade(&store));
    Ok(store)
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    close_global();
    if let Ok(mut stores) = STORES.lock() {
        stores.clear();
    }
}
