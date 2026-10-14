use crate::scenario::AppSection;
use std::process::{Child, Command, Stdio};

/// Host variables the app may see; everything else (tokens, cloud credentials) is withheld.
const PASSED_ENV: [&str; 5] = ["PATH", "HOME", "LANG", "TZ", "TMPDIR"];

#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
#[cfg(unix)]
const SIGKILL: i32 = 9;

/// Starts and stops the application under test.
pub trait AppLauncher {
    /// Starts (or restarts) the app, telling it where the sidecar listens. Returns the app's HTTP port.
    fn start(&mut self, sidecar_port: u16) -> Result<u16, String>;
    /// Stops the app, discarding its in-memory state.
    fn stop(&mut self);
}

/// Runs the app as a child process with `DAPR_HTTP_PORT`, `DAPR_APP_PORT` and `APP_PORT` set.
pub struct ProcessApp {
    section: AppSection,
    port: Option<u16>,
    child: Option<Child>,
}

fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

impl ProcessApp {
    pub fn new(section: AppSection) -> ProcessApp {
        ProcessApp {
            port: section.port,
            section,
            child: None,
        }
    }
}

impl AppLauncher for ProcessApp {
    fn start(&mut self, sidecar_port: u16) -> Result<u16, String> {
        self.stop();
        let Some((program, args)) = self.section.command.split_first() else {
            return Err(
                "no app command configured (set [app].command or pass it after `--`)".to_string(),
            );
        };
        let port = match self.port {
            Some(p) => p,
            None => {
                let p = free_port()?;
                self.port = Some(p); // keep the same port across restarts
                p
            }
        };
        let mut cmd = Command::new(program);
        cmd.args(args).env_clear();
        for name in PASSED_ENV {
            if let Ok(v) = std::env::var(name) {
                cmd.env(name, v);
            }
        }
        cmd.env("DAPR_HTTP_PORT", sidecar_port.to_string())
            .env("DAPR_APP_PORT", port.to_string())
            .env("APP_PORT", port.to_string())
            .stdin(Stdio::null());
        cmd.envs(&self.section.env);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // so stop() can kill the app's children too
        }
        if let Some(dir) = &self.section.cwd {
            cmd.current_dir(dir);
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not launch `{program}`: {e}"))?;
        self.child = Some(child);
        Ok(port)
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            // SAFETY: plain syscall; the child leads its own group (process_group(0)), so -pid targets only that group.
            unsafe {
                kill(-(child.id() as i32), SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ProcessApp {
    fn drop(&mut self) {
        self.stop();
    }
}
