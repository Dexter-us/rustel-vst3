use std::{
    ffi::OsStr,
    fs,
    path::PathBuf,
    sync::Mutex,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static CONFIGURE_LOCK: Mutex<()> = Mutex::new(());
static FOLDERS_LOCK: Mutex<()> = Mutex::new(());

/// Read shared VST3 folder choices; `None` means they have not been saved yet.
fn read_user_folders() -> Result<Option<Vec<PathBuf>>, String> {
    let Some(file) = user_folders_file() else {
        return Ok(None);
    };
    let contents = match fs::read_to_string(file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Could not read Rustel's VST3 folder choices: {error}"
            ));
        }
    };
    Ok(Some(
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
    ))
}

/// Use shared paths when available, or migrate paths from an older plugin state.
pub(crate) fn shared_folder_setting(extra_folders: &str) -> Result<String, String> {
    let folders = match read_user_folders()? {
        Some(folders) => folders,
        None => {
            let folders = split_folders(extra_folders);
            if !folders.is_empty() {
                save_user_folders(extra_folders)?;
            }
            folders
        }
    };
    std::env::join_paths(folders)
        .map(|paths| paths.to_string_lossy().into_owned())
        .map_err(|error| format!("Could not combine VST3 folders: {error}"))
}

/// Store the current folder choices for every Rustel instance in this package.
pub(crate) fn save_user_folders(extra_folders: &str) -> Result<(), String> {
    let _guard = FOLDERS_LOCK
        .lock()
        .map_err(|_| "VST3 folder settings lock was poisoned.".to_owned())?;
    let file = user_folders_file()
        .ok_or_else(|| "Could not locate Rustel's shared data folder.".to_owned())?;
    let parent = file
        .parent()
        .ok_or_else(|| "Could not locate Rustel's shared VST3 settings folder.".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Rustel's VST3 settings folder: {error}"))?;

    let mut contents = String::new();
    for folder in split_folders(extra_folders) {
        contents.push_str(&folder.to_string_lossy());
        contents.push('\n');
    }
    fs::write(file, contents)
        .map_err(|error| format!("Could not save Rustel's VST3 folder choices: {error}"))
}

fn split_folders(value: &str) -> Vec<PathBuf> {
    std::env::split_paths(OsStr::new(value))
        .filter(|folder| !folder.as_os_str().is_empty())
        .collect()
}

fn user_folders_file() -> Option<PathBuf> {
    Some(
        std::env::var_os("RUSTEL_CONFIG_DIR")
            .map(PathBuf::from)?
            .join("vst")
            .join("folders.txt"),
    )
}

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
    let folders = read_user_folders()?.unwrap_or_else(|| split_folders(extra_folders));
    rustel_runtime::vst::set_user_folders(folders);
    rustel_runtime::vst::scan_in_background();

    let host = rustel_runtime::vst::host();
    host.wait_idle();
    let deadline = Instant::now() + Duration::from_secs(2 * 60 * 60);
    while rustel_runtime::vst::scan_progress().is_some() {
        if cancelled.load(Ordering::Acquire) {
            return Err("VST3 plugin scan was cancelled.".into());
        }
        if Instant::now() >= deadline {
            return Err("VST3 plugin scan did not finish within two hours.".into());
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
