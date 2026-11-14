//! Find the Csound runtime shipped beside the VST3 bundle.
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
fn runtime_directory_for_module(module_file: &Path) -> Option<PathBuf> {
    let architecture_directory = module_file.parent()?;
    let contents_directory = architecture_directory.parent()?;
    let bundle_directory = contents_directory.parent()?;
    let package_directory = bundle_directory.parent()?;
    Some(package_directory.join("CsoundRuntime"))
}

#[cfg(windows)]
pub fn configure() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Once;

    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        if std::env::var_os("RUSTEL_CSOUND_LIBRARY").is_some() {
            return;
        }

        let Some(module_file) = current_module_file() else {
            return;
        };
        let Some(runtime_directory) = runtime_directory_for_module(&module_file) else {
            return;
        };
        let library = runtime_directory.join("csound64.dll");
        if !library.is_file() || !runtime_directory.join("stdutil.dll").is_file() {
            return;
        }

        // These settings affect only the DAW process. Existing explicit Csound
        // library paths are respected, and no Windows user settings are changed.
        set_process_variable("RUSTEL_CSOUND_LIBRARY", library.as_os_str());
        if std::env::var_os("OPCODE6DIR64").is_none() {
            set_process_variable("OPCODE6DIR64", runtime_directory.as_os_str());
        }

        let runtime_text = runtime_directory.to_string_lossy();
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let path_contains_runtime = current_path
            .to_string_lossy()
            .split(';')
            .any(|entry| entry.eq_ignore_ascii_case(&runtime_text));
        if !path_contains_runtime {
            let mut updated_path: OsString = current_path;
            if !updated_path.is_empty() {
                updated_path.push(";");
            }
            updated_path.push(runtime_directory.as_os_str());
            set_process_variable("PATH", &updated_path);
        }
    });
}

#[cfg(not(windows))]
pub fn configure() {}

#[cfg(windows)]
fn current_module_file() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x0000_0002;

    let mut module = std::ptr::null_mut();
    let address = current_module_file as *const () as *const u16;
    // SAFETY: FROM_ADDRESS interprets the second argument as an address in
    // this loaded plugin module; the output pointer is valid for the call.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address,
            &mut module,
        )
    };
    if found == 0 {
        return None;
    }

    let mut buffer = vec![0u16; 32_768];
    // SAFETY: `buffer` is writable and its capacity is passed as the length.
    let length = unsafe {
        GetModuleFileNameW(
            module,
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).ok()?,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    buffer.truncate(length);
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
}

#[cfg(windows)]
fn set_process_variable(name: &str, value: &std::ffi::OsStr) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let name: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
    let value: Vec<u16> = value.encode_wide().chain(Some(0)).collect();
    // SAFETY: Both strings are NUL-terminated UTF-16 buffers that remain live
    // for the duration of the Windows API call.
    unsafe { SetEnvironmentVariableW(name.as_ptr(), value.as_ptr()) != 0 }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleExW(
        flags: u32,
        module_name_or_address: *const u16,
        module: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn GetModuleFileNameW(module: *mut std::ffi::c_void, filename: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_runtime_beside_the_vst3_bundle() {
        let module = PathBuf::from("RustelPortable")
            .join("Rustel.vst3")
            .join("Contents")
            .join("x86_64-win")
            .join("Rustel.vst3");

        assert_eq!(
            runtime_directory_for_module(&module),
            Some(PathBuf::from("RustelPortable").join("CsoundRuntime"))
        );
    }

    #[test]
    fn missing_module_parent_is_not_a_portable_package() {
        assert_eq!(runtime_directory_for_module(Path::new("Rustel.vst3")), None);
    }
}
