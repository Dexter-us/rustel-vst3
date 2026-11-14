//! Native score file dialogs for the plugin editor.
use std::{fs, path::PathBuf};

const MAX_SCORE_BYTES: u64 = 128 * 1024;

#[cfg(windows)]
pub fn open_score() -> Result<Option<PathBuf>, String> {
    show_dialog(false)
}

#[cfg(windows)]
pub fn save_score() -> Result<Option<PathBuf>, String> {
    show_dialog(true)
}

#[cfg(not(windows))]
pub fn open_score() -> Result<Option<PathBuf>, String> {
    Err("Score file dialogs are available in the Windows plugin.".into())
}

#[cfg(not(windows))]
pub fn save_score() -> Result<Option<PathBuf>, String> {
    Err("Score file dialogs are available in the Windows plugin.".into())
}

pub fn read_score_file(path: &std::path::Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("Could not open score: {error}"))?;
    if metadata.len() > MAX_SCORE_BYTES {
        return Err("Score files must be 128 KiB or smaller.".into());
    }
    fs::read_to_string(path).map_err(|error| format!("Could not read score: {error}"))
}

pub fn write_score_file(path: &std::path::Path, source: &str) -> Result<(), String> {
    if source.len() as u64 > MAX_SCORE_BYTES {
        return Err("Score files must be 128 KiB or smaller.".into());
    }
    fs::write(path, source).map_err(|error| format!("Could not save score: {error}"))
}

#[cfg(windows)]
fn show_dialog(save: bool) -> Result<Option<PathBuf>, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr;

    const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
    const OFN_HIDEREADONLY: u32 = 0x0000_0004;
    const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
    const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
    const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
    const OFN_EXPLORER: u32 = 0x0008_0000;

    let filter: Vec<u16> =
        "Rustel scores (*.strudel;*.txt)\0*.strudel;*.txt\0Text files (*.txt)\0*.txt\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let title: Vec<u16> = if save {
        "Save Rustel score\0"
    } else {
        "Load Rustel score\0"
    }
    .encode_utf16()
    .collect();
    let default_extension: Vec<u16> = "strudel\0".encode_utf16().collect();
    let initial_directory: Vec<u16> = crate::portable_csound::portable_scores_directory()
        .and_then(|directory| {
            if fs::create_dir_all(&directory).is_ok() && directory.is_dir() {
                Some(directory)
            } else {
                None
            }
        })
        .map(|directory| directory.as_os_str().encode_wide().chain(Some(0)).collect())
        .unwrap_or_default();
    let initial_directory_pointer = if initial_directory.is_empty() {
        ptr::null()
    } else {
        initial_directory.as_ptr()
    };
    let mut file = vec![0u16; 32_768];
    let flags = OFN_EXPLORER
        | OFN_HIDEREADONLY
        | OFN_NOCHANGEDIR
        | OFN_PATHMUSTEXIST
        | if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST
        };
    let mut dialog = OpenFileNameW {
        size: std::mem::size_of::<OpenFileNameW>() as u32,
        owner: ptr::null_mut(),
        instance: ptr::null_mut(),
        filter: filter.as_ptr(),
        custom_filter: ptr::null_mut(),
        max_custom_filter: 0,
        filter_index: 1,
        file: file.as_mut_ptr(),
        max_file: file.len() as u32,
        file_title: ptr::null_mut(),
        max_file_title: 0,
        initial_directory: initial_directory_pointer,
        title: title.as_ptr(),
        flags,
        file_offset: 0,
        file_extension: 0,
        default_extension: default_extension.as_ptr(),
        custom_data: 0,
        hook: ptr::null_mut(),
        template_name: ptr::null(),
        reserved: ptr::null_mut(),
        reserved_value: 0,
        flags_ex: 0,
    };

    // SAFETY: All pointers in `dialog` point to live buffers or are null, and
    // the file buffer has the size declared in `max_file`.
    let accepted = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if accepted == 0 {
        // A zero error code means the user cancelled the dialog.
        // SAFETY: This API has no arguments and does not retain any pointers.
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(format!("Windows file dialog failed (error 0x{error:08x})."))
        };
    }

    let length = file
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(file.len());
    if length == 0 || length == file.len() {
        return Err("The selected score path could not be read.".into());
    }
    Ok(Some(PathBuf::from(OsString::from_wide(&file[..length]))))
}

#[cfg(windows)]
#[repr(C)]
struct OpenFileNameW {
    size: u32,
    owner: *mut std::ffi::c_void,
    instance: *mut std::ffi::c_void,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_directory: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    default_extension: *const u16,
    custom_data: isize,
    hook: *mut std::ffi::c_void,
    template_name: *const u16,
    reserved: *mut std::ffi::c_void,
    reserved_value: u32,
    flags_ex: u32,
}

#[cfg(windows)]
#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(dialog: *mut OpenFileNameW) -> i32;
    fn GetSaveFileNameW(dialog: *mut OpenFileNameW) -> i32;
    fn CommDlgExtendedError() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_score(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rustel-score-{}-{name}.strudel",
            std::process::id()
        ))
    }

    #[test]
    fn saves_and_loads_a_score_file() {
        let path = temporary_score("round-trip");
        let source = "note(\"c3 e3 g3\").csound(\"triangle\")";
        write_score_file(&path, source).expect("save score");
        assert_eq!(read_score_file(&path).expect("load score"), source);
        fs::remove_file(path).expect("remove temporary score");
    }

    #[test]
    fn score_file_size_is_bounded() {
        let path = temporary_score("too-large");
        let source = "x".repeat(MAX_SCORE_BYTES as usize + 1);
        assert!(write_score_file(&path, &source).is_err());
        fs::write(&path, source).expect("write oversized fixture");
        assert!(read_score_file(&path).is_err());
        fs::remove_file(path).expect("remove temporary score");
    }
}
