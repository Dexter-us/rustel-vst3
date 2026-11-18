//! User-editable labels, MIDI tags, and ordering for scanned VST3 plugins.
use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};

static CATALOG_LOCK: Mutex<()> = Mutex::new(());
const FILE_NAME: &str = "directory.tsv";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VstSort {
    #[default]
    Name,
    Type,
    Category,
    MidiFirst,
    Manual,
}

impl VstSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Type => "Type",
            Self::Category => "VST3 category",
            Self::MidiFirst => "MIDI usable first",
            Self::Manual => "Manual order",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "name" => Some(Self::Name),
            "type" => Some(Self::Type),
            "category" => Some(Self::Category),
            "midi-first" => Some(Self::MidiFirst),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Type => "type",
            Self::Category => "category",
            Self::MidiFirst => "midi-first",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginPreference {
    pub label: String,
    pub midi_source: bool,
    pub order: u64,
}

#[derive(Clone, Debug, Default)]
pub struct VstCatalog {
    pub sort: VstSort,
    preferences: HashMap<String, PluginPreference>,
}

#[derive(Clone, Debug)]
pub struct SavedPluginPreference {
    pub key: String,
    pub preference: PluginPreference,
}

impl VstCatalog {
    pub fn load() -> Result<Self, String> {
        let _guard = CATALOG_LOCK
            .lock()
            .map_err(|_| "Plugin directory settings lock was poisoned.".to_owned())?;
        Self::load_unlocked()
    }

    pub fn preference(&self, key: &str, default_order: u64) -> PluginPreference {
        self.preferences
            .get(key)
            .cloned()
            .unwrap_or(PluginPreference {
                label: String::new(),
                midi_source: false,
                order: default_order,
            })
    }

    pub fn save_directory(
        sort: VstSort,
        preferences: &[SavedPluginPreference],
    ) -> Result<(), String> {
        let _guard = CATALOG_LOCK
            .lock()
            .map_err(|_| "Plugin directory settings lock was poisoned.".to_owned())?;
        let mut catalog = Self::load_unlocked()?;
        catalog.sort = sort;
        for saved in preferences {
            catalog
                .preferences
                .insert(saved.key.clone(), saved.preference.clone());
        }
        catalog.write_unlocked()
    }

    fn load_unlocked() -> Result<Self, String> {
        let Some(file) = catalog_file() else {
            return Ok(Self::default());
        };
        let contents = match fs::read_to_string(file) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(format!("Could not read the plugin directory: {error}"));
            }
        };

        Self::parse(&contents)
    }

    fn parse(contents: &str) -> Result<Self, String> {
        let mut catalog = Self::default();
        for (line_index, line) in contents.lines().enumerate() {
            if line.is_empty() || line == "RUSTEL_VST_DIRECTORY\t1" {
                continue;
            }
            let fields = line.split('\t').map(unescape_field).collect::<Vec<_>>();
            match fields.first().map(String::as_str) {
                Some("sort") if fields.len() == 2 => {
                    catalog.sort = VstSort::from_str(&fields[1]).ok_or_else(|| {
                        format!("Unknown plugin directory sort on line {}.", line_index + 1)
                    })?;
                }
                Some("plugin") if fields.len() == 5 => {
                    let midi_source = match fields[3].as_str() {
                        "0" => false,
                        "1" => true,
                        _ => {
                            return Err(format!(
                                "Invalid MIDI tag in plugin directory on line {}.",
                                line_index + 1
                            ));
                        }
                    };
                    let order = fields[4]
                        .parse::<u64>()
                        .map_err(|_| format!("Invalid plugin order on line {}.", line_index + 1))?;
                    catalog.preferences.insert(
                        fields[1].clone(),
                        PluginPreference {
                            label: fields[2].clone(),
                            midi_source,
                            order,
                        },
                    );
                }
                _ => {
                    return Err(format!(
                        "Invalid plugin directory entry on line {}.",
                        line_index + 1
                    ));
                }
            }
        }
        Ok(catalog)
    }

    fn serialize(&self) -> String {
        let mut contents = format!("RUSTEL_VST_DIRECTORY\t1\nsort\t{}\n", self.sort.key());
        let mut preferences = self.preferences.iter().collect::<Vec<_>>();
        preferences.sort_by(|left, right| left.0.cmp(right.0));
        for (key, preference) in preferences {
            contents.push_str(&format!(
                "plugin\t{}\t{}\t{}\t{}\n",
                escape_field(key),
                escape_field(&preference.label),
                u8::from(preference.midi_source),
                preference.order
            ));
        }
        contents
    }

    fn write_unlocked(&self) -> Result<(), String> {
        let file = catalog_file()
            .ok_or_else(|| "Could not locate Rustel's shared data folder.".to_owned())?;
        let parent = file
            .parent()
            .ok_or_else(|| "Could not locate the plugin directory settings folder.".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the plugin directory folder: {error}"))?;

        fs::write(file, self.serialize())
            .map_err(|error| format!("Could not save the plugin directory: {error}"))
    }
}

fn catalog_file() -> Option<PathBuf> {
    Some(
        std::env::var_os("RUSTEL_CONFIG_DIR")
            .map(PathBuf::from)?
            .join("vst")
            .join(FILE_NAME),
    )
}

fn escape_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape_field(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match characters.next() {
            Some('t') => unescaped.push('\t'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            Some('\\') => unescaped.push('\\'),
            Some(other) => {
                unescaped.push('\\');
                unescaped.push(other);
            }
            None => unescaped.push('\\'),
        }
    }
    unescaped
}

#[cfg(test)]
mod tests {
    use super::{PluginPreference, VstCatalog, VstSort, escape_field, unescape_field};

    #[test]
    fn escaped_directory_fields_round_trip() {
        let value = "Keys\tPiano\\Synth\nLine 2";
        assert_eq!(unescape_field(&escape_field(value)), value);
    }

    #[test]
    fn sort_modes_have_stable_labels_and_keys() {
        for sort in [
            VstSort::Name,
            VstSort::Type,
            VstSort::Category,
            VstSort::MidiFirst,
            VstSort::Manual,
        ] {
            assert!(!sort.label().is_empty());
            assert_eq!(VstSort::from_str(sort.key()), Some(sort));
        }
    }

    #[test]
    fn preferences_keep_custom_labels_midi_tags_and_manual_order() {
        let mut catalog = VstCatalog {
            sort: VstSort::Manual,
            ..VstCatalog::default()
        };
        catalog.preferences.insert(
            "class-id".into(),
            PluginPreference {
                label: "My Arp".into(),
                midi_source: true,
                order: 42,
            },
        );
        let loaded = VstCatalog::parse(&catalog.serialize()).expect("saved directory should parse");
        let preference = loaded.preference("class-id", 0);
        assert_eq!(loaded.sort, VstSort::Manual);
        assert_eq!(preference.label, "My Arp");
        assert!(preference.midi_source);
        assert_eq!(preference.order, 42);
        assert_eq!(VstCatalog::default().sort, VstSort::Name);
    }
}
