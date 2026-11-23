//! One trial: a restore of the checkpoint, given its own input files, resumed through the control endpoint and read to the
//! guest's exit report. This module only collects what happened; `judge` says what it means.
use crate::director::{Director, DirectorSpec, Mode, Report};
use crate::error::{InfraError, InfraKind};
use crate::fwcfg::Inputs;
use crate::layout::RunPaths;
use crate::logs::{dir_bytes, keep_tail, len_now};
use crate::prepare::Prepared;
use crate::runlog::{limit_stop_after_connect, RestoreScanKeys};
use crate::serial::{self, Serial};
use crate::session::{Ended, Session, Watch};
use crate::vmargs::restore_args;
use crate::vmname::{Stamp, VmName, NAMES_PER_STAMP};
use std::fs::File;
use std::io::BufReader;
use std::time::{Duration, Instant};

/// Trials with an ordinal at or past this get a stamp of their own, because a stamp has only 100 names and the first is
/// the prepare's.
const FIRST_OWN_STAMP_ORDINAL: u32 = NAMES_PER_STAMP - 1;

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
    /// The serial log; `None` when the file was never there.
    pub serial: Option<Serial>,
    /// A control stop was logged after the connect line: the VM ran into its branch limit.
    pub stopped_at_limit_in_log: bool,
    pub run_log_len: u64,
    pub uart_len: u64,
    pub elapsed: Duration,
    pub run: RunPaths,
}

impl Session {
    /// Restores the checkpoint in `prepared` as trial number `ordinal`, with `inputs` as its input files.
    pub fn restore_trial(
        &self,
        prepared: &Prepared,
        ordinal: u32,
        inputs: Inputs,
    ) -> Result<TrialRun, InfraError> {
        let started = Instant::now();
        let lock = self.lock()?;
        let (stamp, number) = if ordinal < FIRST_OWN_STAMP_ORDINAL {
            (prepared.stamp.clone(), ordinal + 1)
        } else {
            let fresh = lock
                .allocate_stamp()
                .map_err(|e| InfraError::new(InfraKind::FileOperation, e.to_string()))?;
            (fresh, 1)
        };
        let name = VmName::new(&stamp, number)
            .map_err(|e| InfraError::new(InfraKind::Settings, e.to_string()))?;
        let run = self.layout.run_paths(&stamp, &name);
        self.make_dirs(&run)?;
        if !prepared.record.unchanged() {
            return Err(InfraError::new(
                InfraKind::Checkpoint,
                "the checkpoint changed after it was taken",
            ));
        }
        self.check_space(dir_bytes(&prepared.record.dir))?;
        let fwcfg = inputs
            .write(&run.fwcfg_dir)
            .map_err(|e| InfraError::new(InfraKind::InputFiles, e.to_string()))?;
        self.spawner.destroy(&name)?;
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
        let mut child = match self.spawner.start_vm(&args, &run.run_log) {
            Ok(c) => c,
            Err(e) => {
                director.cancel();
                director.finish(&run.control_sock);
                return Err(e);
            }
        };
        let watch = Watch {
            cap: self.timings.trial_cap,
            bounds: &self.bounds,
            run_log: &run.run_log,
            uart_log: &run.uart_log,
            every: self.timings.watch_every,
            stop_on_failure: true,
            grace_after_done: None,
        };
        let ended = self.wait(&mut child, &director, &watch);
        let ended_vm = self.end_vm(&name, &mut child, &director);
        if !director.is_finished() {
            director.cancel();
        }
        let control = director.finish(&run.control_sock);
        let vm_exit = ended_vm?;
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
        }))
    }

    fn collect(&self, c: Collected) -> TrialRun {
        let serial = serial::read(&c.run.uart_log, self.bounds.uart_log_bytes)
            .ok()
            .flatten();
        let reported_exit = serial.as_ref().is_some_and(|s| s.exit.is_some());
        let stopped_at_limit_in_log = !reported_exit
            && File::open(&c.run.run_log).is_ok_and(|f| {
                limit_stop_after_connect(
                    BufReader::new(f),
                    &c.run.control_sock.display().to_string(),
                )
            });
        let run_log_len = len_now(&c.run.run_log);
        let uart_len = len_now(&c.run.uart_log);
        keep_tail(&c.run.run_log);
        keep_tail(&c.run.uart_log);
        TrialRun {
            name: c.name,
            stamp: c.stamp,
            inputs: c.inputs,
            input_hashes: c.input_hashes,
            control: c.control,
            ended: c.ended,
            vm_exit: c.vm_exit,
            serial,
            stopped_at_limit_in_log,
            run_log_len,
            uart_len,
            elapsed: c.started.elapsed(),
            run: c.run,
        }
    }

    /// Removes a trial's files, or leaves its kept tails for a finding or an error.
    pub fn clean_trial(&self, trial: &TrialRun, keep: bool) {
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
}
