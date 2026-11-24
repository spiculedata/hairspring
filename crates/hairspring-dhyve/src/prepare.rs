//! Prepare: one cold boot of the guest to its ready stop, a checkpoint taken there, and a shutdown. The result is the
//! checkpoint record every restore starts from.
//!
//! The host lock is held for the whole of it. Anything that goes wrong is an infrastructure error and no checkpoint
//! record is returned.
use crate::checkpoint::{CheckpointRecord, RecordError, RestoreKeys};
use crate::director::{CloseWhy, ControlError, Director, DirectorSpec, Mode, Report};
use crate::error::{InfraError, InfraKind};
use crate::fwcfg::{Files, Inputs};
use crate::host::Timings;
use crate::layout::RunPaths;
use crate::logs::{keep_tail, len_now};
use crate::pins::verify;
use crate::session::{Ended, Session, Watch};
use crate::spawn::Spawner;
use crate::vmargs::prepare_args;
use crate::vmname::{Stamp, VmName};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// What a prepare produced.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub stamp: Stamp,
    pub record: CheckpointRecord,
    /// The inputs the checkpointed run was given.
    pub inputs: Inputs,
    /// `field=sha256` of those inputs' files.
    pub input_hashes: Vec<(String, String)>,
    pub name: VmName,
    pub elapsed: Duration,
    /// The `vmm` module's line of the module list when the prepare started. Every restore must see the same line.
    pub vmm: String,
    /// The prepare's own files. They are kept (the VM's standard error in full among them) only in a session that keeps
    /// everything; otherwise the directory is gone.
    pub prepare_run: RunPaths,
    /// SHA-256 of the VM program's arguments with the VM name and the run directory masked.
    pub argv_masked_sha256: String,
}

const PREPARE_RUN: u32 = 1;

struct Boot {
    stamp: Stamp,
    name: VmName,
    run: RunPaths,
    ckpt_dir: PathBuf,
    fwcfg: Files,
    inputs: Inputs,
    started: Instant,
    vmm: String,
    spawner: Spawner,
}

/// Waits until the three files exist, are not empty, and keep their sizes across two looks at least `gap` apart.
fn wait_settled(files: &[PathBuf; 3], timings: &Timings) -> Result<(), String> {
    let started = Instant::now();
    let mut previous: Option<(Instant, [u64; 3])> = None;
    loop {
        let sizes = files.clone().map(|f| len_now(&f));
        let present = files.iter().all(|f| f.is_file()) && sizes.iter().all(|s| *s > 0);
        previous = match (present, previous) {
            (true, Some((at, before))) if before == sizes => {
                if at.elapsed() >= timings.checkpoint_stable_gap {
                    return Ok(());
                }
                Some((at, before))
            }
            (true, _) => Some((Instant::now(), sizes)),
            (false, _) => None,
        };
        if started.elapsed() >= timings.checkpoint_wait {
            return Err("the checkpoint files did not settle in time".to_string());
        }
        sleep(timings.checkpoint_read_every);
    }
}

fn on_ready(
    spawner: Spawner,
    name: VmName,
    files: [PathBuf; 3],
    timings: Timings,
) -> crate::director::OnReady {
    Box::new(move |_event| {
        spawner.checkpoint(&name).map_err(|e| e.to_string())?;
        wait_settled(&files, &timings)
    })
}

/// No connection was made. When the VM had already exited, its status says more than the wait does.
fn connect_failure(exit: Option<i32>) -> InfraError {
    match exit {
        Some(n) if n != 0 => InfraError::new(
            InfraKind::NoConnect,
            format!("the VM exited with status {n}"),
        ),
        _ => InfraError::bare(InfraKind::NoConnect),
    }
}

fn failure_of(report: &Report, exit: Option<i32>) -> Option<InfraError> {
    match &report.error {
        Some(ControlError::NoConnect) => return Some(connect_failure(exit)),
        Some(ControlError::Cancelled) if !report.connected => return Some(connect_failure(exit)),
        Some(ControlError::Cancelled) => return Some(InfraError::bare(InfraKind::PrepareTimeout)),
        Some(ControlError::Lost(why)) => {
            return Some(InfraError::new(InfraKind::ControlLost, why.clone()))
        }
        Some(ControlError::Wire(e)) => {
            return Some(InfraError::new(InfraKind::ControlLost, e.to_string()))
        }
        None => {}
    }
    if report.ready == 0 {
        return Some(InfraError::bare(InfraKind::NoReady));
    }
    if let Some(why) = &report.callback_failure {
        return Some(InfraError::new(InfraKind::Checkpoint, why.clone()));
    }
    if !report.shutdown_sent {
        let why = match report.close {
            Some(CloseWhy::NotHeld) => "the stop was no longer held",
            _ => "no shutdown command was sent",
        };
        return Some(InfraError::new(InfraKind::ShutdownFailed, why));
    }
    match exit {
        Some(0) => None,
        Some(n) => Some(InfraError::new(
            InfraKind::ShutdownFailed,
            format!("exit status {n}"),
        )),
        None => Some(InfraError::new(
            InfraKind::ShutdownFailed,
            "the VM was ended by a signal",
        )),
    }
}

impl Session {
    /// Boots the guest to its ready stop, checkpoints it there and shuts it down.
    pub fn prepare(&self, inputs: Inputs) -> Result<Prepared, InfraError> {
        let started = Instant::now();
        self.count_prepare()?;
        verify(&self.host)?;
        let lock = self.lock()?;
        let stamp = match &self.host.session_stamp {
            Some(stamp) => stamp.clone(),
            None => lock
                .allocate_stamp()
                .map_err(|e| InfraError::new(InfraKind::FileOperation, e.to_string()))?,
        };
        let name = VmName::new(&stamp, PREPARE_RUN)
            .map_err(|e| InfraError::new(InfraKind::Settings, e.to_string()))?;
        let spawner = self.spawner(&stamp);
        let run = self.layout.run_paths(&stamp, &name);
        let ckpt_dir = self.layout.checkpoint_dir(&stamp);
        self.make_dirs(&run)?;
        std::fs::create_dir_all(&ckpt_dir)
            .map_err(|e| InfraError::io("create the checkpoint directory", &e))?;
        self.check_space(&spawner, 0)?;
        let fwcfg = inputs
            .write(&run.fwcfg_dir)
            .map_err(crate::trial::inputs_error)?;
        spawner.destroy(&name)?;
        // A start may load a module by itself when none is loaded, so it is checked each time, just before.
        let vmm = spawner.vmm_identity()?;
        let done = self.boot_and_checkpoint(&Boot {
            stamp,
            name,
            run: run.clone(),
            ckpt_dir,
            fwcfg,
            inputs,
            started,
            vmm,
            spawner,
        });
        if done.is_err() && !self.keep_all {
            keep_tail(&run.run_log);
            keep_tail(&run.uart_log);
        }
        done
    }

    fn boot_and_checkpoint(&self, boot: &Boot) -> Result<Prepared, InfraError> {
        let Boot {
            stamp,
            name,
            run,
            ckpt_dir,
            fwcfg,
            inputs,
            started,
            vmm,
            spawner,
        } = boot;
        let director = Director::start(
            DirectorSpec {
                sock_path: run.control_sock.clone(),
                mode: Mode::Make,
                run_log: run.run_log.clone(),
                restore: None,
                connect_wait: self.timings.connect_wait,
                resume_limit: self.timings.resume_limit,
                resume_poll: self.timings.resume_poll,
                shutdown_delay: self.timings.shutdown_delay,
            },
            on_ready(
                spawner.clone(),
                name.clone(),
                self.layout.checkpoint_files(stamp),
                self.timings,
            ),
        )
        .map_err(|e| InfraError::io("listen on the control socket", &e))?;
        let args = prepare_args(&self.host, run, fwcfg, name);
        let argv_masked_sha256 = crate::trial::masked_hash(&args, name, run);
        let mut guard = self.start_guarded(spawner.clone(), name.clone(), &args, run, director)?;
        let watch = Watch {
            cap: self.timings.prepare_cap,
            bounds: &self.bounds,
            run_log: &run.run_log,
            uart_log: &run.uart_log,
            every: self.timings.watch_every,
            stop_on_failure: true,
            grace_after_done: Some(self.timings.helper_cap),
        };
        let ended = guard.wait(&watch);
        let (exit, report) = guard.finish()?;
        self.check_prepare_end(ended, &report, exit)?;
        let ready = report
            .ready_event
            .ok_or_else(|| InfraError::bare(InfraKind::NoReady))?;
        let record = CheckpointRecord::read(stamp, ckpt_dir, RestoreKeys::from_event(&ready))
            .map_err(|e| match e {
                RecordError::Module(_) => InfraError::bare(InfraKind::ModuleMismatch),
                other => InfraError::new(InfraKind::Checkpoint, other.to_string()),
            })?;
        if !self.keep_all {
            remove_run_files(&run.dir);
        }
        Ok(Prepared {
            stamp: stamp.clone(),
            record,
            inputs: *inputs,
            input_hashes: fwcfg.hashes(),
            name: name.clone(),
            elapsed: started.elapsed(),
            vmm: vmm.clone(),
            prepare_run: run.clone(),
            argv_masked_sha256,
        })
    }

    fn check_prepare_end(
        &self,
        ended: Ended,
        report: &Report,
        exit: Option<i32>,
    ) -> Result<(), InfraError> {
        match ended {
            Ended::Breach(_) => return Err(InfraError::bare(InfraKind::LogBound)),
            Ended::Cap => {
                if !report.connected {
                    return Err(connect_failure(exit));
                }
                if report.ready == 0 {
                    return Err(InfraError::bare(InfraKind::NoReady));
                }
                if let Some(e) = failure_of(report, exit) {
                    return Err(e);
                }
                return Err(InfraError::bare(InfraKind::PrepareTimeout));
            }
            Ended::Exited(_) | Ended::EndpointFailed => {}
        }
        match failure_of(report, exit) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Removes the files of a prepare that succeeded: the logs and inputs are no longer needed.
fn remove_run_files(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}
