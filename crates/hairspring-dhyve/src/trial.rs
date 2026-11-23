//! One trial: a restore of the checkpoint, given its own input files, resumed through the control endpoint and read to the
//! guest's exit report. This module only collects what happened; `judge` says what it means.
use crate::director::{Director, DirectorSpec, Mode, Report};
use crate::error::{InfraError, InfraKind};
use crate::fwcfg::Inputs;
use crate::layout::RunPaths;
use crate::logs::{dir_bytes, keep_tail, len_now};
use crate::pins::verify;
use crate::prepare::Prepared;
use crate::runlog::{stop_after_connect, RestoreScanKeys, StopLine};
use crate::safe_open::open_regular;
use crate::serial::{self, Serial};
use crate::session::{Ended, Session, Watch};
use crate::vmargs::restore_args;
use crate::vmname::{Stamp, VmName, NAMES_PER_STAMP};
use hairspring_core::point::encoding::sha256_hex;
use std::io::BufReader;
use std::time::{Duration, Instant};

/// The first restore is the second name of its stamp: the prepare is the first.
const FIRST_RESTORE_RUN: u32 = 2;

/// Everything seen of one restored run.
#[derive(Debug, Clone)]
pub struct TrialRun {
    pub name: VmName,
    pub stamp: Stamp,
    pub inputs: Inputs,
    pub input_hashes: Vec<(String, String)>,
    pub control: Report,
    pub ended: Ended,
    pub vm_exit: Option<i32>,
    /// The serial log; `None` when the file was never there or was refused.
    pub serial: Option<Serial>,
    /// A control stop was logged after the connect line: the VM stopped again after it was resumed. The kind says which
    /// stop it was.
    pub stop_after_resume: Option<StopLine>,
    pub run_log_len: u64,
    pub uart_len: u64,
    pub elapsed: Duration,
    pub run: RunPaths,
    /// SHA-256 of the VM program's arguments with the VM name and the run directory masked: the record that two trials
    /// were started alike, without keeping the arguments.
    pub argv_masked_sha256: String,
}

fn masked_hash(args: &[String], name: &VmName, run: &RunPaths) -> String {
    let dir = run.dir.display().to_string();
    let text: String = args
        .iter()
        .map(|a| a.replace(name.as_str(), "NAME").replace(&dir, "RUN"))
        .collect::<Vec<_>>()
        .join("\n");
    sha256_hex(text.as_bytes())
}

impl Session {
    /// Restores the checkpoint in `prepared` as trial number `ordinal` (from 0), with `inputs` as its input files.
    pub fn restore_trial(
        &self,
        prepared: &Prepared,
        ordinal: u32,
        inputs: Inputs,
    ) -> Result<TrialRun, InfraError> {
        let started = Instant::now();
        verify(&self.host)?;
        let lock = self.lock()?;
        let number = ordinal.saturating_add(FIRST_RESTORE_RUN);
        let (stamp, number) = if number < NAMES_PER_STAMP {
            (prepared.stamp.clone(), number)
        } else if self.host.session_stamp.is_some() {
            return Err(InfraError::new(
                InfraKind::Settings,
                "the session's stamp has no names left",
            ));
        } else {
            let fresh = lock
                .allocate_stamp()
                .map_err(|e| InfraError::new(InfraKind::FileOperation, e.to_string()))?;
            (fresh, 1)
        };
        let name = VmName::new(&stamp, number)
            .map_err(|e| InfraError::new(InfraKind::Settings, e.to_string()))?;
        let spawner = self
            .spawner(&stamp)
            .with_checkpoint_dir(&prepared.record.dir);
        let run = self.layout.run_paths(&stamp, &name);
        self.make_dirs(&run)?;
        if !prepared.record.unchanged() {
            return Err(InfraError::new(
                InfraKind::Checkpoint,
                "the checkpoint changed after it was taken",
            ));
        }
        self.check_space(&spawner, dir_bytes(&prepared.record.dir))?;
        let fwcfg = inputs
            .write(&run.fwcfg_dir)
            .map_err(|e| InfraError::new(InfraKind::InputFiles, e.to_string()))?;
        spawner.destroy(&name)?;
        // A start may load a module by itself when none is loaded, and the module must be the one the checkpoint came
        // from, so it is checked each time, just before.
        if spawner.vmm_identity()? != prepared.vmm {
            return Err(InfraError::bare(InfraKind::VmmChanged));
        }
        let record = &prepared.record;
        let director = Director::start(
            DirectorSpec {
                sock_path: run.control_sock.clone(),
                mode: Mode::Restore,
                run_log: run.run_log.clone(),
                restore: Some(RestoreScanKeys {
                    stop_seq: record.keys.stop_seq,
                    branch_count: record.keys.branch_count,
                    vtime_ns: record.keys.vtime_ns,
                    vcpu_id: record.keys.vcpu_id,
                    rip: Some(record.rip),
                    sock: run.control_sock.display().to_string(),
                }),
                connect_wait: self.timings.connect_wait,
                resume_limit: self.timings.resume_limit,
                resume_poll: self.timings.resume_poll,
                shutdown_delay: self.timings.shutdown_delay,
            },
            Box::new(|_| Ok(())),
        )
        .map_err(|e| InfraError::io("listen on the control socket", &e))?;
        let args = restore_args(&self.host, &run, &fwcfg, record, &name);
        let argv_masked_sha256 = masked_hash(&args, &name, &run);
        let mut guard = self.start_guarded(spawner, name.clone(), &args, &run, director)?;
        let watch = Watch {
            cap: self.timings.trial_cap,
            bounds: &self.bounds,
            run_log: &run.run_log,
            uart_log: &run.uart_log,
            every: self.timings.watch_every,
            stop_on_failure: true,
            grace_after_done: None,
        };
        let ended = guard.wait(&watch);
        let (vm_exit, control) = guard.finish()?;
        drop(lock);
        Ok(self.collect(Collected {
            name,
            stamp,
            inputs,
            input_hashes: fwcfg.hashes(),
            control,
            ended,
            vm_exit,
            run,
            started,
            argv_masked_sha256,
        }))
    }

    fn collect(&self, c: Collected) -> TrialRun {
        let serial = serial::read(&c.run.uart_log, self.bounds.uart_log_bytes)
            .ok()
            .flatten();
        let reported_exit = serial.as_ref().is_some_and(|s| s.exit.is_some());
        let sock = c.run.control_sock.display().to_string();
        let stop_after_resume = if reported_exit {
            None
        } else {
            open_regular(&c.run.run_log)
                .ok()
                .and_then(|f| stop_after_connect(BufReader::new(f), &sock))
        };
        let run_log_len = len_now(&c.run.run_log);
        let uart_len = len_now(&c.run.uart_log);
        if !self.keep_all {
            keep_tail(&c.run.run_log);
            keep_tail(&c.run.uart_log);
        }
        TrialRun {
            name: c.name,
            stamp: c.stamp,
            inputs: c.inputs,
            input_hashes: c.input_hashes,
            control: c.control,
            ended: c.ended,
            vm_exit: c.vm_exit,
            serial,
            stop_after_resume,
            run_log_len,
            uart_len,
            elapsed: c.started.elapsed(),
            run: c.run,
            argv_masked_sha256: c.argv_masked_sha256,
        }
    }

    /// Removes a trial's files, or leaves its kept tails for a finding or an error. A session that keeps everything
    /// removes nothing.
    pub fn clean_trial(&self, trial: &TrialRun, keep: bool) {
        if self.keep_all {
            return;
        }
        if keep {
            let _ = std::fs::remove_dir_all(&trial.run.fwcfg_dir);
        } else {
            let _ = std::fs::remove_dir_all(&trial.run.dir);
        }
    }
}

struct Collected {
    name: VmName,
    stamp: Stamp,
    inputs: Inputs,
    input_hashes: Vec<(String, String)>,
    control: Report,
    ended: Ended,
    vm_exit: Option<i32>,
    run: RunPaths,
    started: Instant,
    argv_masked_sha256: String,
}
