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
            if from_arg_with_password(&db_arg, &url) {
                eprintln!("anthill: warning: the database password is visible in the process list; set {DATABASE_URL_ENV} instead of passing it with --db");
            }
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

/// True when the URL came from the command line (visible in `ps`) and embeds a password.
fn from_arg_with_password(db_arg: &str, url: &str) -> bool {
    if db_arg != url {
        return false;
    }
    let authority = url
        .split_once("://")
        .map_or("", |(_, rest)| rest.split(['/', '?']).next().unwrap_or(""));
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::from_arg_with_password;

    #[test]
    fn warns_only_for_passwords_given_on_the_command_line() {
        let url = "postgres://anthill:s3cret@db/anthill";
        assert!(from_arg_with_password(url, url));
        assert!(
            !from_arg_with_password("results.db", url),
            "the URL came from the environment"
        );
        let no_pw = "postgres://anthill@db/anthill";
        assert!(!from_arg_with_password(no_pw, no_pw));
        let at_in_path = "postgres://db/anthill?x=a@b:c";
        assert!(!from_arg_with_password(at_in_path, at_in_path));
    }
}
