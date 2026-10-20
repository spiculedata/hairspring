use crate::cli::ReplayArgs;
use crate::ctx::{CmdResult, Ctx};
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::ReplayOptions;
use hairspring_service::MAX_BUNDLE_BYTES;
use std::io::{Read, Write};
use std::path::Path;

fn load_text(ctx: &Ctx, input: &str) -> CmdResult<String> {
    if input == "-" {
        let mut s = String::new();
        std::io::stdin()
            .take(MAX_BUNDLE_BYTES as u64 + 1)
            .read_to_string(&mut s)?;
        return Ok(s);
    }
    if Path::new(input).is_file() {
        let meta = std::fs::metadata(input)?;
        if meta.len() as usize > MAX_BUNDLE_BYTES {
            return Err(format!("bundle file exceeds {MAX_BUNDLE_BYTES} bytes").into());
        }
        return Ok(std::fs::read_to_string(input)?);
    }
    if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(t) = ctx.service()?.get_bundle(input)? {
            return Ok(t);
        }
    }
    Err(format!("`{input}` is not a readable bundle file or a stored bundle hash").into())
}

pub fn run(ctx: &Ctx, args: &ReplayArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let text = load_text(ctx, &args.input)?;
    if text.len() > MAX_BUNDLE_BYTES {
        return Err(format!("bundle exceeds {MAX_BUNDLE_BYTES} bytes").into());
    }
    let bundle = Bundle::decode(&text).map_err(|e| format!("invalid bundle: {e}"))?;
    let options = ReplayOptions {
        allow_environment_change: args.allow_environment_change,
    };
    let replay =
        hairspring_service::sanitize::replay_steps(ctx.engine.replay_with(&bundle, options)?);
    for warning in &replay.warnings {
        writeln!(out, "warning: {warning}")?;
    }
    writeln!(
        out,
        "replaying {} steps on target `{}` (backend {})",
        replay.steps.len(),
        bundle.target,
        bundle.backend
    )?;
    for s in &replay.steps {
        writeln!(out, "  {:>3}. {}", s.index + 1, s.label)?;
    }
    if replay.reproduced {
        writeln!(out, "reproduced: the property is still violated")?;
        Ok(crate::EXIT_VIOLATION)
    } else if replay
        .warnings
        .iter()
        .any(|w| w.starts_with("environment differs"))
    {
        writeln!(
            out,
            "NOT reproduced, but the environment differs from the recording: this does not show the bug is fixed"
        )?;
        Ok(crate::EXIT_OK)
    } else {
        writeln!(
            out,
            "NOT reproduced: the bundle no longer fails (fixed, or the build changed)"
        )?;
        Ok(crate::EXIT_OK)
    }
}
