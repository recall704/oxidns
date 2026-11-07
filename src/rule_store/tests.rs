// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::net::IpAddr;
use std::path::Path;

use super::{ListQuery, NewRule, RuleKind, RulePatch, open, reset_for_test};
use crate::config::types::{RuleDbConfig, RuleImportEntry, RuleImportGroup, RuleImportMode};
use crate::proto::Name;

fn db_config(path: &Path) -> RuleDbConfig {
    RuleDbConfig {
        path: path.display().to_string(),
        import: Vec::new(),
        import_mode: RuleImportMode::IfEmpty,
    }
}

fn plain(source: &str) -> RuleImportEntry {
    RuleImportEntry::Plain(source.to_string())
}

fn group(category: &str, sources: &[&str]) -> RuleImportEntry {
    RuleImportEntry::Group(RuleImportGroup {
        category: category.to_string(),
        sources: sources.iter().map(|source| source.to_string()).collect(),
    })
}

fn name(raw: &str) -> Name {
    Name::from_ascii(raw).expect("test name should parse")
}

fn ip(raw: &str) -> IpAddr {
    raw.parse().expect("test IP should parse")
}

fn rule(rule: &str) -> NewRule {
    NewRule {
        rule: rule.to_string(),
        kind: None,
        category: None,
        enabled: None,
    }
}

fn rule_in(rule: &str, category: &str) -> NewRule {
    NewRule {
        rule: rule.to_string(),
        kind: None,
        category: Some(category.to_string()),
        enabled: None,
    }
}

#[test]
fn startup_import_compiles_domain_and_ip_rules_by_category() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let database = dir.path().join("rules.db");
    let file = dir.path().join("extra.rules");
    std::fs::write(
        &file,
        "# extra rules\nkeyword:tracker\n203.0.113.0/24\n\nfull:only.example.com\n",
    )
    .expect("rule file");

    let store = open(&RuleDbConfig {
        path: database.display().to_string(),
        import: vec![
            group("ad", &[&format!("&{}", file.display())]),
            group("gfw", &["suffix.test"]),
        ],
        import_mode: RuleImportMode::IfEmpty,
    })
    .expect("store should open");

    let stats = store.snapshot_stats();
    assert_eq!(stats.total, 4);
    assert_eq!(stats.domain, 3);
    assert_eq!(stats.ip, 1);
    assert_eq!(stats.enabled, 4);

    let categories = store.categories().expect("categories should load");
    let ad = categories
        .iter()
        .find(|entry| entry.category == "ad")
        .expect("ad category");
    let gfw = categories
        .iter()
        .find(|entry| entry.category == "gfw")
        .expect("gfw category");
    assert_eq!(ad.total, 3);
    assert_eq!(ad.domain, 2);
    assert_eq!(ad.ip, 1);
    assert_eq!(gfw.total, 1);

    assert!(store.contains_name(&name("www.suffix.test")));
    assert!(store.contains_name(&name("cdn-tracker.net")));
    assert!(store.contains_name(&name("only.example.com")));
    assert!(!store.contains_name(&name("sub.only.example.com")));
    assert!(store.contains_ip(ip("203.0.113.9")));
    assert!(!store.contains_ip(ip("198.51.100.1")));

    // Category-filtered matching mirrors the provider's category allow-list.
    let ad_filter = vec!["ad".to_string()];
    let gfw_filter = vec!["gfw".to_string()];
    assert!(store.contains_name_in(&name("cdn-tracker.net"), Some(&ad_filter)));
    assert!(!store.contains_name_in(&name("www.suffix.test"), Some(&ad_filter)));
    assert!(store.contains_name_in(&name("www.suffix.test"), Some(&gfw_filter)));
    assert!(!store.contains_ip_in(ip("203.0.113.9"), Some(&gfw_filter)));
    assert!(store.contains_ip_in(ip("203.0.113.9"), Some(&ad_filter)));
}

#[test]
fn crud_round_trip_updates_snapshot_and_categories() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(&db_config(&dir.path().join("rules.db"))).expect("store should open");

    let added = store
        .add_rules(
            vec![
                rule_in("full:api.example.com", "ad"),
                rule("2001:db8::/32"),
                rule_in("full:api.example.com", "ad"),
            ],
            "api",
            true,
            "default",
        )
        .expect("rules should be added");
    assert_eq!(added.added, 2);
    assert_eq!(added.skipped, 1);
    assert_eq!(added.total, 2);
    assert!(
        added
            .categories
            .iter()
            .any(|entry| entry.category == "ad" && entry.total == 1)
    );
    assert!(
        added
            .categories
            .iter()
            .any(|entry| entry.category == "default" && entry.total == 1)
    );

    assert!(store.contains_name(&name("api.example.com")));
    assert!(store.contains_ip(ip("2001:db8::1")));

    let page = store
        .list(&ListQuery {
            kind: Some(RuleKind::Domain),
            ..Default::default()
        })
        .expect("list should work");
    assert_eq!(page.total, 1);
    assert_eq!(page.rules.len(), 1);
    assert_eq!(page.rules[0].category, "ad");
    let id = page.rules[0].id;

    let updated = store
        .update_rule(RulePatch {
            id,
            rule: Some("full:www.example.com".to_string()),
            kind: None,
            category: Some("cdn".to_string()),
            source: None,
            enabled: Some(false),
        })
        .expect("update should work")
        .expect("rule should exist");
    assert_eq!(updated.rule.rule, "full:www.example.com");
    assert_eq!(updated.rule.category, "cdn");
    assert!(!updated.rule.enabled);
    assert!(!store.contains_name(&name("www.example.com")));

    store
        .update_rule(RulePatch {
            id,
            rule: None,
            kind: None,
            category: None,
            source: None,
            enabled: Some(true),
        })
        .expect("enable should work")
        .expect("rule should exist");
    assert!(store.contains_name(&name("www.example.com")));

    let cdn = store
        .list(&ListQuery {
            category: Some("cdn".to_string()),
            ..Default::default()
        })
        .expect("category filter");
    assert_eq!(cdn.total, 1);
    assert_eq!(cdn.rules[0].id, id);

    store
        .update_rule(RulePatch {
            id,
            rule: Some("2001:db8::/32".to_string()),
            kind: None,
            category: None,
            source: None,
            enabled: None,
        })
        .expect_err("duplicate update should be rejected");

    let deleted = store.delete_ids(&[id]).expect("delete should work");
    assert_eq!(deleted.removed, 1);
    assert_eq!(deleted.total, 1);
    assert!(!store.contains_name(&name("www.example.com")));

    let by_rule = store
        .delete_rules(&[rule("2001:db8::/32")])
        .expect("delete by rule should work");
    assert_eq!(by_rule.removed, 1);
    assert_eq!(by_rule.total, 0);
    assert!(!store.contains_ip(ip("2001:db8::1")));

    store
        .add_rules(
            vec![rule("example.com"), rule("198.51.100.0/24")],
            "api",
            true,
            "default",
        )
        .expect("rules should be added");
    let cleared = store.clear().expect("clear should work");
    assert_eq!(cleared.removed, 2);
    assert_eq!(cleared.total, 0);
    assert!(!store.contains_name(&name("example.com")));
}

#[test]
fn import_modes_respect_existing_data_and_categories() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let database = dir.path().join("rules.db");
    let mut config = RuleDbConfig {
        path: database.display().to_string(),
        import: vec![group("ad", &["first.example.com"])],
        import_mode: RuleImportMode::IfEmpty,
    };

    let store = open(&config).expect("store should open");
    assert_eq!(store.snapshot_stats().total, 1);
    drop(store);

    config.import = vec![group("gfw", &["second.example.com"])];
    let store = open(&config).expect("store should reopen");
    assert_eq!(
        store.snapshot_stats().total,
        1,
        "if-empty must not re-import"
    );
    assert!(!store.contains_name(&name("second.example.com")));
    drop(store);

    config.import_mode = RuleImportMode::Merge;
    let store = open(&config).expect("store should merge");
    assert_eq!(store.snapshot_stats().total, 2);
    assert!(store.contains_name(&name("second.example.com")));
    let gfw = store
        .list(&ListQuery {
            category: Some("gfw".to_string()),
            ..Default::default()
        })
        .expect("category filter");
    assert_eq!(gfw.total, 1);
    drop(store);

    config.import_mode = RuleImportMode::Replace;
    config.import = vec![group("cn", &["third.example.com"])];
    let store = open(&config).expect("store should replace");
    assert_eq!(store.snapshot_stats().total, 1);
    assert!(store.contains_name(&name("third.example.com")));
    assert!(!store.contains_name(&name("first.example.com")));
    let categories = store.categories().expect("categories");
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].category, "cn");
}

#[test]
fn stores_are_shared_per_database_path() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let config = db_config(&dir.path().join("rules.db"));

    let first = open(&config).expect("first open");
    let second = open(&config).expect("second open");
    assert!(std::sync::Arc::ptr_eq(&first, &second));

    first
        .add_rules(vec![rule("shared.example.com")], "api", true, "default")
        .expect("add should work");
    assert!(second.contains_name(&name("shared.example.com")));
}

#[test]
fn invalid_rules_are_rejected_without_mutating_state() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(&db_config(&dir.path().join("rules.db"))).expect("store should open");

    for invalid in ["", "   ", "$provider", "&file.rules", "regexp:[", "a b"] {
        assert!(
            store
                .add_rules(vec![rule(invalid)], "api", true, "default")
                .is_err(),
            "rule {invalid:?} should be rejected"
        );
    }
    assert!(
        store
            .add_rules(
                vec![NewRule {
                    rule: "1.2.3.0/24".to_string(),
                    kind: Some(RuleKind::Domain),
                    category: None,
                    enabled: None,
                }],
                "api",
                true,
                "default"
            )
            .is_ok(),
        "an explicit domain kind is authoritative"
    );
    assert!(
        store
            .add_rules(vec![rule("example.com")], "api", true, "Bad Category")
            .is_err(),
        "invalid categories must be rejected"
    );
    assert_eq!(store.snapshot_stats().total, 1);
}

#[test]
fn list_supports_filters_and_pagination() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(&db_config(&dir.path().join("rules.db"))).expect("store should open");

    store
        .add_rules(
            vec![
                rule_in("ads.example.com", "ad"),
                rule_in("tracker.example.net", "gfw"),
                rule_in("198.51.100.0/24", "china"),
                rule_in("2001:db8::/32", "china"),
            ],
            "api",
            true,
            "default",
        )
        .expect("add should work");

    let page = store
        .list(&ListQuery {
            limit: 2,
            ..Default::default()
        })
        .expect("list should work");
    assert_eq!(page.total, 4);
    assert_eq!(page.rules.len(), 2);
    assert_eq!(page.categories.len(), 3);
    let cursor = page.next_cursor.expect("next page expected");

    let next = store
        .list(&ListQuery {
            cursor: Some(cursor),
            limit: 2,
            ..Default::default()
        })
        .expect("second page");
    assert_eq!(next.rules.len(), 2);
    assert!(next.next_cursor.is_none());

    let ips = store
        .list(&ListQuery {
            kind: Some(RuleKind::Ip),
            ..Default::default()
        })
        .expect("ip filter");
    assert_eq!(ips.total, 2);

    let ads = store
        .list(&ListQuery {
            keyword: Some("ADS".to_string()),
            ..Default::default()
        })
        .expect("keyword filter");
    assert_eq!(ads.total, 1);
    assert_eq!(ads.rules[0].rule, "ads.example.com");
    assert_eq!(ads.rules[0].category, "ad");

    let china = store
        .list(&ListQuery {
            category: Some("CHINA".to_string()),
            ..Default::default()
        })
        .expect("category filter");
    assert_eq!(china.total, 2);
    assert!(china.rules.iter().all(|rule| rule.category == "china"));
}

#[test]
fn config_import_accepts_grouped_categories() {
    let config: RuleDbConfig = serde_yaml_ng::from_str(
        r#"
path: ./rules.db
import:
  - category: ad
    sources:
      - "&./ads.txt"
      - inline-ad.example
  - "legacy.example"
import_mode: if-empty
"#,
    )
    .expect("config should parse");
    assert_eq!(config.import.len(), 2);
    assert!(matches!(&config.import[0], RuleImportEntry::Group(group) if group.category == "ad"));
    assert!(
        matches!(&config.import[1], RuleImportEntry::Plain(source) if source == "legacy.example")
    );
}

#[test]
fn config_import_rejects_provider_references() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let err = open(&RuleDbConfig {
        path: dir.path().join("rules.db").display().to_string(),
        import: vec![plain("$domain_set")],
        import_mode: RuleImportMode::IfEmpty,
    })
    .expect_err("provider references must be rejected");
    assert!(err.to_string().contains("$provider"));
}

#[test]
fn v1_schema_migrates_and_backfills_categories() {
    reset_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let database = dir.path().join("legacy.db");

    {
        let conn = rusqlite::Connection::open(&database).expect("legacy database");
        conn.execute_batch(
            "CREATE TABLE rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                rule TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                UNIQUE(kind, rule)
            );
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO meta VALUES ('schema_revision', '1');
            INSERT INTO rules(kind, rule, source, enabled, created_at_ms, updated_at_ms)
                VALUES ('domain', 'legacy.example', 'import', 1, 1, 1);",
        )
        .expect("create v1 schema");
    }

    let store = open(&RuleDbConfig {
        path: database.display().to_string(),
        import: vec![group("ad", &["legacy.example"])],
        import_mode: RuleImportMode::IfEmpty,
    })
    .expect("v1 database should migrate");

    let categories = store.categories().expect("categories");
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].category, "ad");
    assert_eq!(categories[0].total, 1);
    assert!(store.contains_name(&name("legacy.example")));
}
