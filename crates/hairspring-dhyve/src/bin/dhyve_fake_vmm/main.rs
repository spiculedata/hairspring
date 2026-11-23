//! A stand-in for `bhyve`, `bhyvectl` and `sudo` that lets the run path be exercised on any machine. It is one program,
//! started through three symlinks named for the programs it stands in for, and it behaves as scripted by the files
//! next to the symlinks (see `scenario`). Nothing here is part of a product build.
mod bhyve;
mod ctl;
mod scenario;

fn main() {
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let fake = scenario::Fake::from_argv0(&argv0);
    let program = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let code = match program {
        "bhyve" => bhyve::run(&fake, &rest),
        "bhyvectl" => ctl::bhyvectl(&fake, &rest),
        "sudo" => ctl::sudo(&fake, &rest),
        _ => 64,
    };
    std::process::exit(code);
}
