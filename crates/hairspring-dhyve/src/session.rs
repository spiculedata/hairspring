//! A session: one host's settings, with the parts shared by a prepare and a trial: taking the host lock, waiting for a VM
//! while watching its logs, and a guard that owns a started VM so that it is ended on every path, a panic included.
use crate::bounded::reap_within;
use crate::director::{Director, Progress, Report};
use crate::error::{InfraError, InfraKind};
use crate::host::{HostConfig, Timings};
use crate::hostlock::{HostLock, LockError};
use crate::layout::{RunPaths, StateLayout};
use crate::logs::{len_now, Bounds, Breach};
use crate::signal::Terminator;
use crate::spawn::{SpawnLog, Spawner};
use crate::vmname::{Stamp, VmName};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
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
    pub(crate) layout: StateLayout,
    /// Keep every log, input file and the prepare's run directory, in full.
    pub(crate) keep_all: bool,
    pub(crate) terminator: Terminator,
    /// When set, a second prepare in this session is refused.
    pub(crate) prepares: Option<Arc<AtomicU32>>,
    /// Told of each root command, for a steps log.
    pub(crate) spawn_log: Option<Arc<dyn SpawnLog>>,
    #[cfg(feature = "fake-vmm")]
    pub(crate) panic_in_wait: bool,
}

impl Session {
    pub fn new(host: HostConfig, timings: Timings, bounds: Bounds) -> Session {
        Session {
            layout: StateLayout::new(&host.state_dir, host.session_stamp.is_some()),
            host,
            timings,
            bounds,
            keep_all: false,
            terminator: Terminator::System,
            prepares: None,
            spawn_log: None,
            #[cfg(feature = "fake-vmm")]
            panic_in_wait: false,
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

    /// The same session keeping every log and input file in full.
    pub fn with_keep_all(&self) -> Session {
        Session {
            keep_all: true,
            ..self.clone()
        }
    }

    /// The same session refusing a second prepare, whichever copy of it asks.
    pub fn with_one_prepare(&self) -> Session {
        Session {
            prepares: Some(Arc::new(AtomicU32::new(0))),
            ..self.clone()
        }
    }

    /// The same session telling `log` of each root command it runs.
    pub fn with_spawn_log(&self, log: Arc<dyn SpawnLog>) -> Session {
        Session {
            spawn_log: Some(log),
            ..self.clone()
        }
    }

    /// The same session ending VMs through `terminator`. Tests stand in one that is always refused, as a root process under
    /// `sudo` refuses an unprivileged caller.
    pub fn with_terminator(&self, terminator: Terminator) -> Session {
        Session {
            terminator,
            ..self.clone()
        }
    }

    /// The same session that panics in the first wait after a VM start, so the cleanup on unwinding can be tested.
    #[cfg(feature = "fake-vmm")]
    pub fn with_panic_in_wait(&self) -> Session {
        Session {
            panic_in_wait: true,
            ..self.clone()
        }
    }

    pub(crate) fn spawner(&self, stamp: &Stamp) -> Spawner {
        Spawner::new(&self.host, self.timings, stamp).with_log(self.spawn_log.clone())
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

    /// Counts a prepare; refuses the second when the session allows only one.
    pub(crate) fn count_prepare(&self) -> Result<(), InfraError> {
        match &self.prepares {
            Some(n) if n.fetch_add(1, Ordering::SeqCst) >= 1 => {
                Err(InfraError::bare(InfraKind::SecondPrepare))
            }
            _ => Ok(()),
        }
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
    pub(crate) fn check_space(
        &self,
        spawner: &Spawner,
        checkpoint_bytes: u64,
    ) -> Result<(), InfraError> {
        let free = spawner.free_kib(&self.host.state_dir)?;
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

    /// Starts a VM and returns the guard that owns it.
    pub(crate) fn start_guarded<'a>(
        &'a self,
        spawner: Spawner,
        name: VmName,
        args: &[String],
        run: &RunPaths,
        director: Director,
    ) -> Result<VmGuard<'a>, InfraError> {
        let child = match spawner.start_vm(args, &run.run_log) {
            Ok(c) => c,
            Err(e) => {
                director.cancel();
                director.finish(&run.control_sock);
                return Err(e);
            }
        };
        Ok(VmGuard {
            session: self,
            spawner,
            name,
            sock: run.control_sock.clone(),
            child: Some(child),
            director: Some(director),
        })
    }
}

/// Owns a started VM, its starter process and the control endpoint. [`VmGuard::finish`] ends them and says how it went;
/// dropping the guard, including by a panic, ends them as well as it can, so no VM, child process or control socket is
/// left behind.
pub(crate) struct VmGuard<'a> {
    session: &'a Session,
    spawner: Spawner,
    name: VmName,
    sock: PathBuf,
    child: Option<Child>,
    director: Option<Director>,
}

impl VmGuard<'_> {
    pub(crate) fn wait(&mut self, w: &Watch) -> Ended {
        #[cfg(feature = "fake-vmm")]
        if self.session.panic_in_wait {
            panic!("a test asked for a panic while waiting for the VM");
        }
        match (self.child.as_mut(), self.director.as_ref()) {
            (Some(child), Some(director)) => self.session.wait(child, director, w),
            _ => Ended::Exited(None),
        }
    }

    /// Ends the VM, then the endpoint. The VM is destroyed once (the one destroy after its start). A VM that is still
    /// there after a grace is sent SIGTERM through its group, then SIGKILL; every wait has a deadline, and a VM that
    /// will not go is an error and not a wait.
    fn end_vm(&mut self) -> Result<Option<i32>, InfraError> {
        let grace = self.session.timings.kill_grace;
        let terminator = self.session.terminator;
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let on_its_own = matches!(child.try_wait(), Ok(Some(_)));
        let destroyed = self.spawner.destroy(&self.name);
        if !on_its_own {
            let mut gone = reap_within(child, grace);
            let needed_a_signal = !gone;
            if !gone {
                terminator.terminate_group(child);
                gone = reap_within(child, grace);
            }
            if !gone {
                terminator.kill_child(child);
                gone = reap_within(child, grace);
            }
            if let Some(d) = &self.director {
                d.vm_exited();
            }
            if !gone {
                return Err(InfraError::bare(InfraKind::VmDidNotExit));
            }
            // The destroy command failed while the VM was alive, and the VM went only when it was signalled.
            if needed_a_signal && !matches!(destroyed, Ok(Some(0))) {
                return Err(InfraError::new(
                    InfraKind::VmDidNotExit,
                    "the destroy command did not succeed",
                ));
            }
        }
        let code = child.wait().ok().and_then(|s| s.code());
        if let Some(d) = &self.director {
            d.vm_exited();
        }
        self.spawner.tell_ended(&self.name, code);
        Ok(code)
    }

    /// Ends the VM and the endpoint and returns the VM's exit status and what the endpoint saw.
    pub(crate) fn finish(mut self) -> Result<(Option<i32>, Report), InfraError> {
        let ended = self.end_vm();
        let director = self.director.take();
        let report = director.map(|d| {
            if !d.is_finished() {
                d.cancel();
            }
            d.finish(&self.sock)
        });
        self.child = None;
        match (ended, report) {
            (Ok(code), Some(report)) => Ok((code, report)),
            (Err(e), _) => Err(e),
            (Ok(_), None) => Err(InfraError::bare(InfraKind::ControlLost)),
        }
    }
}

impl Drop for VmGuard<'_> {
    fn drop(&mut self) {
        if self.child.is_none() && self.director.is_none() {
            return;
        }
        let _ = self.end_vm();
        if let Some(d) = self.director.take() {
            d.cancel();
            d.finish(&self.sock);
        }
    }
}
