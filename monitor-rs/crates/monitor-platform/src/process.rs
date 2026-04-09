use std::process::Command;
use tracing::{info, warn};

/// Kill all processes matching the given name (case-insensitive).
/// Used at startup to clean up stale FFmpeg processes from a previous run.
pub fn kill_stale(process_name: &str) -> u32 {
    // On Windows, use taskkill to terminate by image name.
    let output = Command::new("taskkill")
        .args(["/F", "/IM", process_name])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let killed = stdout.matches("SUCCESS").count() as u32;
            if killed > 0 {
                info!("killed {killed} stale {process_name} process(es)");
            }
            killed
        }
        Err(e) => {
            warn!("failed to run taskkill for {process_name}: {e}");
            0
        }
    }
}

/// Terminate a process tree by PID.
pub fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}
