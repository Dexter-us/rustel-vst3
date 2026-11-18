use crate::{engine::Shared, params::RustelParams};
use nih_plug::prelude::*;
use nih_plug_egui::{
    EguiState, create_egui_editor,
    egui::{self, Color32, FontId, RichText, TextEdit},
    widgets,
};
use std::{sync::Arc, sync::atomic::Ordering, time::Duration};

#[derive(Default)]
struct ScoreFileUi {
    message: String,
    dialog: Option<ScoreDialog>,
    folder_picker: Option<FolderPicker>,
    csound_editor: Option<CsoundSourceUi>,
    vst_folders_loaded: bool,
    editor_selection: Option<std::ops::Range<usize>>,
    pending_insert: Option<String>,
    plugin_directory_open: bool,
    plugin_search: String,
}

struct FolderPicker {
    directory: Option<std::path::PathBuf>,
    directories: Vec<std::path::PathBuf>,
    selected: Option<std::path::PathBuf>,
    error: Option<String>,
}

struct CsoundSourceUi {
    directory: std::path::PathBuf,
    files: Vec<std::path::PathBuf>,
    selected: Option<std::path::PathBuf>,
    filename: String,
    source: String,
    message: String,
    error: Option<String>,
    confirm_overwrite: bool,
}

enum CsoundSourceAction {
    Close,
    Load(std::path::PathBuf),
    Save(std::path::PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScoreDialogMode {
    Load,
    Save,
}

struct ScoreDialog {
    mode: ScoreDialogMode,
    directory: std::path::PathBuf,
    files: Vec<std::path::PathBuf>,
    selected: Option<std::path::PathBuf>,
    filename: String,
    confirm_overwrite: bool,
    error: Option<String>,
}

enum ScoreDialogAction {
    Cancel,
    Load(std::path::PathBuf),
    Save(std::path::PathBuf),
}

pub fn create(params: Arc<RustelParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let state: Arc<EguiState> = params.editor.clone();
    create_egui_editor(
        state,
        ScoreFileUi::default(),
        |ctx, _| {
            ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*ctx.style()).clone();
            style.visuals.selection.bg_fill = Color32::from_rgb(87, 56, 125);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(112, 75, 164);
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            ctx.set_style(style);
        },
        move |ctx, setter, file_ui| {
            ctx.request_repaint_after(Duration::from_millis(50));
            if !file_ui.vst_folders_loaded {
                file_ui.vst_folders_loaded = true;
                let current = params.vst3_folders.lock().clone();
                match crate::portable_vst::shared_folder_setting(&current) {
                    Ok(shared) if shared != current => {
                        *params.vst3_folders.lock() = shared;
                        mark_dirty(setter, &params);
                    }
                    Ok(_) => {}
                    Err(error) => file_ui.message = error,
                }
            }
            egui::TopBottomPanel::top("header").show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("rustel")
                            .size(28.0)
                            .strong()
                            .color(Color32::from_rgb(199, 167, 244)),
                    );
                    ui.small(format!("v{}", env!("CARGO_PKG_VERSION")));
                    ui.label("PATTERN INSTRUMENT");
                    ui.separator();
                    let bpm = f64::from_bits(shared.bpm.load(Ordering::Relaxed));
                    let beat = f64::from_bits(shared.beat.load(Ordering::Relaxed));
                    ui.label(format!("{bpm:.1} BPM  ·  Beat {beat:.2}"));
                });
                ui.add_space(6.0);
            });
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(shared.status.lock().as_str());
                let peak = shared.peak.load(Ordering::Relaxed);
                let db = if peak > 0.00001 {
                    format!("{:.1} dBFS", 20.0 * peak.log10())
                } else {
                    "−∞ dBFS".into()
                };
                ui.horizontal(|ui| {
                    ui.label(format!("Output {db}"));
                    let refused = shared.refused.load(Ordering::Relaxed);
                    if refused > 0 {
                        ui.colored_label(Color32::YELLOW, format!("{refused} notes exceeded capacity"));
                    }
                });
                ui.small("VST3 instruments and effects are hosted by Rustel and rendered into the repeating phrase.");
                ui.small("DAW MIDI uses the score's first oscillator sound, or sine by default.");
                ui.add_space(6.0);
            });
            egui::SidePanel::right("controls")
                .exact_width(195.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.heading("Transport");
                    toggle(ui, setter, &params.enabled, "Enabled");
                    toggle(ui, setter, &params.follow_host, "Follow DAW");
                    toggle(ui, setter, &params.allow_csound, "Allow native Csound");
                    ui.small("Trusted orchestra code can access files and run processes. Only enable scores you trust.");
                    ui.label("Trusted .orc folder (optional)");
                    let mut orchestra_root = params.csound_orchestra_root.lock().clone();
                    if ui
                        .add(
                            TextEdit::singleline(&mut orchestra_root)
                                .hint_text("Full path to a trusted folder"),
                        )
                        .changed()
                    {
                        *params.csound_orchestra_root.lock() = orchestra_root;
                        mark_dirty(setter, &params);
                    }
                    if !params.follow_host.value() {
                        ui.small("Free-run preview. Your DAW must still process audio.");
                    }
                    ui.separator();
                    ui.label("Output");
                    ui.add(widgets::ParamSlider::for_param(&params.gain, setter));
                    ui.separator();
                    ui.heading("Phrase");
                    ui.label("Cycles before repeating");
                    ui.add(widgets::ParamSlider::for_param(&params.cycles, setter));
                    ui.label("Quarter-note beats per cycle");
                    ui.add(widgets::ParamSlider::for_param(&params.beats_per_cycle, setter));
                    ui.small("The score is evaluated across this phrase, then repeats. Longer evolving patterns need a longer phrase.");
                    ui.separator();
                    ui.heading("VST3 plugins");
                    ui.small(
                        "Scans the standard Windows VST3 locations and any folders you add below.",
                    );
                    for folder in standard_vst3_folders() {
                        ui.small(folder.display().to_string());
                    }
                    ui.label("Additional VST3 folders (optional)");
                    let mut vst3_folders = params.vst3_folders.lock().clone();
                    let folder_response = ui.add(
                        TextEdit::singleline(&mut vst3_folders)
                            .hint_text("Folder path; multiple paths use the OS separator"),
                    );
                    if folder_response.changed() {
                        *params.vst3_folders.lock() = vst3_folders.clone();
                        mark_dirty(setter, &params);
                    }
                    if folder_response.lost_focus() {
                        if let Err(error) =
                            crate::portable_vst::save_user_folders(&vst3_folders)
                        {
                            file_ui.message = error;
                        }
                    }
                    ui.small("Custom folders are shared by Rustel instances in this package and saved in RustelData/vst/folders.txt.");
                    if ui.button("Browse for a plugin folder…").clicked() {
                        start_folder_picker(file_ui);
                    }
                    ui.small(
                        "The bundled RustelVstWorker checks each plugin separately. It should not open a console window.",
                    );
                    if ui.button("Scan VST3 plugins").clicked() {
                        shared.scan_vst_plugins.store(true, Ordering::Release);
                    }
                    if let Some(progress) = rustel_runtime::vst::scan_progress() {
                        let current_bundle = progress.read.saturating_add(1).min(progress.bundles);
                        ui.colored_label(
                            Color32::LIGHT_BLUE,
                            format!(
                                "Scanning: {} — bundle {current_bundle}/{}",
                                progress.plugin, progress.bundles
                            ),
                        );
                    }
                    ui.small("Each plugin class has a 30-second limit; the full scan can run up to two hours. Results persist in RustelData/vst/scan.json; delete that file to retry an unchanged bundle.");
                    let plugin_count = shared.vst_plugins.lock().len();
                    ui.horizontal(|ui| {
                        ui.label(format!("Scanned plugins: {plugin_count}"));
                        if ui.button("Open plugin list…").clicked() {
                            file_ui.plugin_directory_open = true;
                        }
                    });
                    ui.small("The full directory opens in a searchable popup grouped by instrument/effect and VST3 category.");
                    ui.separator();
                    ui.label("Examples");
                    for (name, source) in [
                        ("Minor arpeggio", crate::params::DEFAULT_SCORE),
                        ("Soft pulse", r#"note("c3 ~ eb3 g3").s("triangle").lpf(1200).gain(0.3)"#),
                        ("Kick + bass", r#"stack(s("bd*4").gain(0.6), note("c2 ~ g1 ~").s("sine").gain(0.35))"#),
                        (
                            "Csound triangle",
                            r#"note("c3 eb3 g3 bb3").csound("triangle").gain(0.4).slow(2)"#,
                        ),
                    ] {
                        if ui.button(name).clicked() {
                            *params.draft.lock() = source.into();
                            mark_dirty(setter, &params);
                        }
                    }
                    ui.add_space(12.0);
                    ui.hyperlink_to(
                        "Source & build instructions",
                        "https://github.com/Dexter-us/rustel-vst3",
                    );
                    });
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Score");
                    if ui
                        .button("Load…")
                        .on_hover_text("Load a .strudel or .txt score")
                        .clicked()
                    {
                        start_score_dialog(file_ui, ScoreDialogMode::Load);
                    }
                    if ui
                        .button("Save…")
                        .on_hover_text("Save the current draft")
                        .clicked()
                    {
                        start_score_dialog(file_ui, ScoreDialogMode::Save);
                    }
                    if ui
                        .button("Csound files…")
                        .on_hover_text("Open the separate .orc/.csd source editor")
                        .clicked()
                    {
                        start_csound_editor(file_ui);
                    }
                    if ui
                        .button(RichText::new("Evaluate").strong())
                        .on_hover_text("Ctrl+Enter · invalid edits keep the last good phrase")
                        .clicked()
                        || ctx.input(|input| {
                            input.modifiers.ctrl && input.key_pressed(egui::Key::Enter)
                        })
                    {
                        shared.requested.fetch_add(1, Ordering::Release);
                        mark_dirty(setter, &params);
                    }
                });
                ui.small("Place the caret in the score, then use a plugin's Insert button. Load and Save use .strudel or .txt files.");
                if !file_ui.message.is_empty() {
                    ui.colored_label(Color32::LIGHT_BLUE, &file_ui.message);
                }
                let mut draft = params.draft.lock().clone();
                let mut changed = false;
                let mut inserted_cursor = None;
                if let Some(snippet) = file_ui.pending_insert.take() {
                    let selection = file_ui.editor_selection.clone().unwrap_or_else(|| {
                        let end = draft.chars().count();
                        end..end
                    });
                    let (updated, cursor) =
                        insert_code_at_char_range(&draft, selection, &snippet);
                    draft = updated;
                    inserted_cursor = Some(cursor);
                    changed = true;
                }
                egui::ScrollArea::both().show(ui, |ui| {
                    let mut output = TextEdit::multiline(&mut draft)
                        .id(egui::Id::new("rustel-score-editor-text"))
                        .font(FontId::monospace(15.0))
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .show(ui);
                    if let Some(cursor) = inserted_cursor {
                        output.state.cursor.set_char_range(Some(
                            egui::text::CCursorRange::one(egui::text::CCursor::new(cursor)),
                        ));
                        output.state.store(ui.ctx(), output.response.id);
                        file_ui.editor_selection = Some(cursor..cursor);
                    } else if let Some(cursor_range) = output.cursor_range {
                        file_ui.editor_selection =
                            Some(cursor_range.as_sorted_char_range());
                    }
                    if output.response.changed() {
                        changed = true;
                    }
                });
                if changed {
                    *params.draft.lock() = draft;
                    mark_dirty(setter, &params);
                }
            });
            draw_plugin_directory(ctx, file_ui, &shared);
            draw_score_dialog(ctx, file_ui, &params, setter);
            draw_folder_picker(ctx, file_ui, &params, setter);
            draw_csound_editor(ctx, file_ui);
        },
    )
}

fn insert_code_at_char_range(
    source: &str,
    selection: std::ops::Range<usize>,
    snippet: &str,
) -> (String, usize) {
    let char_count = source.chars().count();
    let start = selection.start.min(char_count);
    let end = selection.end.min(char_count).max(start);
    let byte_at = |char_index: usize| {
        source
            .char_indices()
            .nth(char_index)
            .map_or(source.len(), |(byte, _)| byte)
    };
    let start_byte = byte_at(start);
    let end_byte = byte_at(end);
    let before = &source[..start_byte];
    let after = &source[end_byte..];

    let mut insertion = String::new();
    if !before.is_empty() && !before.ends_with('\n') {
        insertion.push('\n');
    }
    insertion.push_str(snippet.trim());
    if !after.is_empty() && !after.starts_with('\n') {
        insertion.push('\n');
    }

    let mut result = String::with_capacity(source.len() + insertion.len());
    result.push_str(before);
    result.push_str(&insertion);
    result.push_str(after);
    (result, start + insertion.chars().count())
}

fn quoted_code_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if character.is_control() => quoted.push(' '),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn plugin_example(name: &str, instrument: bool) -> String {
    let method = if instrument { "vsti" } else { "vst" };
    format!("note(\"c3 e3 g3\").{method}({})", quoted_code_string(name))
}

fn plugin_midi_example(name: &str, instrument: bool, target: Option<&str>) -> String {
    let method = if instrument { "vsti" } else { "vst" };
    let target = match target {
        Some(target) => format!("vsti {}", quoted_code_string(target)),
        None => "rustel \"sine\"".into(),
    };
    format!(
        "// MIDI target: {target}\nnote(\"c4\").{method}({})",
        quoted_code_string(name)
    )
}

fn draw_plugin_directory(ctx: &egui::Context, file_ui: &mut ScoreFileUi, shared: &Shared) {
    if !file_ui.plugin_directory_open {
        return;
    }

    let mut open = file_ui.plugin_directory_open;
    let mut close_after_insert = false;
    egui::Window::new("VST3 Plugin Directory")
        .open(&mut open)
        .default_size(egui::vec2(760.0, 620.0))
        .resizable(true)
        .show(ctx, |ui| {
            let mut plugins = shared.vst_plugins.lock().clone();
            let plugin_message = shared.vst_plugins_message.lock().clone();
            let saved_sort = *shared.vst_sort.lock();
            let mut sort = saved_sort;

            ui.horizontal(|ui| {
                ui.strong(format!("{} scanned", plugins.len()));
                ui.add(
                    TextEdit::singleline(&mut file_ui.plugin_search)
                        .desired_width(260.0)
                        .hint_text("Search name, vendor, category, MIDI tag"),
                );
                egui::ComboBox::from_id_salt("rustel-vst-directory-sort")
                    .selected_text(sort.label())
                    .show_ui(ui, |ui| {
                        for option in [
                            crate::vst_catalog::VstSort::Name,
                            crate::vst_catalog::VstSort::Type,
                            crate::vst_catalog::VstSort::Category,
                            crate::vst_catalog::VstSort::MidiFirst,
                            crate::vst_catalog::VstSort::Manual,
                        ] {
                            ui.selectable_value(&mut sort, option, option.label());
                        }
                    });
            });
            if sort != saved_sort {
                *shared.vst_sort.lock() = sort;
            }
            sort_vst_plugins(&mut plugins, sort);
            let total_plugins = plugins.len();

            let query = file_ui.plugin_search.trim().to_ascii_lowercase();
            let visible_indices = plugins
                .iter()
                .enumerate()
                .filter_map(|(index, plugin)| {
                    plugin_matches_search(plugin, &query).then_some(index)
                })
                .collect::<Vec<_>>();
            ui.small(format!(
                "Showing {} of {}. VST3 categories are grouped below; your MIDI tags are marked separately.",
                visible_indices.len(),
                plugins.len()
            ));
            let targets = plugins
                .iter()
                .filter(|plugin| plugin.instrument)
                .cloned()
                .collect::<Vec<_>>();
            let mut directory_changed = sort != saved_sort;
            let mut reorder = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if plugins.is_empty() {
                        ui.colored_label(Color32::LIGHT_BLUE, &plugin_message);
                        ui.small("After scanning, open this list to browse all found instruments and effects.");
                    } else if visible_indices.is_empty() {
                        ui.label("No plugins match that search.");
                    } else if matches!(
                        sort,
                        crate::vst_catalog::VstSort::Manual
                            | crate::vst_catalog::VstSort::MidiFirst
                    ) {
                        let can_reorder = sort == crate::vst_catalog::VstSort::Manual
                            && query.is_empty();
                        for &index in &visible_indices {
                            draw_plugin_row(
                                ui,
                                &mut plugins[index],
                                &targets,
                                index,
                                total_plugins,
                                sort,
                                can_reorder,
                                &mut directory_changed,
                                &mut reorder,
                                &mut close_after_insert,
                                file_ui,
                            );
                        }
                        if sort == crate::vst_catalog::VstSort::Manual && !query.is_empty() {
                            ui.small("Clear the search to rearrange the full manual list.");
                        }
                    } else {
                        let mut groups =
                            std::collections::BTreeMap::<
                                String,
                                std::collections::BTreeMap<String, Vec<usize>>,
                            >::new();
                        for &index in &visible_indices {
                            let (type_group, category_group) =
                                plugin_category_group(&plugins[index]);
                            groups
                                .entry(type_group)
                                .or_default()
                                .entry(category_group)
                                .or_default()
                                .push(index);
                        }
                        for (type_group, categories) in groups {
                            let group_count = categories.values().map(Vec::len).sum::<usize>();
                            egui::CollapsingHeader::new(format!(
                                "{type_group} ({group_count})"
                            ))
                            .default_open(true)
                            .show(ui, |ui| {
                                for (category, indices) in categories {
                                    egui::CollapsingHeader::new(format!(
                                        "{category} ({})",
                                        indices.len()
                                    ))
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        for index in indices {
                                            draw_plugin_row(
                                                ui,
                                                &mut plugins[index],
                                                &targets,
                                                index,
                                                total_plugins,
                                                sort,
                                                false,
                                                &mut directory_changed,
                                                &mut reorder,
                                                &mut close_after_insert,
                                                file_ui,
                                            );
                                        }
                                    });
                                }
                            });
                        }
                    }

                    if let Some((index, direction)) = reorder {
                        let next = if direction < 0 {
                            index.saturating_sub(1)
                        } else {
                            (index + 1).min(plugins.len().saturating_sub(1))
                        };
                        if next != index {
                            plugins.swap(index, next);
                            for (position, plugin) in plugins.iter_mut().enumerate() {
                                plugin.order = position as u64 * 1024;
                            }
                            directory_changed = true;
                        }
                    }
                });

            if directory_changed {
                *shared.vst_plugins.lock() = plugins.clone();
                if let Err(error) = save_plugin_directory(sort, &plugins) {
                    file_ui.message = error;
                }
            }
        });
    file_ui.plugin_directory_open = open && !close_after_insert;
}

fn draw_plugin_row(
    ui: &mut egui::Ui,
    plugin: &mut crate::engine::VstPluginEntry,
    targets: &[crate::engine::VstPluginEntry],
    index: usize,
    total: usize,
    sort: crate::vst_catalog::VstSort,
    can_reorder: bool,
    directory_changed: &mut bool,
    reorder: &mut Option<(usize, i8)>,
    close_after_insert: &mut bool,
    file_ui: &mut ScoreFileUi,
) {
    ui.group(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong(plugin_display_label(plugin));
            ui.small(if plugin.instrument {
                "Instrument"
            } else {
                "Effect / processor"
            });
            if plugin.midi_source {
                ui.small("MIDI");
            }
        });
        let vendor = if plugin.vendor.is_empty() {
            "Vendor unavailable"
        } else {
            &plugin.vendor
        };
        let category = if plugin.categories.is_empty() {
            "VST3 category unavailable"
        } else {
            &plugin.categories
        };
        ui.small(format!("{vendor} · {category}"));
        ui.horizontal_wrapped(|ui| {
            let insert_label = if plugin.instrument {
                "Insert instrument"
            } else {
                "Insert effect"
            };
            if ui.small_button(insert_label).clicked() {
                file_ui.pending_insert = Some(plugin_example(&plugin.name, plugin.instrument));
                *close_after_insert = true;
            }
            ui.menu_button("MIDI source", |ui| {
                ui.small("For a plugin that outputs MIDI; the tag is optional.");
                if ui.button("Route to Rustel sine").clicked() {
                    file_ui.pending_insert =
                        Some(plugin_midi_example(&plugin.name, plugin.instrument, None));
                    *close_after_insert = true;
                    ui.close_menu();
                }
                for target in targets {
                    if ui.button(format!("Route to {}", target.name)).clicked() {
                        file_ui.pending_insert = Some(plugin_midi_example(
                            &plugin.name,
                            plugin.instrument,
                            Some(&target.name),
                        ));
                        *close_after_insert = true;
                        ui.close_menu();
                    }
                }
            });
            if can_reorder && sort == crate::vst_catalog::VstSort::Manual {
                ui.add_enabled_ui(index > 0, |ui| {
                    if ui.small_button("↑").clicked() {
                        *reorder = Some((index, -1));
                    }
                });
                ui.add_enabled_ui(index + 1 < total, |ui| {
                    if ui.small_button("↓").clicked() {
                        *reorder = Some((index, 1));
                    }
                });
            }
        });
        ui.collapsing("Edit label and MIDI tag", |ui| {
            let mut label = plugin.display_label.clone();
            let response = ui.add(
                TextEdit::singleline(&mut label)
                    .id(egui::Id::new(("rustel-vst-label", plugin.key.clone())))
                    .hint_text("Optional display label"),
            );
            if response.changed() {
                plugin.display_label = label;
                *directory_changed = true;
            }
            let mut midi_source = plugin.midi_source;
            if ui
                .checkbox(&mut midi_source, "MIDI output usable")
                .changed()
            {
                plugin.midi_source = midi_source;
                *directory_changed = true;
            }
            ui.small("The MIDI tag is your note; it does not test or change the plugin.");
        });
    });
}

fn plugin_matches_search(plugin: &crate::engine::VstPluginEntry, query: &str) -> bool {
    query.is_empty()
        || [
            plugin_display_label(plugin),
            &plugin.name,
            &plugin.vendor,
            &plugin.categories,
            if plugin.instrument {
                "instrument"
            } else {
                "effect processor"
            },
            if plugin.midi_source {
                "MIDI usable"
            } else {
                ""
            },
        ]
        .iter()
        .any(|field| field.to_ascii_lowercase().contains(query))
}

fn plugin_category_group(plugin: &crate::engine::VstPluginEntry) -> (String, String) {
    let type_group = if plugin.instrument {
        "Instruments"
    } else {
        "Effects / processors"
    };
    let parts = plugin
        .categories
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let subcategory = match parts.as_slice() {
        [] => "Uncategorized".to_owned(),
        [only] => (*only).to_owned(),
        [_, rest @ ..] => rest.join(" / "),
    };
    (type_group.to_owned(), subcategory)
}

fn sort_vst_plugins(
    plugins: &mut [crate::engine::VstPluginEntry],
    sort: crate::vst_catalog::VstSort,
) {
    plugins.sort_by(|left, right| {
        let name_order = || {
            plugin_display_label(left)
                .to_ascii_lowercase()
                .cmp(&plugin_display_label(right).to_ascii_lowercase())
                .then_with(|| left.key.cmp(&right.key))
        };
        match sort {
            crate::vst_catalog::VstSort::Name => name_order(),
            crate::vst_catalog::VstSort::Type => right
                .instrument
                .cmp(&left.instrument)
                .then_with(|| left.midi_source.cmp(&right.midi_source))
                .then_with(|| name_order()),
            crate::vst_catalog::VstSort::Category => left
                .categories
                .to_ascii_lowercase()
                .cmp(&right.categories.to_ascii_lowercase())
                .then_with(|| name_order()),
            crate::vst_catalog::VstSort::MidiFirst => right
                .midi_source
                .cmp(&left.midi_source)
                .then_with(|| name_order()),
            crate::vst_catalog::VstSort::Manual => {
                left.order.cmp(&right.order).then_with(|| name_order())
            }
        }
    });
}

fn plugin_display_label(plugin: &crate::engine::VstPluginEntry) -> &str {
    if plugin.display_label.trim().is_empty() {
        &plugin.name
    } else {
        plugin.display_label.trim()
    }
}

fn save_plugin_directory(
    sort: crate::vst_catalog::VstSort,
    plugins: &[crate::engine::VstPluginEntry],
) -> Result<(), String> {
    let preferences = plugins
        .iter()
        .map(|plugin| crate::vst_catalog::SavedPluginPreference {
            key: plugin.key.clone(),
            preference: crate::vst_catalog::PluginPreference {
                label: plugin.display_label.clone(),
                midi_source: plugin.midi_source,
                order: plugin.order,
            },
        })
        .collect::<Vec<_>>();
    crate::vst_catalog::VstCatalog::save_directory(sort, &preferences)
}

fn start_folder_picker(file_ui: &mut ScoreFileUi) {
    let directory = preferred_vst3_folder();
    let directories = directory
        .as_deref()
        .map(list_child_directories)
        .unwrap_or_default();
    file_ui.folder_picker = Some(FolderPicker {
        directory,
        directories,
        selected: None,
        error: None,
    });
}

#[cfg(test)]
mod vst_code_generation_tests {
    use super::{
        insert_code_at_char_range, plugin_category_group, plugin_example, plugin_matches_search,
        plugin_midi_example, sort_vst_plugins,
    };
    use crate::{engine::VstPluginEntry, vst_catalog::VstSort};

    #[test]
    fn examples_choose_the_call_from_the_scanned_plugin_type() {
        assert_eq!(
            plugin_example("Soft Keys", true),
            "note(\"c3 e3 g3\").vsti(\"Soft Keys\")"
        );
        assert_eq!(
            plugin_example("Room Verb", false),
            "note(\"c3 e3 g3\").vst(\"Room Verb\")"
        );
    }

    #[test]
    fn midi_examples_set_a_destination_and_escape_plugin_names() {
        assert_eq!(
            plugin_midi_example("Arp \"Plus\"", false, Some("Soft Keys")),
            "// MIDI target: vsti \"Soft Keys\"\nnote(\"c4\").vst(\"Arp \\\"Plus\\\"\")"
        );
        assert_eq!(
            plugin_midi_example("Arp", false, None),
            "// MIDI target: rustel \"sine\"\nnote(\"c4\").vst(\"Arp\")"
        );
    }

    #[test]
    fn insertion_replaces_the_selected_characters_and_handles_unicode() {
        let (result, cursor) = insert_code_at_char_range("α\nβ", 2..3, "PLUGIN");
        assert_eq!(result, "α\nPLUGIN");
        assert_eq!(cursor, 8);
    }

    #[test]
    fn directory_sorting_uses_custom_labels_and_optional_midi_tags() {
        let plugin = |key: &str, name: &str, label: &str, midi_source, order| VstPluginEntry {
            key: key.into(),
            name: name.into(),
            vendor: String::new(),
            categories: "Fx|Tools".into(),
            instrument: false,
            display_label: label.into(),
            midi_source,
            order,
        };
        let mut plugins = vec![
            plugin("b", "Zebra", "Alpha", true, 20),
            plugin("a", "Alpha", "", false, 10),
        ];

        sort_vst_plugins(&mut plugins, VstSort::Name);
        assert_eq!(plugins[0].display_label, "");
        assert_eq!(plugins[0].name, "Alpha");

        sort_vst_plugins(&mut plugins, VstSort::MidiFirst);
        assert_eq!(plugins[0].key, "b");

        sort_vst_plugins(&mut plugins, VstSort::Manual);
        assert_eq!(plugins[0].key, "a");
    }

    #[test]
    fn directory_search_finds_labels_vendors_and_vst3_subcategories() {
        let synth = VstPluginEntry {
            key: "synth".into(),
            name: "Super Synth".into(),
            vendor: "Audio Co".into(),
            categories: "Instrument|Synth".into(),
            instrument: true,
            display_label: "My Keys".into(),
            midi_source: true,
            order: 0,
        };
        assert!(plugin_matches_search(&synth, "my keys"));
        assert!(plugin_matches_search(&synth, "audio co"));
        assert!(plugin_matches_search(&synth, "midi"));
        assert_eq!(
            plugin_category_group(&synth),
            ("Instruments".into(), "Synth".into())
        );
    }
}

fn preferred_vst3_folder() -> Option<std::path::PathBuf> {
    standard_vst3_folders()
        .into_iter()
        .find(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok())
}

fn standard_vst3_folders() -> Vec<std::path::PathBuf> {
    [
        std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|path| path.join("Programs/Common/VST3")),
        std::env::var_os("CommonProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|path| path.join("VST3")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn list_child_directories(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut directories = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    directories
}

fn folder_picker_roots() -> Vec<std::path::PathBuf> {
    #[cfg(windows)]
    {
        ('C'..='Z')
            .map(|letter| std::path::PathBuf::from(format!("{letter}:\\")))
            .filter(|path| path.is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![std::path::PathBuf::from("/")]
    }
}

fn add_vst3_folder(
    selected: std::path::PathBuf,
    file_ui: &mut ScoreFileUi,
    params: &RustelParams,
    setter: &ParamSetter,
) {
    let mut folders =
        std::env::split_paths(std::ffi::OsStr::new(params.vst3_folders.lock().as_str()))
            .filter(|folder| !folder.as_os_str().is_empty())
            .collect::<Vec<_>>();
    if !folders.iter().any(|folder| folder == &selected) {
        folders.push(selected.clone());
    }
    match std::env::join_paths(folders) {
        Ok(joined) => {
            let joined = joined.to_string_lossy().into_owned();
            *params.vst3_folders.lock() = joined.clone();
            mark_dirty(setter, params);
            file_ui.message = match crate::portable_vst::save_user_folders(&joined) {
                Ok(()) => format!("Added and saved plugin folder: {}", selected.display()),
                Err(error) => format!("Added folder for this instance, but {error}"),
            };
            file_ui.folder_picker = None;
        }
        Err(error) => {
            if let Some(picker) = file_ui.folder_picker.as_mut() {
                picker.error = Some(format!("Could not add that folder: {error}"));
            }
        }
    }
}

fn draw_folder_picker(
    ctx: &egui::Context,
    file_ui: &mut ScoreFileUi,
    params: &RustelParams,
    setter: &ParamSetter,
) {
    let Some(picker) = file_ui.folder_picker.as_mut() else {
        return;
    };
    let mut open = true;
    let mut selected_folder = None;
    let mut close = false;
    egui::Window::new("Choose a VST3 plugin folder")
        .id(egui::Id::new("rustel-vst3-folder-picker"))
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label("Choose a folder that contains .vst3 plugins.");
            ui.horizontal(|ui| {
                if ui.button("Computer / drives").clicked() {
                    picker.directory = None;
                    picker.directories.clear();
                    picker.selected = None;
                    picker.error = None;
                }
                if let Some(parent) = picker
                    .directory
                    .as_ref()
                    .and_then(|directory| directory.parent())
                    .map(std::path::Path::to_path_buf)
                {
                    if ui.button("Up one folder").clicked() {
                        picker.directory = Some(parent);
                        picker.directories = picker
                            .directory
                            .as_deref()
                            .map(list_child_directories)
                            .unwrap_or_default();
                        picker.selected = None;
                        picker.error = None;
                    }
                }
            });
            ui.label(picker.directory.as_deref().map_or_else(
                || "Choose a drive".into(),
                |path| path.display().to_string(),
            ));
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    if picker.directory.is_some() {
                        if picker.directories.is_empty() {
                            ui.label("No subfolders found here.");
                        }
                        for child in picker.directories.clone() {
                            let name = child.file_name().unwrap_or_default().to_string_lossy();
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        picker.selected.as_ref() == Some(&child),
                                        name.to_string(),
                                    )
                                    .clicked()
                                {
                                    picker.selected = Some(child.clone());
                                }
                                if ui.small_button("Open").clicked() {
                                    picker.directory = Some(child.clone());
                                    picker.directories = list_child_directories(&child);
                                    picker.selected = None;
                                    picker.error = None;
                                }
                            });
                        }
                    } else {
                        for drive in folder_picker_roots() {
                            let label = drive.display().to_string();
                            if ui.button(label).clicked() {
                                picker.directory = Some(drive.clone());
                                picker.directories = list_child_directories(&drive);
                                picker.selected = None;
                                picker.error = None;
                            }
                        }
                    }
                });
            if let Some(error) = &picker.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        picker.directory.is_some(),
                        egui::Button::new("Use current folder"),
                    )
                    .clicked()
                {
                    selected_folder = picker.directory.clone();
                }
                if ui
                    .add_enabled(
                        picker.selected.is_some(),
                        egui::Button::new("Use selected folder"),
                    )
                    .clicked()
                {
                    selected_folder = picker.selected.clone();
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    if !open || close {
        file_ui.folder_picker = None;
    } else if let Some(selected) = selected_folder {
        if selected.is_dir() {
            add_vst3_folder(selected, file_ui, params, setter);
        } else if let Some(picker) = file_ui.folder_picker.as_mut() {
            picker.error = Some("That folder is no longer available.".into());
        }
    }
}

fn start_csound_editor(file_ui: &mut ScoreFileUi) {
    let directory = match crate::score_files::prepare_scores_directory() {
        Ok(directory) => directory,
        Err(error) => {
            file_ui.message = error;
            return;
        }
    };
    let files = match crate::score_files::list_csound_files(&directory) {
        Ok(files) => files,
        Err(error) => {
            file_ui.message = error;
            return;
        }
    };
    file_ui.csound_editor = Some(CsoundSourceUi {
        directory,
        files,
        selected: None,
        filename: "orchestra.orc".into(),
        source: String::new(),
        message: String::new(),
        error: None,
        confirm_overwrite: false,
    });
}

fn draw_csound_editor(ctx: &egui::Context, file_ui: &mut ScoreFileUi) {
    let Some(editor) = file_ui.csound_editor.as_mut() else {
        return;
    };
    let files = editor.files.clone();
    let mut open = true;
    let mut action = None;
    egui::Window::new("Csound source files")
        .id(egui::Id::new("rustel-csound-source-editor"))
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(720.0, 520.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("Portable Scores folder: {}", editor.directory.display()));
            ui.small(
                "This separate editor handles .orc and .csd text; it does not replace the Rustel score. To use an .orc, set Trusted .orc folder to this folder, then call await loadOrc(\"local:voice.orc\") in a score.",
            );
            ui.small("Save your edits before closing; this source editor is separate from the saved DAW score.");
            ui.horizontal(|ui| {
                ui.label("Filename");
                if ui.text_edit_singleline(&mut editor.filename).changed() {
                    editor.confirm_overwrite = false;
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        editor.selected.is_some(),
                        egui::Button::new("Load selected"),
                    )
                    .clicked()
                {
                    if let Some(path) = editor.selected.clone() {
                        action = Some(CsoundSourceAction::Load(path));
                    }
                }
                if ui.button(if editor.confirm_overwrite {
                    "Replace source file"
                } else {
                    "Save Csound source"
                }).clicked() {
                    match crate::score_files::csound_file_path(
                        &editor.directory,
                        &editor.filename,
                    ) {
                        Ok(path) if path.exists() && !editor.confirm_overwrite => {
                            editor.confirm_overwrite = true;
                        }
                        Ok(path) => action = Some(CsoundSourceAction::Save(path)),
                        Err(error) => editor.error = Some(error),
                    }
                }
                if ui.button("Close").clicked() {
                    action = Some(CsoundSourceAction::Close);
                }
            });
            if !files.is_empty() {
                ui.label("Csound source files in this folder");
                egui::ScrollArea::vertical()
                    .max_height(92.0)
                    .show(ui, |ui| {
                        for path in &files {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui
                                .selectable_label(editor.selected.as_ref() == Some(path), name)
                                .clicked()
                            {
                                editor.selected = Some(path.clone());
                            }
                        }
                    });
            } else {
                ui.label("No .orc or .csd source files saved here yet.");
            }
            if !editor.message.is_empty() {
                ui.colored_label(Color32::LIGHT_BLUE, &editor.message);
            }
            if let Some(error) = &editor.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            if editor.confirm_overwrite {
                ui.colored_label(Color32::YELLOW, "That file exists. Select Replace source file to overwrite it.");
            }
            ui.separator();
            ui.add(
                TextEdit::multiline(&mut editor.source)
                    .font(FontId::monospace(14.0))
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(20),
            );
        });

    if !open || matches!(action, Some(CsoundSourceAction::Close)) {
        file_ui.csound_editor = None;
        return;
    }
    match action {
        Some(CsoundSourceAction::Load(path)) => {
            match crate::score_files::read_csound_source(&path) {
                Ok(source) => {
                    editor.filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    editor.source = source;
                    editor.message = format!(
                        "Loaded {} as Csound source text.",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    editor.error = None;
                    editor.confirm_overwrite = false;
                }
                Err(error) => editor.error = Some(error),
            }
        }
        Some(CsoundSourceAction::Save(path)) => {
            match crate::score_files::write_csound_source(&path, &editor.source) {
                Ok(()) => {
                    editor.filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    match crate::score_files::list_csound_files(&editor.directory) {
                        Ok(files) => editor.files = files,
                        Err(error) => editor.error = Some(error),
                    }
                    editor.selected = Some(path.clone());
                    editor.message = format!(
                        "Saved {}.",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    editor.error = None;
                    editor.confirm_overwrite = false;
                }
                Err(error) => {
                    editor.error = Some(format!("Could not save Csound source: {error}"));
                    editor.confirm_overwrite = false;
                }
            }
        }
        Some(CsoundSourceAction::Close) | None => {}
    }
}

fn start_score_dialog(file_ui: &mut ScoreFileUi, mode: ScoreDialogMode) {
    let directory = match crate::score_files::prepare_scores_directory() {
        Ok(directory) => directory,
        Err(error) => {
            file_ui.message = error;
            return;
        }
    };
    let files = match crate::score_files::list_score_files(&directory) {
        Ok(files) => files,
        Err(error) => {
            file_ui.message = error;
            return;
        }
    };
    file_ui.message.clear();
    file_ui.dialog = Some(ScoreDialog {
        mode,
        directory,
        files,
        selected: None,
        filename: "score.strudel".into(),
        confirm_overwrite: false,
        error: None,
    });
}

fn draw_score_dialog(
    ctx: &egui::Context,
    file_ui: &mut ScoreFileUi,
    params: &RustelParams,
    setter: &ParamSetter,
) {
    let Some(dialog) = file_ui.dialog.as_mut() else {
        return;
    };

    let title = match dialog.mode {
        ScoreDialogMode::Load => "Load Rustel score",
        ScoreDialogMode::Save => "Save Rustel score",
    };
    let files = dialog.files.clone();
    let mut window_open = true;
    let mut action = None;
    egui::Window::new(title)
        .id(egui::Id::new("rustel-score-file-dialog"))
        .collapsible(false)
        .resizable(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.label(format!("Scores folder: {}", dialog.directory.display()));
            ui.separator();
            if dialog.mode == ScoreDialogMode::Save {
                ui.label("Filename (.strudel or .txt)");
                if ui.text_edit_singleline(&mut dialog.filename).changed() {
                    dialog.confirm_overwrite = false;
                }
            }
            if files.is_empty() {
                ui.label("No saved scores yet.");
            } else {
                ui.label("Saved scores");
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for path in &files {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui
                                .selectable_label(dialog.selected.as_ref() == Some(path), name)
                                .clicked()
                            {
                                dialog.selected = Some(path.clone());
                                if dialog.mode == ScoreDialogMode::Save {
                                    dialog.filename = path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned();
                                    dialog.confirm_overwrite = false;
                                }
                            }
                        }
                    });
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            if dialog.confirm_overwrite {
                ui.colored_label(Color32::YELLOW, "That file already exists. Replace it?");
            }
            ui.separator();
            ui.horizontal(|ui| {
                if dialog.mode == ScoreDialogMode::Load {
                    let selected = dialog.selected.clone();
                    if ui
                        .add_enabled(selected.is_some(), egui::Button::new("Load"))
                        .clicked()
                    {
                        if let Some(path) = selected {
                            action = Some(ScoreDialogAction::Load(path));
                        }
                    }
                } else if ui
                    .button(if dialog.confirm_overwrite {
                        "Replace"
                    } else {
                        "Save"
                    })
                    .clicked()
                {
                    match crate::score_files::score_file_path(&dialog.directory, &dialog.filename) {
                        Ok(path) if path.exists() && !dialog.confirm_overwrite => {
                            dialog.confirm_overwrite = true;
                        }
                        Ok(path) => action = Some(ScoreDialogAction::Save(path)),
                        Err(error) => dialog.error = Some(error),
                    }
                }
                if ui.button("Cancel").clicked() {
                    action = Some(ScoreDialogAction::Cancel);
                }
            });
        });

    if !window_open || matches!(action, Some(ScoreDialogAction::Cancel)) {
        file_ui.dialog = None;
        return;
    }
    match action {
        Some(ScoreDialogAction::Load(path)) => match crate::score_files::read_score_file(&path) {
            Ok(source) => {
                *params.draft.lock() = source;
                mark_dirty(setter, params);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                file_ui.message = format!("Loaded {name}. Press Evaluate to apply it.");
                file_ui.dialog = None;
            }
            Err(error) => {
                if let Some(dialog) = file_ui.dialog.as_mut() {
                    dialog.error = Some(error);
                }
            }
        },
        Some(ScoreDialogAction::Save(path)) => {
            let draft = params.draft.lock().clone();
            match crate::score_files::write_score_file(&path, &draft) {
                Ok(()) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    file_ui.message = format!("Saved {name}.");
                    file_ui.dialog = None;
                }
                Err(error) => {
                    if let Some(dialog) = file_ui.dialog.as_mut() {
                        dialog.error = Some(format!("Could not save score: {error}"));
                        dialog.confirm_overwrite = false;
                    }
                }
            }
        }
        Some(ScoreDialogAction::Cancel) | None => {}
    }
}

fn toggle(ui: &mut egui::Ui, setter: &ParamSetter, param: &BoolParam, label: &str) {
    let mut value = param.value();
    if ui.checkbox(&mut value, label).changed() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

fn mark_dirty(setter: &ParamSetter, params: &RustelParams) {
    setter.begin_set_parameter(&params.score_revision);
    setter.set_parameter(
        &params.score_revision,
        (params.score_revision.value() + 1) % 1_000_001,
    );
    setter.end_set_parameter(&params.score_revision);
}
