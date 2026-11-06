# Versions, upgrades and uninstalling

Hairspring is experimental and changes quickly. This page says how to tell which build you have, what differs between
the pre-release binary and a build of `main`, what happens to your results database when you upgrade, and how to remove
Hairspring again. For running a server see [deploy.md](deploy.md).

Everything on this page about commands was checked by running them on macOS (Apple silicon), with the v0.1-RC2 and
v0.1-RC1 macOS binaries and a build of `main`. Anything that needs Linux x86-64 is marked "not run here".

## Which build do I have?

```console
$ hairspring --version
hairspring 0.1.0 (c2687b8 2026-11-05)
```

That line is what the v0.1-RC2 binary prints. `hairspring --version` (or `-V`) prints the crate version, then the commit
the binary was built from and that commit's date. v0.1-RC2 was built from commit `c2687b8`. The v0.1-RC1 binary was
built before the commit and date were added and prints only `hairspring 0.1.0`, so a bare `hairspring 0.1.0` with no
commit means v0.1-RC1 (or another build older than the change); it also means you cannot tell two such builds apart from
the version alone. The release notes for v0.1-RC1 name its commit: `f819c1a`, committed on 24 October 2026. A build from
a source archive, or any build that had no git history, prints
`hairspring 0.1.0 (unknown)`. When you report a problem, quote the whole line (see
[Getting started and getting help](../README.md#getting-started-and-getting-help)).

## Pre-release binary or a build of `main`?

The documentation in this repository describes `main`. The current pre-release is v0.1-RC2, which
`bash scripts/install.sh --version v0.1-RC2` installs. Comparing `--help` of the v0.1-RC2 macOS binary with a build of
`main` (commit `567d810`), for the top level and for every subcommand, showed no difference in commands, options or
help text; the only difference in the output is the program name on the `Usage:` line, which is the downloaded file's
name. `main` can still differ from the pre-release in behaviour that `--help` does not show: the "Unreleased" section of
[CHANGELOG.md](../CHANGELOG.md) lists what has changed since v0.1-RC2 where it is visible to a user. The database
schema is the same (version 8). If a page of the documentation describes something your binary does not do, check
`hairspring --help` and `hairspring <command> --help` first, then your `hairspring --version`, and if the behaviour is
in `main` only, build from source to use it:

```bash
git clone https://github.com/spiculedata/hairspring && cd hairspring
bash scripts/install.sh --with-ui      # builds the web UI (needs Node.js and npm) and the CLI, installs to ~/.local/bin
# Leave --with-ui out for a build without the web UI: the server then shows a placeholder page.
```

### If you have v0.1-RC1

The v0.1-RC1 binary is older than v0.1-RC2. Comparing `--help` of the v0.1-RC1 macOS binary with v0.1-RC2 (each
subcommand's options) shows these differences, and no others on the command line:

| Only in v0.1-RC2 and later | What v0.1-RC1 says |
|---|---|
| `hairspring gc` (retention) | `error: unrecognized subcommand 'gc'` |
| `hairspring serve --trusted-proxy` | `error: unexpected argument '--trusted-proxy' found` |
| `hairspring serve --organisation` | `error: unexpected argument '--organisation' found` |
| `hairspring serve --max-connections` and `--max-replays` | not in `serve --help`; not accepted |
| `hairspring test --confirm` (with `--sweep`) | not in `test --help`; not accepted |
| the commit and date in `hairspring --version` | prints `hairspring 0.1.0` only |

Changes inside the server and the web UI are not in this table: only the command line was compared. To get them, install
v0.1-RC2 or build from source as above.

You need Rust 1.96 or newer, a C compiler and linker, and for `--with-ui` Node.js 20.19 or newer (or 22.12 or newer) with npm (see [quickstart.md](quickstart.md#what-you-need)). A pre-release
binary is checked against the release's `SHA256SUMS` before it is installed, which catches a damaged download but does
not prove who built it; to pin a hash you got another way, pass `--sha256` to the installer.

## Upgrading to schema 8 on PostgreSQL

Schema 8 adds a table, `run_inputs`, and an index on `runs`. The first open of a version 7 schema must be made by the role
that owns the schema's tables (creating the index needs ownership of `runs`), at a quiet time: the index build blocks
writes to `runs` until it finishes. Before any other role opens the schema, the owner must grant it the new table:

```sql
GRANT SELECT, INSERT, DELETE ON <schema>.run_inputs TO <role>;
```

A role without those rights is refused at open with that statement in the message, before anything is changed. On SQLite
nothing needs granting; every write to the database waits during the upgrade (up to the busy timeout, then fails). The
details are in [architecture.md](architecture.md#run-inputs-schema-version-8).

## The schema-version error

Hairspring stores results in a database (SQLite by default, `.hairspring/hairspring.db`, or PostgreSQL). The database
records a schema version. When a newer build opens a database made by an older one it **migrates it in place, silently**.
An older build then refuses the database:

```console
$ hairspring results
error: storage error: database schema version 6 is not supported (expected 5)
```

(exit status 2). That text was printed by the v0.1-RC1 binary for a copy of a database made by a build of `main` when `main`
was at schema 6. With a database made by v0.1-RC2 or a current `main` build, which is at schema 8, the same binary gives

```text
error: storage error: database schema version 8 is not supported (expected 5)
```

(exit status 2; run). The v0.1-RC2 binary refuses a database made by a newer schema in the same way, with `(expected 8)`
at the end. The numbers change as schemas are added; the meaning does not. The number after
"version" is the database's; the number after "expected" is the highest your binary knows. The database is not damaged: the
binary you are running is older than the one that last opened it. The table of what each version adds is in
[architecture.md](architecture.md).

What to do:

- **Keep using the newer build.** This is the usual answer: install or build the newer one wherever you open that
  database (the same machine, and every server and worker that uses it).
- **Or point the older build at a different database** with `--db`, for example `hairspring --db other.db results`. That
  starts a new, empty set of results; it does not read the old ones.
- **Or restore a backup** taken before the newer build ran (below).

The migration is one way. There is no downgrade command in the `hairspring` CLI. [architecture.md](architecture.md)
describes an offline downgrade routine for developers; it drops stored data for the newer versions, so do not use it on
results you want to keep, and restore a backup instead.

## Upgrade safely

1. **Take a backup first.** For SQLite:

   ```bash
   (umask 077; sqlite3 .hairspring/hairspring.db ".backup 'hairspring-backup.db'")
   ```

   This was run and the copy read back with `hairspring --db hairspring-backup.db results`. For PostgreSQL use
   `pg_dump -h db.internal -U hairspring hairspring > hairspring-backup.sql` with the password in `~/.pgpass` (mode 600) or a
   `PGPASSFILE`, not on the command line (not run here). Copy `.hairspring/bundles` as well if
   you keep bundle files there, and keep your `hairspring.toml` and tokens file.
2. **Stop every server and worker** that uses the database. A process left running on the old build can keep writing in
   the old way after the database has moved on (architecture.md gives an example for sweeps).
3. **Upgrade the server and all workers together**, on every host, to the same build. The documentation does not describe a
   mixed-build setup, and architecture.md says to stop every older server and worker before upgrading.
4. Start the server, then the workers. Check `hairspring --version` on each, and open the Workers page.
5. To go back, stop everything, put the old binary back, and **restore the backup** over the database. Anything stored
   after the backup is lost.

If you only want to try a newer build without touching your results, run it with a different database (`--db`) in a
different directory.

## Uninstall

What Hairspring writes outside a project directory, from `scripts/install.sh` and `scripts/install-hermit.sh`:

| What | Where (the defaults) | Remove with |
|---|---|---|
| The `hairspring` command | `~/.local/bin/hairspring` (`--prefix DIR` puts it in `DIR/bin`) | `rm ~/.local/bin/hairspring` |
| Hermit, if you installed it | `~/.local/share/hairspring/hermit` (`--prefix` changes it; the folder holds a provenance file written by the installer) | `rm -rf ~/.local/share/hairspring/hermit` |
| Results and bundles | `.hairspring/` in each directory you ran `hairspring` in | `rm -rf .hairspring` in each |
| Your configuration | `hairspring.toml` where you ran `hairspring init` | delete it if you no longer want it |

The installers never use `sudo` and, by their own description, write only below their prefix and into a scratch
directory they remove. Building from source also fills cargo's own caches (`~/.cargo`), which are not specific to
Hairspring. Hermit's installer can also install a Rust
toolchain for you if you pass `--install-toolchain`; remove it with `rustup toolchain uninstall <toolchain>`, using the `RUST_TOOLCHAIN` named in `scripts/hermit.pin`
if you no longer need it (not run here). Delete the checkout you built from if you do not need it. If you added the
folder holding `hairspring` to your `PATH` or exported `HAIRSPRING_HERMIT` in a shell profile, remove those lines.

A server or worker set up as in [deploy.md](deploy.md): stop and disable the systemd units and remove them
(`systemctl disable --now hairspring`, and `hairspring-worker` on a worker host; `rm /etc/systemd/system/hairspring.service`
and `hairspring-worker.service`; `systemctl daemon-reload`; not run here). What else you created is yours to keep or
remove, and nothing here deletes it for you: the `hairspring` system user (`userdel hairspring`), `/etc/hairspring` (tokens
and configuration), and `/var/lib/hairspring` and `/var/lib/hairspring-worker` (the database, bundles and working
directories; take a backup first if you may want the results). Not run here. For Docker Compose, `docker compose down`
stops the stack, and `docker compose down -v` also deletes the PostgreSQL volume with all results (not run here).

### The SDKs

The SDKs live in the repository; nothing is installed system-wide unless you installed one yourself.

- **Python:** `pip uninstall hairspring` if you ran `pip install ./sdks/python`. That install also left ignored `build/`
  and `*.egg-info` folders under `sdks/python`; delete them if you want a clean checkout.
- **Rust, Go, Java, .NET, C:** remove the dependency you added (the path dependency in `Cargo.toml`, the `go.mod`
  requirement, the jar on your class path, the project reference, the built library) and rebuild your program.
