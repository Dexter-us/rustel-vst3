use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::Mutex,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static CONFIGURE_LOCK: Mutex<()> = Mutex::new(());

pub fn configure(extra_folders: &str, cancelled: &AtomicBool) -> Result<(), String> {
    let _guard = CONFIGURE_LOCK
        .lock()
        .map_err(|_| "VST3 host setup lock was poisoned.".to_owned())?;
    let worker = worker_path()?;
    if !worker.is_file() {
        return Err(format!(
            "VST3 scan helper is missing beside Rustel: {}",
            worker.display()
        ));
    }

    rustel_runtime::vst::set_worker(worker, Vec::new());
    let folders = std::env::split_paths(OsStr::new(extra_folders))
        .filter(|folder| !folder.as_os_str().is_empty())
        .collect();
    rustel_runtime::vst::set_user_folders(folders);
    rustel_runtime::vst::scan_in_background();

    let host = rustel_runtime::vst::host();
    host.wait_idle();
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    while rustel_runtime::vst::scan_progress().is_some() {
        if cancelled.load(Ordering::Acquire) {
            return Err("VST3 plugin scan was cancelled.".into());
        }
        if Instant::now() >= deadline {
            return Err("VST3 plugin scan did not finish within ten minutes.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(windows)]
fn worker_path() -> Result<PathBuf, String> {
    let module_file = crate::portable_csound::current_module_file()
        .ok_or_else(|| "Could not locate the loaded Rustel VST3 module.".to_owned())?;
    helper_path_for_module(&module_file)
        .ok_or_else(|| "Could not locate the Rustel package folder.".to_owned())
}

#[cfg(not(windows))]
fn worker_path() -> Result<PathBuf, String> {
    Err("Nested VST3 hosting is currently packaged for Windows x64.".into())
}

#[cfg(any(windows, test))]
fn helper_path_for_module(module_file: &std::path::Path) -> Option<PathBuf> {
    Some(
        crate::portable_csound::package_directory_for_module(module_file)?
            .join("RustelVstWorker.exe"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_scan_helper_beside_the_portable_bundle() {
        let module = PathBuf::from("RustelPortable")
            .join("Rustel.vst3")
            .join("Contents")
            .join("x86_64-win")
            .join("Rustel.vst3");
        assert_eq!(
            helper_path_for_module(&module),
            Some(PathBuf::from("RustelPortable").join("RustelVstWorker.exe"))
        );
    }
}
