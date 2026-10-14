use anthill_engine::config::Config;
use anthill_engine::engine::Engine;
use anthill_engine::erased::Registry;
use anthill_service::Service;
use anthill_store::{SqliteStore, Store};
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

    pub fn store(&self) -> CmdResult<Arc<dyn Store>> {
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
