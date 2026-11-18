//! Launch the portable helper that hosts a free-running MIDI VST3 source.
use std::{path::Path, path::PathBuf, process::Child};

pub(crate) fn launch(
    bundle: &Path,
    plugin_name: &str,
    stream_port: u16,
    stream_token: &str,
) -> Result<(Child, PathBuf), String> {
    let helper = helper_path()?;
    if !helper.is_file() {
        return Err(format!(
            "The Rustel MIDI host helper is missing beside the portable package: {}",
            helper.display()
        ));
    }
    if !bundle.is_dir() || plugin_name.trim().is_empty() {
        return Err("Choose a scanned MIDI VST3 plugin before opening its editor.".into());
    }

    let config_path = std::env::temp_dir().join(format!(
        "rustel-midi-host-{}-{stream_port}.json",
        std::process::id()
    ));
    let config = serde_json::json!({
        "plugin_path": bundle.to_string_lossy().into_owned(),
        "plugin_name": plugin_name,
        "input_midi_path": "",
        "output_midi_path": "",
        "status_path": "",
        "duration_seconds": 0,
        "ticks_per_beat": 480,
        "tempo_microseconds": 500000,
        "stream_host": "127.0.0.1",
        "stream_port": stream_port,
        "stream_token": stream_token,
    });
    let encoded = serde_json::to_vec(&config)
        .map_err(|error| format!("Could not prepare the MIDI host settings: {error}"))?;
    std::fs::write(&config_path, encoded)
        .map_err(|error| format!("Could not write the MIDI host settings: {error}"))?;

    match std::process::Command::new(&helper)
        .arg(&config_path)
        .spawn()
    {
        Ok(child) => Ok((child, config_path)),
        Err(error) => {
            let _ = std::fs::remove_file(&config_path);
            Err(format!("Could not open the MIDI VST editor: {error}"))
        }
    }
}

#[cfg(windows)]
fn helper_path() -> Result<PathBuf, String> {
    let module = crate::portable_csound::current_module_file()
        .ok_or_else(|| "Could not locate the loaded Rustel VST3 module.".to_owned())?;
    crate::portable_csound::package_directory_for_module(&module)
        .map(|directory| directory.join("RustelMidiHost.exe"))
        .ok_or_else(|| "Could not locate the Rustel portable package folder.".to_owned())
}

#[cfg(not(windows))]
fn helper_path() -> Result<PathBuf, String> {
    Err("The live MIDI VST editor helper is currently packaged for Windows x64.".into())
}
