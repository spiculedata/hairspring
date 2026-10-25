//! Offline downgrade of a Hairspring database from schema version 3, 4 or 5 to version 2. Back the database up first;
//! stop every server and worker that uses it.
//!
//!     cargo run -p hairspring-store --example store_downgrade -- --sqlite path/to/hairspring.db --dry-run
//!     cargo run -p hairspring-store --example store_downgrade -- --sqlite path/to/hairspring.db --yes
//!     HAIRSPRING_PG_URL=postgres://user:pass@host/db \
//!       cargo run -p hairspring-store --features postgres --example store_downgrade -- \
//!         --postgres --schema public [--drop-branch-runs] --yes
//!
//! The PostgreSQL URL (it holds the password) is read from `HAIRSPRING_PG_URL`, never from the command line.
//! Branch runs (runs created from a point of a trial tree) are listed first and kept as ordinary runs unless
//! `--drop-branch-runs` is given. `--dry-run` reports what would happen, with true counts, and changes nothing. A
//! real run changes the database and needs `--yes`.
use hairspring_store::{downgrade_sqlite, DowngradeOptions, DowngradeReport};
use std::path::PathBuf;
use std::process::ExitCode;

enum Target {
    Sqlite(PathBuf),
    #[cfg(feature = "postgres")]
    Postgres {
        url: String,
        schema: String,
    },
}

struct Args {
    target: Target,
    options: DowngradeOptions,
}

fn parse(args: Vec<String>) -> Result<Args, String> {
    let (mut sqlite, mut postgres, mut schema) = (None, false, "public".to_string());
    let (mut options, mut yes) = (DowngradeOptions::default(), false);
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--sqlite" => sqlite = Some(it.next().ok_or("--sqlite needs a path")?),
            "--postgres" => postgres = true,
            "--schema" => schema = it.next().ok_or("--schema needs a name")?,
            "--drop-branch-runs" => options.drop_branch_runs = true,
            "--dry-run" => options.dry_run = true,
            "--yes" => yes = true,
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if !options.dry_run && !yes {
        return Err(
            "this changes the database: back it up, then pass --yes (or --dry-run to look)".into(),
        );
    }
    let target = match (sqlite, postgres) {
        (Some(path), false) => Target::Sqlite(PathBuf::from(path)),
        #[cfg(feature = "postgres")]
        (None, true) => {
            let url = std::env::var("HAIRSPRING_PG_URL")
                .map_err(|_| "set HAIRSPRING_PG_URL to the PostgreSQL URL")?;
            Target::Postgres { url, schema }
        }
        #[cfg(not(feature = "postgres"))]
        (None, true) => {
            return Err("this build has no PostgreSQL support (--features postgres)".into())
        }
        _ => return Err("give exactly one of --sqlite PATH or --postgres".into()),
    };
    #[cfg(not(feature = "postgres"))]
    drop(schema);
    Ok(Args { target, options })
}

fn print(report: &DowngradeReport) {
    println!(
        "schema version {} -> {}",
        report.from_version, report.to_version
    );
    println!("branch runs ({}):", report.branch_runs.len());
    for run in &report.branch_runs {
        println!("  {run}");
    }
    println!("deleted branch runs: {}", report.dropped_runs.len());
    println!("removed blobs: {}", report.dropped_blobs);
    println!("applied: {}", report.applied);
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1).collect()) {
        Ok(a) => a,
        Err(why) => {
            eprintln!("error: {why}");
            return ExitCode::from(2);
        }
    };
    let result = match &args.target {
        Target::Sqlite(path) => downgrade_sqlite(path, &args.options),
        #[cfg(feature = "postgres")]
        Target::Postgres { url, schema } => {
            hairspring_store::downgrade_postgres(url, schema, &args.options)
        }
    };
    match result {
        Ok(report) => {
            print(&report);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
