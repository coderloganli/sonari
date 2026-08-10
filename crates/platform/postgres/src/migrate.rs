use anyhow::{Context, Result, bail};
use sqlx::{
    PgPool,
    migrate::{Migration, MigrationType, Migrator},
    postgres::PgPoolOptions,
    query_scalar,
};
use std::time::Duration;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};
use tokio::time;

use crate::PostgresConfig;

pub async fn connect(config: &PostgresConfig) -> Result<PgPool> {
    time::timeout(
        Duration::from_secs(5),
        PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.dsn),
    )
    .await
    .with_context(|| format!("timed out connecting postgres: {}", config.dsn))?
    .with_context(|| format!("failed to connect postgres: {}", config.dsn))
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let migrations =
        load_migrations(&migrations_path).context("failed to load postgres migrations")?;

    reject_legacy_applied_versions(pool, &migrations)
        .await
        .context("failed to validate existing postgres migration lineage")?;

    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
    .run(pool)
    .await
    .context("failed to run postgres migrations")
}

fn load_migrations(path: &Path) -> Result<Vec<Migration>> {
    let mut migrations = Vec::new();

    for entry in fs::read_dir(path)
        .with_context(|| format!("error reading migration directory {}", path.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "error reading contents of migration directory {}",
                path.display()
            )
        })?;
        let entry_path = entry.path();
        let metadata = fs::metadata(&entry_path).with_context(|| {
            format!(
                "error getting metadata of migration path {}",
                entry_path.display()
            )
        })?;

        if !metadata.is_file() {
            continue;
        }

        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let Some((version, description_part, migration_type)) =
            parse_migration_filename(&file_name)
        else {
            continue;
        };

        let sql = fs::read_to_string(&entry_path).with_context(|| {
            format!(
                "error reading contents of migration {}",
                entry_path.display()
            )
        })?;
        let description = description_part
            .trim_end_matches(migration_type.suffix())
            .replace('_', " ");
        let no_tx = sql.starts_with("-- no-transaction");

        migrations.push(Migration::new(
            version,
            Cow::Owned(description),
            migration_type,
            Cow::Owned(sql),
            no_tx,
        ));
    }

    migrations.sort_by_key(|migration| migration.version);
    Ok(migrations)
}

fn parse_migration_filename(file_name: &str) -> Option<(i64, &str, MigrationType)> {
    let parts = file_name.splitn(3, '_').collect::<Vec<_>>();

    if parts.len() == 3
        && parts[0].chars().all(|ch| ch.is_ascii_digit())
        && parts[1].chars().all(|ch| ch.is_ascii_digit())
        && parts[2].ends_with(".sql")
    {
        let version = format!("{}{}", parts[0], parts[1]).parse().ok()?;
        let migration_type = MigrationType::from_filename(parts[2]);
        return Some((version, parts[2], migration_type));
    }

    let parts = file_name.splitn(2, '_').collect::<Vec<_>>();
    if parts.len() != 2 || !parts[1].ends_with(".sql") {
        return None;
    }

    let version = parts[0].parse().ok()?;
    let migration_type = MigrationType::from_filename(parts[1]);
    Some((version, parts[1], migration_type))
}

async fn reject_legacy_applied_versions(pool: &PgPool, migrations: &[Migration]) -> Result<()> {
    let migrations_table: Option<String> =
        query_scalar("select to_regclass('public._sqlx_migrations')::text")
            .fetch_one(pool)
            .await
            .context("failed to resolve _sqlx_migrations table")?;

    if migrations_table.is_none() {
        return Ok(());
    }

    let legacy_versions =
        collect_legacy_versions(&Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))?;

    if legacy_versions.is_empty() {
        return Ok(());
    }

    let applied_legacy_versions: Vec<i64> = query_scalar(
        "select version from _sqlx_migrations where version = any($1) order by version",
    )
    .bind(&legacy_versions)
    .fetch_all(pool)
    .await
    .context("failed to read applied migration versions")?;

    if applied_legacy_versions.is_empty() {
        return Ok(());
    }

    let resolved_versions = migrations
        .iter()
        .map(|migration| migration.version.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    bail!(
        "detected legacy sqlx migration versions ({}) recorded in _sqlx_migrations while the current migration set resolves to unique combined versions ({}); manual migration lineage reconciliation is required before startup",
        applied_legacy_versions
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        resolved_versions
    );
}

fn collect_legacy_versions(path: &PathBuf) -> Result<Vec<i64>> {
    let mut versions = Vec::new();

    for entry in fs::read_dir(path)
        .with_context(|| format!("error reading migration directory {}", path.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "error reading contents of migration directory {}",
                path.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let parts = file_name.splitn(3, '_').collect::<Vec<_>>();
        if parts.len() == 3
            && parts[0].chars().all(|ch| ch.is_ascii_digit())
            && let Ok(version) = parts[0].parse::<i64>()
        {
            versions.push(version);
        }
    }

    versions.sort_unstable();
    versions.dedup();
    Ok(versions)
}
