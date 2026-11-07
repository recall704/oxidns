// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read-only provider backed by the SQLite rule database.
//!
//! The provider shares one [`crate::rule_store::RuleStore`] with the top-level
//! `rule_db` section when both point at the same file, so rules added or edited
//! through the management API become visible to matchers immediately after the
//! store publishes its new snapshot.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::info;

use crate::config::types::{PluginConfig, RuleDbConfig, RuleImportEntry, RuleImportMode};
use crate::infra::error::{DnsError, Result as DnsResult};
use crate::plugin::provider::Provider;
use crate::plugin::{Plugin, PluginFactory, UninitializedPlugin};
use crate::plugin_factory;
use crate::proto::{Name, Question};
use crate::rule_store::{RuleStore, normalize_category, open as open_rule_store};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SqliteRuleSetArgs {
    /// Optional database path. An empty path reuses the top-level `rule_db`.
    #[serde(default)]
    path: String,
    /// One-shot import sources applied when the database is empty.
    #[serde(default)]
    import: Vec<RuleImportEntry>,
    #[serde(default)]
    import_mode: RuleImportMode,
    /// Optional category allow-list. Empty means every category.
    #[serde(default)]
    categories: Vec<String>,
}

impl SqliteRuleSetArgs {
    fn from_plugin_config(plugin_config: &PluginConfig) -> DnsResult<Self> {
        match plugin_config.args.clone() {
            Some(args) => serde_yaml_ng::from_value::<Self>(args).map_err(|err| {
                DnsError::plugin(format!("failed to parse sqlite_rule_set config: {err}"))
            }),
            None => Ok(Self::default()),
        }
    }

    fn into_rule_db(self) -> RuleDbConfig {
        RuleDbConfig {
            path: self.path,
            import: self.import,
            import_mode: self.import_mode,
        }
    }
}

/// Provider handle registered in the plugin registry.
#[derive(Debug)]
pub struct SqliteRuleSet {
    tag: String,
    config: RuleDbConfig,
    categories: Vec<String>,
    store: Option<Arc<RuleStore>>,
}

#[async_trait]
impl Plugin for SqliteRuleSet {
    fn tag(&self) -> &str {
        &self.tag
    }

    async fn init(&mut self, _context: &crate::plugin::PluginInitContext<'_>) -> DnsResult<()> {
        let config = self.config.clone();
        let store = if config.is_enabled() {
            open_rule_store(&config)?
        } else {
            crate::rule_store::global().ok_or_else(|| {
                DnsError::plugin(format!(
                    "sqlite_rule_set '{}' requires args.path or a top-level rule_db.path",
                    self.tag
                ))
            })?
        };
        info!(
            tag = %self.tag,
            database = %store.path().display(),
            rules = store.snapshot_stats().total,
            categories = ?self.categories,
            "sqlite_rule_set initialized"
        );
        #[cfg(feature = "api")]
        crate::rule_store::api::register_plugin(&self.tag, &store)?;
        self.store = Some(store);
        Ok(())
    }

    async fn destroy(&self) -> DnsResult<()> {
        Ok(())
    }
}

impl SqliteRuleSet {
    fn store(&self) -> DnsResult<&Arc<RuleStore>> {
        self.store
            .as_ref()
            .ok_or_else(|| DnsError::runtime("sqlite_rule_set provider is not initialized"))
    }

    fn category_filter(&self) -> Option<&[String]> {
        if self.categories.is_empty() {
            None
        } else {
            Some(self.categories.as_slice())
        }
    }
}

#[async_trait]
impl Provider for SqliteRuleSet {
    fn as_any(&self) -> &dyn Any {
        self
    }

    #[inline]
    #[hotpath::measure]
    fn contains_name(&self, name: &Name) -> bool {
        self.store
            .as_ref()
            .is_some_and(|store| store.contains_name_in(name, self.category_filter()))
    }

    #[inline]
    #[hotpath::measure]
    fn contains_question(&self, question: &Question) -> bool {
        self.contains_name(question.name())
    }

    #[inline]
    fn contains_ip(&self, ip: std::net::IpAddr) -> bool {
        self.store
            .as_ref()
            .is_some_and(|store| store.contains_ip_in(ip, self.category_filter()))
    }

    async fn reload(&self) -> DnsResult<()> {
        self.store()?.reload()
    }

    fn supports_ip_matching(&self) -> bool {
        true
    }

    fn supports_domain_matching(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
#[plugin_factory("sqlite_rule_set")]
pub struct SqliteRuleSetFactory;

impl PluginFactory for SqliteRuleSetFactory {
    fn create(
        &self,
        plugin_config: &PluginConfig,
        _init_context: &crate::plugin::PluginInitContext<'_>,
    ) -> DnsResult<UninitializedPlugin> {
        let args = SqliteRuleSetArgs::from_plugin_config(plugin_config)?;
        let mut categories = Vec::with_capacity(args.categories.len());
        for category in &args.categories {
            categories.push(normalize_category(category)?);
        }
        let config = args.into_rule_db();
        Ok(UninitializedPlugin::Provider(Box::new(SqliteRuleSet {
            tag: plugin_config.tag.clone(),
            config,
            categories,
            store: None,
        })))
    }
}
