//! Builds the environment identity for a run: the host, the Hermit executable and the guest's inputs.
use crate::env::guest_env;
use crate::exec::{resolve_bin, ProcessSpec};
use crate::hermit_version;
use crate::host_identity::{self, GuestInputs, HermitIdentity, HostSources};
use hairspring_core::environment::{sha256_file, EnvironmentIdentity};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

pub struct EnvironmentProbe {
    sources: HostSources,
    /// `hermit --version` answers, by the hash of the executable that gave them, so an upgrade in place is asked again.
    /// Only answers are kept: a probe that timed out is retried next time.
    versions: Mutex<BTreeMap<String, String>>,
}

impl EnvironmentProbe {
    pub fn new(sources: HostSources) -> Self {
        EnvironmentProbe {
            sources,
            versions: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn with_sources(self, sources: HostSources) -> Self {
        EnvironmentProbe { sources, ..self }
    }

    fn hermit(&self, configured: &Path) -> HermitIdentity {
        let bin = resolve_bin(configured);
        let Ok(sha256) = sha256_file(&bin) else {
            return HermitIdentity::default();
        };
        let cached = self
            .versions
            .lock()
            .ok()
            .and_then(|map| map.get(&sha256).cloned());
        let version = cached.or_else(|| {
            let asked = hermit_version::probe(&bin)?;
            if let Ok(mut map) = self.versions.lock() {
                map.insert(sha256.clone(), asked.clone());
            }
            Some(asked)
        });
        HermitIdentity {
            sha256: Some(sha256),
            version,
        }
    }

    /// The environment this run would record, or that a replay compares against.
    pub fn identity(
        &self,
        hermit_bin: &Path,
        extra_env_names: &[String],
        p: &ProcessSpec,
    ) -> EnvironmentIdentity {
        let env = guest_env(p.command, p.env);
        host_identity::collect(
            &self.sources,
            &self.hermit(hermit_bin),
            &GuestInputs {
                command: p.command,
                cwd: p.cwd,
                env: &env,
                extra_env_names,
                hermit_args: p.hermit_args,
            },
        )
    }
}
