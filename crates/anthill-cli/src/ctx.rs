use anthill_engine::config::Config;
use anthill_engine::engine::Engine;
use anthill_engine::erased::Registry;
use anthill_service::Service;
use anthill_store::{PgStore, SqliteStore, Store};

const DATABASE_URL_ENV: &str = "ANTHILL_DATABASE_URL";
const DATABASE_SCHEMA_ENV: &str = "ANTHILL_DATABASE_SCHEMA";
const POOL_SIZE: u32 = 8;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Ctx {
    pub engine: Engine,
    pub db: PathBuf,
}

pub type CmdResult<T> = Result<T, Box<dyn std::error::Error>>;

impl Ctx {
    pub fn load(registry: Registry, config_path: &Path, db: PathBuf) -> CmdResult<Ctx> {
        let registry = Arc::new(registry);
        let config = if config_path.exists() {
            let text = std::fs::read_to_string(config_path)
                .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
            Config::parse(&text)?.with_registry_defaults(&registry)
        } else {
            Config::from_registry(&registry)
        };
        let engine = crate::commands::build_engine(registry.clone(), config);
        Ok(Ctx { engine, db })
    }

    /// The results store: PostgreSQL when `ANTHILL_DATABASE_URL` is set (or `--db` is a postgres URL),
    /// otherwise a local SQLite file. URLs carry passwords, so they are never printed.
    pub fn store(&self) -> CmdResult<Arc<dyn Store>> {
        let from_env = std::env::var(DATABASE_URL_ENV)
            .ok()
            .filter(|u| !u.is_empty());
        let db_arg = self.db.to_string_lossy();
        let url = from_env.or_else(|| {
            (db_arg.starts_with("postgres://") || db_arg.starts_with("postgresql://"))
                .then(|| db_arg.to_string())
        });
        if let Some(url) = url {
            let schema = std::env::var(DATABASE_SCHEMA_ENV)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "public".to_string());
            let store = PgStore::connect_in_schema(&url, &schema, POOL_SIZE)
                .map_err(|e| format!("cannot use the PostgreSQL database: {e}"))?;
            return Ok(Arc::new(store));
        }
        if let Some(parent) = self.db.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        Ok(Arc::new(SqliteStore::open(&self.db)?))
    }

    pub fn service(&self) -> CmdResult<Service> {
        Ok(Service::new(self.store()?))
    }
}
