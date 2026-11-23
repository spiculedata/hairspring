//! A session: one host's settings, with the parts shared by a prepare and a trial: taking the host lock, waiting for a VM
//! while watching its logs, and ending a VM that is still there.
use crate::director::{Director, Progress};
use crate::error::{InfraError, InfraKind};
use crate::host::{HostConfig, Timings};
use crate::hostlock::{HostLock, LockError};
use crate::layout::{RunPaths, StateLayout};
use crate::logs::{len_now, Bounds, Breach};
use crate::spawn::Spawner;
use crate::vmname::VmName;
use std::path::Path;
use std::process::Child;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// How often a waiting loop looks at the VM and the endpoint.
const LOOK_EVERY: Duration = Duration::from_millis(10);

/// Why a wait for a VM ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The VM exited, with this status (`None` for a signal).
    Exited(Option<i32>),
    /// The time cap passed.
    Cap,
    /// A log passed its bound.
    Breach(Breach),
    /// The control endpoint ended without doing its task.
    EndpointFailed,
}

/// What a wait watches.
pub struct Watch<'a> {
    pub cap: Duration,
    pub bounds: &'a Bounds,
    pub run_log: &'a Path,
    pub uart_log: &'a Path,
    pub every: Duration,
    /// Stop as soon as the endpoint has failed, without waiting for the VM.
    pub stop_on_failure: bool,
    /// Once the endpoint is done, how long the VM has to exit; `None` waits for the cap.
    pub grace_after_done: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub(crate) host: HostConfig,
    pub(crate) timings: Timings,
    pub(crate) bounds: Bounds,
    pub(crate) spawner: Spawner,
    pub(crate) layout: StateLayout,
}

impl Session {
    pub fn new(host: HostConfig, timings: Timings, bounds: Bounds) -> Session {
        Session {
            spawner: Spawner::new(&host, timings),
            layout: StateLayout::new(&host.state_dir),
            host,
            timings,
            bounds,
        }
    }

    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    /// The same session with trials cut off after `cap`.
    pub fn with_trial_cap(&self, cap: Duration) -> Session {
        Session {
            timings: Timings {
                trial_cap: cap,
                ..self.timings
            },
            ..self.clone()
        }
    }

    /// Takes the host lock for one prepare or one trial.
    pub(crate) fn lock(&self) -> Result<HostLock, InfraError> {
        HostLock::acquire(
            &self.host.state_dir,
            self.timings.lock_wait,
            Duration::from_millis(20),
        )
        .map_err(|e| match e {
            LockError::Busy => InfraError::bare(InfraKind::HostBusy),
            LockError::Io(m) => InfraError::new(InfraKind::FileOperation, m),
        })
    }

    /// Makes the directories a VM run needs and checks its control socket path fits.
    pub(crate) fn make_dirs(&self, run: &RunPaths) -> Result<(), InfraError> {
        if !run.socket_path_fits() {
            return Err(InfraError::new(
                InfraKind::Settings,
                "the state directory path is too long",
            ));
        }
        std::fs::create_dir_all(&run.dir)
            .map_err(|e| InfraError::io("create the run directory", &e))
    }

    /// Checks there is room for the logs and a restore.
    pub(crate) fn check_space(&self, checkpoint_bytes: u64) -> Result<(), InfraError> {
        let free = self.spawner.free_kib(&self.host.state_dir)?;
        let need = self.bounds.required_free_kib(checkpoint_bytes);
        if free < need {
            return Err(InfraError::new(
                InfraKind::LowSpace,
                format!("{free} KiB free, {need} KiB needed"),
            ));
        }
        Ok(())
    }

    /// Waits for `child`, looking at the endpoint and the log sizes.
    pub(crate) fn wait(&self, child: &mut Child, director: &Director, w: &Watch) -> Ended {
        let started = Instant::now();
        let mut last_look = Instant::now();
        let mut done_at: Option<Instant> = None;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ended::Exited(status.code()),
                Ok(None) => {}
                Err(_) => return Ended::Exited(None),
            }
            if started.elapsed() >= w.cap {
                return Ended::Cap;
            }
            match director.progress() {
                Progress::Failed if w.stop_on_failure => return Ended::EndpointFailed,
                Progress::Done if done_at.is_none() => done_at = Some(Instant::now()),
                _ => {}
            }
            if let (Some(at), Some(grace)) = (done_at, w.grace_after_done) {
                if at.elapsed() >= grace {
                    return Ended::Cap;
                }
            }
            if last_look.elapsed() >= w.every {
                last_look = Instant::now();
                if let Some(breach) = w.bounds.breach(len_now(w.run_log), len_now(w.uart_log)) {
                    return Ended::Breach(breach);
                }
            }
            sleep(LOOK_EVERY);
        }
    }

    /// Ends the VM `name` if its process is still there: destroy it, give it a moment to exit, and only then end the
    /// process that started it. Returns the exit status when it is known.
    pub(crate) fn end_vm(
        &self,
        name: &VmName,
        child: &mut Child,
        director: &Director,
    ) -> Result<Option<i32>, InfraError> {
        let mut code = None;
        if matches!(child.try_wait(), Ok(None)) {
            let destroyed = self.spawner.destroy(name);
            let until = Instant::now() + self.timings.kill_grace;
            while Instant::now() < until && matches!(child.try_wait(), Ok(None)) {
                sleep(LOOK_EVERY);
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            destroyed?;
        }
        if let Ok(status) = child.wait() {
            code = status.code();
        }
        director.vm_exited();
        self.spawner.destroy(name)?;
        Ok(code)
    }
}
