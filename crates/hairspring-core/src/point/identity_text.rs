//! Renders the canonical identity text: one `key=value` line per input, in a fixed order.
use super::encoding::{
    encode_bool, encode_list, encode_optional_number, encode_optional_text, escape_value,
    normalize_sha256, Recorded,
};
use super::identity::{
    CommonInputs, HermitInputs, IdentityDraft, IdentityError, TmpPolicy, WasmInputs, SEED_ENV_NAME,
};
use std::fmt::Display;

/// Collects lines, the names of inputs that were not recorded, and the first invalid input.
#[derive(Default)]
struct Lines {
    text: String,
    missing: Vec<&'static str>,
    invalid: Option<(&'static str, &'static str)>,
}

impl Lines {
    fn push(&mut self, key: &str, value: &str) {
        self.text.push_str(key);
        self.text.push('=');
        self.text.push_str(value);
        self.text.push('\n');
    }

    fn miss(&mut self, key: &'static str) {
        self.missing.push(key);
    }

    fn reject(&mut self, key: &'static str, reason: &'static str) {
        self.invalid.get_or_insert((key, reason));
    }

    /// A SHA-256 hex digest, written in lowercase. Anything else counts as not recorded.
    fn sha(&mut self, key: &'static str, input: &Recorded<String>) {
        match input.value().and_then(|v| normalize_sha256(v)) {
            Some(digest) => self.push(key, &digest),
            None => self.miss(key),
        }
    }

    /// A non-empty text value.
    fn text(&mut self, key: &'static str, input: &Recorded<String>) {
        match input.value().filter(|v| !v.is_empty()) {
            Some(v) => self.push(key, &escape_value(v)),
            None => self.miss(key),
        }
    }

    fn number<T: Display>(&mut self, key: &'static str, input: &Recorded<T>) {
        match input.value() {
            Some(v) => self.push(key, &v.to_string()),
            None => self.miss(key),
        }
    }

    fn flag(&mut self, key: &'static str, input: &Recorded<bool>) {
        match input.value() {
            Some(v) => self.push(key, encode_bool(*v)),
            None => self.miss(key),
        }
    }

    fn optional_number(&mut self, key: &'static str, input: &Recorded<Option<u64>>) {
        match input.value() {
            Some(v) => self.push(key, &encode_optional_number(*v)),
            None => self.miss(key),
        }
    }

    fn optional_text(&mut self, key: &'static str, input: &Recorded<Option<String>>) {
        match input.value() {
            Some(v) => self.push(key, &encode_optional_text(v.as_deref())),
            None => self.miss(key),
        }
    }

    /// A list; `non_empty` lists (a command line) count as not recorded when empty.
    fn list(&mut self, key: &'static str, input: &Recorded<Vec<String>>, non_empty: bool) {
        match input.value().filter(|v| !non_empty || !v.is_empty()) {
            Some(items) => self.push(key, &encode_list(items)),
            None => self.miss(key),
        }
    }

    fn finish(self) -> Result<String, IdentityError> {
        if !self.missing.is_empty() {
            return Err(IdentityError::Incomplete {
                missing: self.missing,
            });
        }
        match self.invalid {
            Some((field, reason)) => Err(IdentityError::Invalid { field, reason }),
            None => Ok(self.text),
        }
    }
}

pub(super) fn render(draft: &IdentityDraft) -> Result<String, IdentityError> {
    let mut lines = Lines::default();
    let backend = draft.backend().as_str();
    match draft {
        IdentityDraft::Sim(common) => common_lines(&mut lines, backend, common),
        IdentityDraft::Wasm(common, wasm) => {
            common_lines(&mut lines, backend, common);
            wasm_lines(&mut lines, wasm);
        }
        IdentityDraft::Hermit(common, hermit) => {
            common_lines(&mut lines, backend, common);
            hermit_lines(&mut lines, hermit);
        }
    }
    lines.finish()
}

/// Each scheme is the backend name and its identity format version.
fn common_lines(lines: &mut Lines, backend: &str, common: &CommonInputs) {
    lines.push("scheme", &format!("{backend}/1"));
    lines.sha("hairspring_sha", &common.hairspring_sha);
    lines.text("target", &common.target);
    lines.push("backend", backend);
}

fn wasm_lines(lines: &mut Lines, wasm: &WasmInputs) {
    lines.sha("module_sha", &wasm.module_sha);
    lines.number("fuel", &wasm.fuel);
    lines.number("max_memory_bytes", &wasm.max_memory_bytes);
    lines.number("vm_semantics", &wasm.vm_semantics);
}

fn hermit_lines(lines: &mut Lines, h: &HermitInputs) {
    lines.sha("hermit_sha", &h.hermit_sha);
    lines.list("target_argv", &h.target_argv, true);
    lines.sha("target_exe_sha", &h.target_exe_sha);
    lines.text("epoch", &h.epoch);
    lines.optional_text("sched_heuristic", &h.sched_heuristic);
    lines.optional_number("sched_sticky_permille", &h.sched_sticky_permille);
    lines.flag("sched_chaos", &h.sched_chaos);
    lines.optional_number("sched_max_timeslice_ns", &h.sched_max_timeslice_ns);
    lines.optional_number("sched_skid_margin", &h.sched_skid_margin);
    lines.flag("sched_rbc_overshoot", &h.sched_rbc_overshoot);
    lines.list("hermit_args", &h.hermit_args, false);
    guest_env_line(lines, &h.guest_env);
    lines.text("cwd", &h.cwd);
    tmp_policy_line(lines, &h.tmp_policy);
    lines.text("events_transport", &h.events_transport);
    lines.sha("loader_ld_so_cache_sha", &h.loader_ld_so_cache_sha);
    lines.sha("loader_libc_sha", &h.loader_libc_sha);
    lines.sha("loader_ld_linux_sha", &h.loader_ld_linux_sha);
    lines.flag("hang_is_violation", &h.hang_is_violation);
}

/// The environment in the order given, as `KEY=VALUE` items. Keys must be usable and unique, and
/// the seed variable is never part of the identity.
fn guest_env_line(lines: &mut Lines, input: &Recorded<Vec<(String, String)>>) {
    const KEY: &str = "guest_env";
    let Some(env) = input.value() else {
        lines.miss(KEY);
        return;
    };
    let mut seen: Vec<&str> = Vec::with_capacity(env.len());
    for (name, _) in env {
        if name.is_empty() || name.contains('=') {
            lines.reject(KEY, "a variable name must be non-empty and contain no `=`");
        } else if name == SEED_ENV_NAME {
            lines.reject(KEY, "the seed variable is not an identity input");
        } else if seen.contains(&name.as_str()) {
            lines.reject(KEY, "duplicate variable name");
        }
        seen.push(name);
    }
    let items: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    lines.push(KEY, &encode_list(&items));
}

fn tmp_policy_line(lines: &mut Lines, input: &Recorded<TmpPolicy>) {
    const KEY: &str = "tmp_policy";
    match input.value() {
        None => lines.miss(KEY),
        Some(TmpPolicy::None) => lines.push(KEY, "none"),
        Some(TmpPolicy::Path(path)) if path.is_empty() => {
            lines.reject(KEY, "a `--tmp` path must not be empty");
        }
        Some(TmpPolicy::Path(path)) => lines.push(KEY, &format!("path:{}", escape_value(path))),
    }
}
