// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Starts the application packaged after this executable without parsing application arguments.

use janex_host::{
    native_launcher::{self, LaunchOverrides},
    run::{ExecutionPlan, LaunchMode},
};
use std::{fs::File, process::ExitStatus};

/// Reports preparation failures and returns the application's exit status.
fn main() {
    let code = match run() {
        Ok(status) => exit_code(status),
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    };
    std::process::exit(code);
}

/// Opens the running executable independently of argv[0] and keeps its snapshot alive through exit.
fn run() -> Result<ExitStatus, Box<dyn std::error::Error>> {
    let target = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    let source = File::open("/proc/self/exe")?;
    #[cfg(not(target_os = "linux"))]
    let source = File::open(&target)?;
    let mut overrides = LaunchOverrides::default();
    overrides.java.java = std::env::var_os("JANEX_JAVA").map(Into::into);
    if let Some(mode) = std::env::var_os("JANEX_LAUNCH_MODE") {
        overrides.launch_mode = Some(match mode.to_str() {
            Some("bootstrap") => LaunchMode::Bootstrap,
            Some("direct") => LaunchMode::Direct,
            _ => return Err("JANEX_LAUNCH_MODE must be bootstrap or direct".into()),
        });
    }
    let plan = native_launcher::prepare(
        source,
        &target,
        std::env::args_os().skip(1).collect(),
        overrides,
    )?;
    execute(plan)
}

/// Waits for Java while retaining launch resources and handling process termination.
fn execute(plan: ExecutionPlan) -> Result<ExitStatus, Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, kill},
            unistd::Pid,
        };
        use signal_hook::{
            consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
            iterator::Signals,
        };
        use std::time::Duration;
        let mut signals = Signals::new([SIGINT, SIGTERM, SIGHUP, SIGQUIT])?;
        let mut child = plan.command().spawn()?;
        loop {
            for signal in signals.pending() {
                // The child is unreaped, so its PID cannot have been recycled.
                let _ = kill(Pid::from_raw(child.id() as i32), Signal::try_from(signal)?);
            }
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        // Console events are delivered to Java too; keep the parent alive until Java releases files.
        ctrlc::set_handler(|| {})?;
        Ok(plan.execute()?)
    }
}

/// Preserves native exit codes and represents Unix termination signals as shell statuses.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}
