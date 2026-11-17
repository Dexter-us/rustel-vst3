//! Portable score file access for the plugin editor's in-app file picker.
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_SCORE_BYTES: u64 = 128 * 1024;

pub fn prepare_scores_directory() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let directory = crate::portable_csound::portable_scores_directory()
            .ok_or_else(|| "Could not locate the portable plugin folder.".to_string())?;
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Could not create the Scores folder: {error}"))?;
        Ok(directory)
    }
    #[cfg(not(windows))]
    {
        Err("Portable score files are available in the Windows plugin.".into())
    }
}

pub fn list_score_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    list_files(directory, is_supported_score, "score")
}

pub fn list_csound_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    list_files(directory, is_supported_csound, "Csound source")
}

fn list_files(
    directory: &Path,
    is_supported: fn(&Path) -> bool,
    kind: &str,
) -> Result<Vec<PathBuf>, String> {
    let entries =
        fs::read_dir(directory).map_err(|error| format!("Could not list {kind} files: {error}"))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not list {kind} files: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("Could not inspect a {kind} file: {error}"))?
            .is_file()
            && is_supported(&entry.path())
        {
            files.push(entry.path());
        }
    }
    files.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    Ok(files)
}

pub fn csound_file_path(directory: &Path, filename: &str) -> Result<PathBuf, String> {
    let filename = filename.trim();
    if filename.is_empty() {
        return Err("Enter a Csound filename.".into());
    }
    if filename.chars().any(|character| {
        matches!(
            character,
            '\\' | '/' | ':' | '<' | '>' | '"' | '|' | '?' | '*'
        )
    }) || filename.ends_with(['.', ' '])
        || filename == "."
        || filename == ".."
    {
        return Err("Enter a filename only, without a folder path.".into());
    }

    let extension = Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str());
    let filename = match extension {
        None => format!("{filename}.orc"),
        Some(extension)
            if extension.eq_ignore_ascii_case("orc") || extension.eq_ignore_ascii_case("csd") =>
        {
            filename.to_string()
        }
        Some(_) => return Err("Use a .orc or .csd filename.".into()),
    };
    Ok(directory.join(filename))
}

pub fn score_file_path(directory: &Path, filename: &str) -> Result<PathBuf, String> {
    let filename = filename.trim();
    if filename.is_empty() {
        return Err("Enter a score filename.".into());
    }
    if filename.chars().any(|character| {
        matches!(
            character,
            '\\' | '/' | ':' | '<' | '>' | '"' | '|' | '?' | '*'
        )
    }) || filename.ends_with(['.', ' '])
        || filename == "."
        || filename == ".."
    {
        return Err("Enter a filename only, without a folder path.".into());
    }

    let extension = Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str());
    let filename = match extension {
        None => format!("{filename}.strudel"),
        Some(extension)
            if extension.eq_ignore_ascii_case("strudel")
                || extension.eq_ignore_ascii_case("txt") =>
        {
            filename.to_string()
        }
        Some(_) => return Err("Use a .strudel or .txt filename.".into()),
    };
    Ok(directory.join(filename))
}

fn is_supported_score(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("strudel") || extension.eq_ignore_ascii_case("txt")
        })
}

fn is_supported_csound(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("orc") || extension.eq_ignore_ascii_case("csd")
        })
}

pub fn read_score_file(path: &std::path::Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("Could not open score: {error}"))?;
    if metadata.len() > MAX_SCORE_BYTES {
        return Err("Score files must be 128 KiB or smaller.".into());
    }
    fs::read_to_string(path).map_err(|error| format!("Could not read score: {error}"))
}

pub fn read_csound_source(path: &Path) -> Result<String, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Could not open Csound source: {error}"))?;
    if metadata.len() > MAX_SCORE_BYTES {
        return Err("Csound source files must be 128 KiB or smaller.".into());
    }
    fs::read_to_string(path).map_err(|error| format!("Could not read Csound source: {error}"))
}

pub fn write_score_file(path: &std::path::Path, source: &str) -> Result<(), String> {
    if source.len() as u64 > MAX_SCORE_BYTES {
        return Err("Score files must be 128 KiB or smaller.".into());
    }
    fs::write(path, source).map_err(|error| format!("Could not save score: {error}"))
}

pub fn write_csound_source(path: &Path, source: &str) -> Result<(), String> {
    if source.len() as u64 > MAX_SCORE_BYTES {
        return Err("Csound source files must be 128 KiB or smaller.".into());
    }
    fs::write(path, source).map_err(|error| format!("Could not save Csound source: {error}"))
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

    #[test]
    fn score_filename_defaults_to_strudel_and_rejects_folder_paths() {
        let directory = Path::new("portable").join("Scores");
        assert_eq!(
            score_file_path(&directory, "my-score").expect("valid filename"),
            directory.join("my-score.strudel")
        );
        assert_eq!(
            score_file_path(&directory, "my-score.TXT").expect("valid extension"),
            directory.join("my-score.TXT")
        );
        assert!(score_file_path(&directory, "../outside").is_err());
        assert!(score_file_path(&directory, r"folder\outside").is_err());
        assert!(score_file_path(&directory, "score.exe").is_err());
    }

    #[test]
    fn csound_filename_defaults_to_orc_and_accepts_csd() {
        let directory = Path::new("portable").join("Scores");
        assert_eq!(
            csound_file_path(&directory, "my-orchestra").expect("valid filename"),
            directory.join("my-orchestra.orc")
        );
        assert_eq!(
            csound_file_path(&directory, "my-project.CSD").expect("valid extension"),
            directory.join("my-project.CSD")
        );
        assert!(csound_file_path(&directory, "../outside").is_err());
        assert!(csound_file_path(&directory, "project.strudel").is_err());
    }

    #[test]
    fn csound_sources_round_trip_and_score_files_are_not_listed_as_csound() {
        let directory =
            std::env::temp_dir().join(format!("rustel-csound-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create temporary Csound folder");
        let orchestra = directory.join("voice.orc");
        let project = directory.join("project.csd");
        fs::write(directory.join("pattern.strudel"), "note(\"c3\")")
            .expect("write a score beside the Csound files");
        write_csound_source(&orchestra, "instr 1\nendin").expect("save orchestra");
        write_csound_source(&project, "<CsoundSynthesizer />").expect("save CSD");

        assert_eq!(
            read_csound_source(&orchestra).expect("read orchestra"),
            "instr 1\nendin"
        );
        assert_eq!(
            list_csound_files(&directory).expect("list Csound files"),
            vec![project.clone(), orchestra.clone()]
        );

        fs::remove_dir_all(directory).expect("remove temporary Csound folder");
    }
}
