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
    let entries =
        fs::read_dir(directory).map_err(|error| format!("Could not list scores: {error}"))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not list scores: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("Could not inspect a score file: {error}"))?
            .is_file()
            && is_supported_score(&entry.path())
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
}
