use crate::{
    engine::{Shared, VstPluginEntry},
    params::RustelParams,
    patch::{
        ExpressionSignal, InstrumentKind, MAX_PATCH_NODES, PatchGraph, PatchNodeKind, PortKind,
    },
};
use copypasta::{ClipboardContext, ClipboardProvider};
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
    csound_root_initialized: bool,
    vst_folders_loaded: bool,
    editor_selection: Option<std::ops::Range<usize>>,
    pending_insert: Option<String>,
    pending_score_update: Option<(String, usize)>,
    context_vst_target: Option<VstCodeTarget>,
    parameter_search: String,
    midi_sample_target: String,
    midi_wavetable_target: String,
    midi_graph_target: String,
    plugin_directory_open: bool,
    plugin_search: String,
    plugin_directory_midi_only: bool,
    patch_graph: PatchGraph,
    patch_loaded: bool,
    patch_screen: bool,
    patch_wire_from: Option<u64>,
    patch_pan: egui::Vec2,
    patch_message: String,
    patch_add_position: Option<[f32; 2]>,
    clear_patch_confirm: bool,
    add_instrument_kind: InstrumentKind,
}

#[derive(Clone, Debug)]
struct VstCodeTarget {
    name: String,
    name_bytes: std::ops::Range<usize>,
    call_close_byte: usize,
    has_second_argument: bool,
    parameter_object_bytes: Option<std::ops::Range<usize>>,
}

struct FolderPicker {
    purpose: FolderPickerPurpose,
    directory: Option<std::path::PathBuf>,
    directories: Vec<std::path::PathBuf>,
    selected: Option<std::path::PathBuf>,
    orc_files: Vec<std::path::PathBuf>,
    selected_orc: Option<std::path::PathBuf>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum FolderPickerPurpose {
    Vst3,
    CsoundOrc { target_node_id: u64 },
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
            if !file_ui.csound_root_initialized {
                file_ui.csound_root_initialized = true;
                if let Ok(scores) = crate::score_files::prepare_scores_directory() {
                    let current_root = params.csound_orchestra_root.lock().clone();
                    if should_use_portable_orchestra_root(&current_root, &scores)
                        && set_trusted_orchestra_root_value(&scores, &params)
                    {
                        mark_dirty(setter, &params);
                    }
                }
            }
            if !file_ui.patch_loaded {
                file_ui.patch_loaded = true;
                let saved_patch = params.patch_graph.lock().clone();
                if !saved_patch.trim().is_empty() {
                    match PatchGraph::from_json(&saved_patch) {
                        Ok(mut graph) => {
                            let has_orc = graph.nodes.iter().any(|node| {
                                matches!(
                                    &node.kind,
                                    PatchNodeKind::Instrument {
                                        instrument: InstrumentKind::Csound,
                                        orc_file: Some(_),
                                        ..
                                    }
                                )
                            });
                            let mut patch_changed = false;
                            if has_orc
                                && let Ok(scores) = crate::score_files::prepare_scores_directory()
                            {
                                patch_changed = rebase_portable_orc_references(&mut graph, &scores);
                                if graph_has_relative_orc(&graph) {
                                    patch_changed |=
                                        set_trusted_orchestra_root_value(&scores, &params);
                                }
                            }
                            file_ui.patch_graph = graph;
                            if patch_changed {
                                match file_ui.patch_graph.to_json() {
                                    Ok(saved) => {
                                        *params.patch_graph.lock() = saved;
                                        mark_dirty(setter, &params);
                                    }
                                    Err(error) => file_ui.patch_message = error,
                                }
                            }
                        }
                        Err(error) => file_ui.patch_message = error,
                    }
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
                    let dropped_live_midi = shared
                        .live_midi
                        .as_ref()
                        .map_or(0, |bridge| bridge.dropped_count());
                    if dropped_live_midi > 0 {
                        ui.colored_label(
                            Color32::YELLOW,
                            format!("{dropped_live_midi} live MIDI event(s) were dropped"),
                        );
                    }
                });
                ui.small("Score-based MIDI phrases repeat; free-running MIDI sources can stream into Rustel synths or hosted VSTi instruments.");
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
                        (
                            "FM Electric Piano",
                            "await loadOrc(\"local:fm-electric-piano.orc\");\nnote(\"c4 eb4 g4 c5\").csoundm(\"fm_electric_piano\").gain(0.55)",
                        ),
                        (
                            "FM Bass",
                            "await loadOrc(\"local:fm-bass.orc\");\nnote(\"c2 ~ c2 g1 ~ g1 bb1 c2\").csoundm(\"fm_bass\").gain(0.65).slow(2)",
                        ),
                        (
                            "FM Bell",
                            "await loadOrc(\"local:fm-bell.orc\");\nnote(\"c5 ~ e5 ~ g5 ~ b5 ~\").csoundm(\"fm_bell\").gain(0.50)",
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
                    if ui.selectable_label(!file_ui.patch_screen, "Score").clicked() {
                        file_ui.patch_screen = false;
                    }
                    if ui.selectable_label(file_ui.patch_screen, "Patch").clicked() {
                        file_ui.patch_screen = true;
                    }
                });
                ui.separator();
                if file_ui.patch_screen {
                    if draw_patch_editor(ui, file_ui, &shared, &params) {
                        match file_ui.patch_graph.to_json() {
                            Ok(saved) => {
                                *params.patch_graph.lock() = saved;
                                mark_dirty(setter, &params);
                            }
                            Err(error) => file_ui.patch_message = error,
                        }
                    }
                } else {
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
                if let Some((updated, cursor)) = file_ui.pending_score_update.take() {
                    draft = updated;
                    inserted_cursor = Some(cursor);
                    changed = true;
                } else if let Some(snippet) = file_ui.pending_insert.take() {
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
                        output.state.clone().store(ui.ctx(), output.response.id);
                        file_ui.editor_selection = Some(cursor..cursor);
                    } else if let Some(cursor_range) = output.cursor_range {
                        file_ui.editor_selection =
                            Some(cursor_range.as_sorted_char_range());
                    }
                    if output.response.changed() {
                        changed = true;
                    }
                    if output.response.secondary_clicked() {
                        let previous_target = file_ui
                            .context_vst_target
                            .as_ref()
                            .map(|target| target.name.clone());
                        file_ui.context_vst_target = None;
                        let clicked_char = ui.ctx().input(|input| {
                            input.pointer.interact_pos().map(|position| {
                                output
                                    .galley
                                    .cursor_from_pos(position - output.galley_pos)
                                    .ccursor
                                    .index
                            })
                        });
                        if let Some(clicked_char) = clicked_char {
                            let existing = file_ui.editor_selection.clone();
                            file_ui.editor_selection = Some(match existing {
                                Some(range)
                                    if range.start <= clicked_char && clicked_char <= range.end =>
                                {
                                    range
                                }
                                _ => clicked_char..clicked_char,
                            });
                            file_ui.context_vst_target = find_vst_call_at_char(&draft, clicked_char);
                            if let Some(target) = &file_ui.context_vst_target {
                                if previous_target.as_deref() != Some(target.name.as_str()) {
                                    file_ui.parameter_search.clear();
                                }
                                let name_range = draft[..target.name_bytes.start].chars().count()
                                    ..draft[..target.name_bytes.end].chars().count();
                                output.state.cursor.set_char_range(Some(
                                    egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(name_range.start),
                                        egui::text::CCursor::new(name_range.end),
                                    ),
                                ));
                                output.state.store(ui.ctx(), output.response.id);
                                file_ui.editor_selection = Some(name_range);
                                shared.request_vst_parameters(&target.name);
                            }
                        }
                    }
                    let context = file_ui.context_vst_target.clone();
                    output.response.context_menu(|menu| {
                        let selection = file_ui.editor_selection.clone().unwrap_or_else(|| {
                            let end = draft.chars().count();
                            end..end
                        });
                        let selected = text_at_char_range(&draft, selection.clone());
                        if menu.button("Copy").clicked() {
                            if !selected.is_empty() {
                                menu.ctx().copy_text(selected.clone());
                            }
                            menu.close_menu();
                        }
                        if menu.button("Cut").clicked() {
                            if !selected.is_empty() {
                                menu.ctx().copy_text(selected);
                                let (updated, cursor) =
                                    replace_char_range(&draft, selection.clone(), "");
                                file_ui.pending_score_update = Some((updated, cursor));
                            }
                            menu.close_menu();
                        }
                        if menu.button("Paste").clicked() {
                            match clipboard_text() {
                                Ok(pasted) => {
                                    let (updated, cursor) =
                                        replace_char_range(&draft, selection.clone(), &pasted);
                                    file_ui.pending_score_update = Some((updated, cursor));
                                }
                                Err(error) => file_ui.message = error,
                            }
                            menu.close_menu();
                        }

                        if let Some(target) = context {
                            menu.separator();
                            menu.label(format!("Effect parameters · {}", target.name));
                            let state = shared.vst_parameter_state.lock().clone();
                            if state.plugin != target.name || state.loading {
                                shared.request_vst_parameters(&target.name);
                                menu.colored_label(
                                    Color32::LIGHT_BLUE,
                                    "Loading the effect and its parameters…",
                                );
                            } else if let Some(error) = state.error {
                                menu.colored_label(Color32::LIGHT_RED, error);
                            } else if state.parameters.is_empty() {
                                menu.label("This effect has no exposed parameters.");
                            } else {
                                if target.has_second_argument
                                    && target.parameter_object_bytes.is_none()
                                {
                                    menu.small(
                                        "Use a literal parameter object to insert controls here.",
                                    );
                                }
                                menu.add(
                                    TextEdit::singleline(&mut file_ui.parameter_search)
                                        .hint_text("Filter parameters"),
                                );
                                let query = file_ui.parameter_search.trim().to_ascii_lowercase();
                                egui::ScrollArea::vertical()
                                    .max_height(240.0)
                                    .show(menu, |menu| {
                                        for parameter in &state.parameters {
                                            if !query.is_empty()
                                                && !parameter
                                                    .name
                                                    .to_ascii_lowercase()
                                                    .contains(&query)
                                                && !parameter.key.to_ascii_lowercase().contains(&query)
                                                && !parameter
                                                    .group
                                                    .to_ascii_lowercase()
                                                    .contains(&query)
                                            {
                                                continue;
                                            }
                                            let group = if parameter.group.is_empty() {
                                                String::new()
                                            } else {
                                                format!("{} · ", parameter.group)
                                            };
                                            let label = format!(
                                                "{group}{} ({}) · {}",
                                                parameter.name,
                                                parameter.key,
                                                parameter.default_shown
                                            );
                                            if menu.button(label).clicked() {
                                                if let Some((updated, cursor)) =
                                                    insert_vst_parameter(
                                                        &draft,
                                                        &target,
                                                        parameter,
                                                    )
                                                {
                                                    file_ui.pending_score_update =
                                                        Some((updated, cursor));
                                                } else {
                                                    file_ui.message = "The parameter picker can edit a direct `{ key: value }` object, not a variable or expression.".into();
                                                }
                                                menu.close_menu();
                                            }
                                        }
                                    });
                            }
                        } else {
                            menu.separator();
                            menu.small("Right-click an effect name inside .vst(\"Name\") to browse its parameters.");
                        }
                    });
                });
                if changed {
                    *params.draft.lock() = draft;
                    mark_dirty(setter, &params);
                }
                }
            });
            draw_plugin_directory(ctx, file_ui, &shared);
            draw_score_dialog(ctx, file_ui, &params, setter);
            draw_folder_picker(ctx, file_ui, &params, setter);
            draw_csound_editor(ctx, file_ui, &params, setter);
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

const PATCH_NODE_WIDTH: f32 = 158.0;
const PATCH_NODE_HEIGHT: f32 = 112.0;
const PATCH_NODE_CONTENT_HEIGHT: f32 = 60.0;
const PATCH_NODE_COLUMN_STEP: f32 = 178.0;
const PATCH_NODE_ROW_STEP: f32 = 158.0;
const PATCH_NODE_COLLISION_HEIGHT: f32 = PATCH_NODE_HEIGHT + 36.0;

#[derive(Clone, Copy)]
enum PluginPickerFilter {
    Instrument,
    Effect,
    MidiSource,
}

impl PluginPickerFilter {
    fn matches(self, plugin: &VstPluginEntry) -> bool {
        match self {
            Self::Instrument => plugin.instrument,
            // The scan classification is a useful hint, but score `.vst()` can
            // host any scanned plugin that accepts audio input.
            Self::Effect => true,
            Self::MidiSource => plugin.midi_source,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Instrument => "instrument",
            Self::Effect => "VST3 plugin",
            Self::MidiSource => "MIDI source",
        }
    }

    fn empty_message(self) -> &'static str {
        match self {
            Self::Instrument => "No scanned VST3 instruments are available.",
            Self::Effect => "No scanned VST3 plugins are available.",
            Self::MidiSource => {
                "No plugins are in the MIDI index. Open Plugin list, choose All plugins, then click Add to MIDI index on a plugin that outputs MIDI."
            }
        }
    }
}

fn next_open_patch_position(graph: &PatchGraph) -> [f32; 2] {
    for slot in 0..=MAX_PATCH_NODES {
        let column = slot % 3;
        let row = slot / 3;
        let candidate = [
            14.0 + column as f32 * PATCH_NODE_COLUMN_STEP,
            24.0 + row as f32 * PATCH_NODE_ROW_STEP,
        ];
        let overlaps = graph.nodes.iter().any(|node| {
            candidate[0] < node.position[0] + PATCH_NODE_WIDTH + 12.0
                && candidate[0] + PATCH_NODE_WIDTH + 12.0 > node.position[0]
                && candidate[1] < node.position[1] + PATCH_NODE_COLLISION_HEIGHT
                && candidate[1] + PATCH_NODE_COLLISION_HEIGHT > node.position[1]
        });
        if !overlaps {
            return candidate;
        }
    }
    [14.0, 24.0]
}

fn plugin_picker_matches(plugin: &VstPluginEntry, filter: PluginPickerFilter) -> bool {
    filter.matches(plugin)
}

#[derive(Default)]
struct PatchNodeActions {
    changed: bool,
    input_clicked: bool,
    output_clicked: bool,
    remove_clicked: bool,
    input_port: Option<egui::Pos2>,
    output_port: Option<egui::Pos2>,
}

fn draw_patch_editor(
    ui: &mut egui::Ui,
    file_ui: &mut ScoreFileUi,
    shared: &Shared,
    params: &RustelParams,
) -> bool {
    let mut changed = false;
    let mut new_node = None;
    let mut new_node_position = None;
    let plugins = shared.vst_plugins.lock().clone();
    let instrument_count = plugins.iter().filter(|plugin| plugin.instrument).count();
    let effect_count = plugins.iter().filter(|plugin| !plugin.instrument).count();
    let midi_source_count = plugins.iter().filter(|plugin| plugin.midi_source).count();
    ui.horizontal_wrapped(|ui| {
        if ui.button("＋ Notes").clicked() {
            new_node = Some(PatchNodeKind::NotePattern {
                expression: "note(\"c3 e3 g3\")".into(),
            });
        }
        if ui.button("＋ MIDI VST").clicked() {
            new_node = Some(PatchNodeKind::MidiVst {
                plugin: first_plugin_name(shared, |plugin| plugin.midi_source).unwrap_or_default(),
                is_effect: true,
            });
        }
        egui::ComboBox::from_id_salt("rustel-patch-add-instrument-kind")
            .selected_text(format!("Add: {}", file_ui.add_instrument_kind.label()))
            .show_ui(ui, |ui| {
                for kind in [
                    InstrumentKind::RustelSynth,
                    InstrumentKind::Sample,
                    InstrumentKind::Wavetable,
                    InstrumentKind::Graph,
                    InstrumentKind::Csound,
                    InstrumentKind::Vsti,
                ] {
                    ui.selectable_value(&mut file_ui.add_instrument_kind, kind, kind.label());
                }
            });
        if ui.button("＋ Instrument").clicked() {
            let kind = file_ui.add_instrument_kind;
            new_node = Some(PatchNodeKind::Instrument {
                instrument: kind,
                value: default_instrument_value(kind, shared),
                orc_file: None,
            });
        }
        if ui.button("＋ Pattern op").clicked() {
            new_node = Some(PatchNodeKind::Expression {
                signal: ExpressionSignal::Midi,
                suffix: ".slow(2)".into(),
            });
        }
        if ui.button("＋ Audio op").clicked() {
            new_node = Some(PatchNodeKind::Expression {
                signal: ExpressionSignal::Audio,
                suffix: ".gain(0.5)".into(),
            });
        }
        if ui.button("＋ VST effect").clicked() {
            new_node = Some(PatchNodeKind::VstEffect {
                plugin: first_plugin_name(shared, |plugin| !plugin.instrument)
                    .or_else(|| first_plugin_name(shared, |_| true))
                    .unwrap_or_default(),
                parameters: String::new(),
            });
        }
        let has_output = file_ui
            .patch_graph
            .nodes
            .iter()
            .any(|node| matches!(node.kind, PatchNodeKind::Output));
        if ui
            .add_enabled(!has_output, egui::Button::new("＋ Output"))
            .clicked()
        {
            new_node = Some(PatchNodeKind::Output);
        }
        if ui.button("Clear all blocks…").clicked() {
            file_ui.clear_patch_confirm = true;
        }
        ui.separator();
        if ui
            .button(RichText::new("Generate Code").strong())
            .on_hover_text(
                "Validate this patch and replace the score editor with generated Rustel code",
            )
            .clicked()
        {
            let orchestra_root = params.csound_orchestra_root.lock().clone();
            match file_ui
                .patch_graph
                .generate_code_with_orchestra_root(&orchestra_root)
            {
                Ok(code) => {
                    let cursor = code.chars().count();
                    file_ui.pending_score_update = Some((code, cursor));
                    file_ui.patch_screen = false;
                    file_ui.patch_message.clear();
                    file_ui.message =
                        "Generated score code. Review it in Score, then press Evaluate.".into();
                }
                Err(error) => file_ui.patch_message = error,
            }
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.small(format!(
            "Scanned VST3s: {} · Instruments: {instrument_count} · Effects: {effect_count} · MIDI-tagged: {midi_source_count}",
            plugins.len()
        ));
        if ui.button("Plugin list…").clicked() {
            file_ui.plugin_directory_open = true;
        }
        if ui.button("Rescan VST3").clicked() {
            shared.scan_vst_plugins.store(true, Ordering::Release);
            *shared.vst_plugins_message.lock() = "VST3 rescan requested…".into();
        }
    });
    let scan_message = shared.vst_plugins_message.lock().clone();
    if !scan_message.is_empty() {
        ui.small(scan_message);
    }
    if midi_source_count == 0 {
        ui.small(
            "Add MIDI-outputting plugins from Plugin list → All plugins → Add to MIDI index. This tag does not test the plugin.",
        );
    }

    let mut clear_requested = false;
    if file_ui.clear_patch_confirm {
        let mut confirm_open = true;
        let mut cancel_clear = false;
        egui::Window::new("Clear patch canvas?")
            .id(egui::Id::new("rustel-patch-clear-confirm"))
            .collapsible(false)
            .resizable(false)
            .open(&mut confirm_open)
            .show(ui.ctx(), |ui| {
                ui.label("This removes every block and cable from the Patch canvas.");
                ui.small("Your Score text is not changed.");
                ui.horizontal(|ui| {
                    if ui.button("Clear all blocks").clicked() {
                        clear_requested = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_clear = true;
                    }
                });
            });
        file_ui.clear_patch_confirm = confirm_open && !clear_requested && !cancel_clear;
    }
    if clear_requested {
        file_ui.patch_graph.clear();
        file_ui.patch_wire_from = None;
        file_ui.patch_add_position = None;
        file_ui.patch_message =
            "Canvas cleared. Right-click the background or use the buttons above to add blocks."
                .into();
        changed = true;
    }

    ui.horizontal_wrapped(|ui| {
        ui.small("Right-click to add blocks. Click an output, then a matching input to connect. Drag the ⋮⋮ handle to move a block; hold Space and drag the background to pan.");
        ui.label(RichText::new("MIDI").color(port_color(PortKind::Midi)));
        ui.label(RichText::new("Audio").color(port_color(PortKind::Audio)));
        if file_ui.patch_wire_from.is_some() && ui.small_button("Cancel cable").clicked() {
            file_ui.patch_wire_from = None;
        }
    });
    if !file_ui.patch_message.is_empty() {
        ui.colored_label(Color32::LIGHT_RED, &file_ui.patch_message);
    }

    let wires = file_ui.patch_graph.connections.clone();
    let mut remove_wire = None;
    ui.collapsing("Connections · click × to remove a cable", |ui| {
        ui.horizontal_wrapped(|ui| {
            for wire in &wires {
                let source = file_ui
                    .patch_graph
                    .nodes
                    .iter()
                    .find(|node| node.id == wire.source)
                    .map(|node| node.kind.title())
                    .unwrap_or("Missing node");
                let target = file_ui
                    .patch_graph
                    .nodes
                    .iter()
                    .find(|node| node.id == wire.target)
                    .map(|node| node.kind.title())
                    .unwrap_or("Missing node");
                if ui
                    .small_button(format!("{} {source} → {target} ×", wire.signal.label()))
                    .clicked()
                {
                    remove_wire = Some((wire.source, wire.target));
                }
            }
            if wires.is_empty() {
                ui.small("No cables yet.");
            }
        });
    });
    if let Some((source, target)) = remove_wire {
        file_ui.patch_graph.disconnect(source, target);
        changed = true;
    }
    ui.separator();

    let available = ui.available_size();
    let canvas_size = egui::vec2(available.x.max(240.0), available.y.max(280.0));
    let (canvas_rect, canvas_response) =
        ui.allocate_exact_size(canvas_size, egui::Sense::click_and_drag());
    if canvas_response.dragged() && ui.input(|input| input.key_down(egui::Key::Space)) {
        file_ui.patch_pan += canvas_response.drag_delta();
    }
    if canvas_response.secondary_clicked() {
        if let Some(pointer) = canvas_response.interact_pointer_pos() {
            file_ui.patch_add_position = Some([
                pointer.x - canvas_rect.left() - file_ui.patch_pan.x,
                pointer.y - canvas_rect.top() - file_ui.patch_pan.y,
            ]);
        }
    }
    let has_output = file_ui
        .patch_graph
        .nodes
        .iter()
        .any(|node| matches!(node.kind, PatchNodeKind::Output));
    canvas_response.context_menu(|ui| {
        ui.label("Add a block at this position");
        if ui.button("MIDI pattern").clicked() {
            new_node = Some(PatchNodeKind::NotePattern {
                expression: "note(\"c3 e3 g3\")".into(),
            });
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
        if ui.button("MIDI VST3 source").clicked() {
            new_node = Some(PatchNodeKind::MidiVst {
                plugin: first_plugin_name(shared, |plugin| plugin.midi_source).unwrap_or_default(),
                is_effect: true,
            });
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
        ui.menu_button("Instrument", |ui| {
            for kind in [
                InstrumentKind::RustelSynth,
                InstrumentKind::Sample,
                InstrumentKind::Wavetable,
                InstrumentKind::Graph,
                InstrumentKind::Csound,
                InstrumentKind::Vsti,
            ] {
                if ui.button(kind.label()).clicked() {
                    file_ui.add_instrument_kind = kind;
                    new_node = Some(PatchNodeKind::Instrument {
                        instrument: kind,
                        value: default_instrument_value(kind, shared),
                        orc_file: None,
                    });
                    new_node_position = file_ui.patch_add_position.take();
                    ui.close_menu();
                }
            }
        });
        if ui.button("Pattern modifier").clicked() {
            new_node = Some(PatchNodeKind::Expression {
                signal: ExpressionSignal::Midi,
                suffix: ".slow(2)".into(),
            });
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
        if ui.button("Audio modifier").clicked() {
            new_node = Some(PatchNodeKind::Expression {
                signal: ExpressionSignal::Audio,
                suffix: ".gain(0.5)".into(),
            });
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
        if ui.button("VST3 effect").clicked() {
            new_node = Some(PatchNodeKind::VstEffect {
                plugin: first_plugin_name(shared, |plugin| !plugin.instrument).unwrap_or_default(),
                parameters: String::new(),
            });
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
        if ui
            .add_enabled(!has_output, egui::Button::new("Audio output"))
            .clicked()
        {
            new_node = Some(PatchNodeKind::Output);
            new_node_position = file_ui.patch_add_position.take();
            ui.close_menu();
        }
    });
    if let Some(kind) = new_node.take() {
        let position = new_node_position
            .take()
            .unwrap_or_else(|| next_open_patch_position(&file_ui.patch_graph));
        match file_ui.patch_graph.add_node(kind, position) {
            Ok(_) => {
                file_ui.patch_add_position = None;
                file_ui.patch_message.clear();
                changed = true;
            }
            Err(error) => file_ui.patch_message = error,
        }
    }
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 4.0, Color32::from_rgb(24, 27, 34));
    if file_ui.patch_graph.nodes.is_empty() {
        painter.text(
            canvas_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Empty Patch canvas\nRight-click here or use the buttons above to add blocks.",
            egui::FontId::proportional(16.0),
            Color32::from_gray(174),
        );
    }

    let mut removed_node = None;
    let mut new_connection = None;
    let mut port_positions = std::collections::HashMap::new();
    for index in 0..file_ui.patch_graph.nodes.len() {
        let node = &mut file_ui.patch_graph.nodes[index];
        let id = node.id;
        let midi_input_connected = file_ui
            .patch_graph
            .connections
            .iter()
            .any(|wire| wire.target == id);
        let screen_position =
            canvas_rect.min + file_ui.patch_pan + egui::vec2(node.position[0], node.position[1]);
        let mut actions = PatchNodeActions::default();
        egui::Area::new(egui::Id::new(("rustel-patch-node", id)))
            .order(egui::Order::Middle)
            .fixed_pos(screen_position)
            .show(ui.ctx(), |ui| {
                egui::Frame::group(ui.style())
                    .fill(Color32::from_rgb(39, 43, 53))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(83, 91, 108)))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(
                            PATCH_NODE_WIDTH - 12.0,
                            PATCH_NODE_HEIGHT - 12.0,
                        ));
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
                        let mut node_drag_delta = egui::Vec2::ZERO;
                        ui.horizontal(|ui| {
                            let drag_width = (PATCH_NODE_WIDTH - 12.0 - 26.0).max(40.0);
                            let drag = ui
                                .add_sized(
                                    [drag_width, 20.0],
                                    egui::Label::new(
                                        RichText::new(format!("⋮⋮ {}", node.kind.title()))
                                            .strong()
                                            .color(Color32::from_rgb(202, 187, 246)),
                                    )
                                    .sense(egui::Sense::drag()),
                                )
                                .on_hover_text("Drag anywhere across this header to move the block");
                            if drag.dragged() {
                                node_drag_delta = drag.drag_delta();
                            }
                            if ui.small_button("×").on_hover_text("Remove node").clicked() {
                                actions.remove_clicked = true;
                            }
                        });
                        if node_drag_delta != egui::Vec2::ZERO {
                            node.position[0] =
                                (node.position[0] + node_drag_delta.x).clamp(-4096.0, 4096.0);
                            node.position[1] =
                                (node.position[1] + node_drag_delta.y).clamp(-4096.0, 4096.0);
                            actions.changed = true;
                        }
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .id_salt(("rustel-patch-node-fields", id))
                            .max_height(PATCH_NODE_CONTENT_HEIGHT)
                            .show(ui, |ui| {
                                actions.changed |= draw_patch_node_fields(
                                    ui,
                                    &mut node.kind,
                                    id,
                                    shared,
                                    &mut file_ui.folder_picker,
                                    params,
                                    &mut file_ui.patch_message,
                                    midi_input_connected,
                                );
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if let Some(input) = node.kind.input_kind() {
                                let connected = file_ui
                                    .patch_graph
                                    .connections
                                    .iter()
                                    .any(|wire| wire.target == id);
                                let optional_midi = matches!(node.kind, PatchNodeKind::MidiVst { .. });
                                let label = if connected {
                                    format!("← {} In ✓", input.label())
                                } else if optional_midi {
                                    "← MIDI In?".to_owned()
                                } else {
                                    format!("← {} In", input.label())
                                };
                                let response =
                                    ui.small_button(RichText::new(label).color(port_color(input)))
                                        .on_hover_text(if optional_midi {
                                            "Optional. Leave unconnected for a free-running MIDI generator, or connect a Note Pattern to drive a MIDI processor."
                                        } else {
                                            "Click an output first, then click this input."
                                        });
                                actions.input_port = Some(response.rect.center());
                                if response.clicked() {
                                    actions.input_clicked = true;
                                }
                            }
                            if let Some(output) = node.kind.output_kind() {
                                let selected = file_ui.patch_wire_from == Some(id);
                                let label = if selected {
                                    format!("{} Out …", output.label())
                                } else {
                                    format!("{} Out →", output.label())
                                };
                                let response = ui
                                    .with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.small_button(
                                                RichText::new(label).color(port_color(output)),
                                            )
                                        },
                                    )
                                    .inner;
                                actions.output_port = Some(response.rect.center());
                                if response.clicked() {
                                    actions.output_clicked = true;
                                }
                            }
                        });
                    });
            });

        let node_title = node.kind.title();
        let node_output_kind = node.kind.output_kind();
        port_positions.insert(id, (actions.input_port, actions.output_port));
        if actions.changed {
            changed = true;
            let dropped = file_ui.patch_graph.reconcile_connections();
            if dropped > 0 {
                file_ui.patch_message = format!(
                    "Removed {dropped} cable(s) that no longer match the changed port type."
                );
            }
        }
        if actions.remove_clicked {
            removed_node = Some(id);
        }
        if actions.output_clicked {
            file_ui.patch_wire_from = Some(id);
            file_ui.patch_message = format!(
                "Choose a compatible input for the {} output.",
                node_output_kind.map_or("node", PortKind::label)
            );
        }
        if actions.input_clicked {
            if let Some(source) = file_ui.patch_wire_from.take() {
                new_connection = Some((source, id));
            } else {
                file_ui.patch_message = format!(
                    "Click an output port first, then choose the {} input.",
                    node_title
                );
            }
        }
    }
    if let Some(id) = removed_node {
        file_ui.patch_graph.remove_node(id);
        if file_ui.patch_wire_from == Some(id) {
            file_ui.patch_wire_from = None;
        }
        changed = true;
    }
    if let Some((source, target)) = new_connection {
        match file_ui.patch_graph.connect(source, target) {
            Ok(()) => {
                changed = true;
                file_ui.patch_message.clear();
            }
            Err(error) => file_ui.patch_message = error,
        }
    }
    for wire in &file_ui.patch_graph.connections {
        if let (Some((_, Some(from))), Some((Some(to), _))) = (
            port_positions.get(&wire.source),
            port_positions.get(&wire.target),
        ) {
            painter.arrow(
                *from,
                *to - *from,
                egui::Stroke::new(2.5_f32, port_color(wire.signal)),
            );
        }
    }
    if let (Some(source_id), Some(to)) = (
        file_ui.patch_wire_from,
        ui.ctx().input(|input| input.pointer.hover_pos()),
    ) && let Some((_, Some(from))) = port_positions.get(&source_id)
    {
        let signal = file_ui
            .patch_graph
            .nodes
            .iter()
            .find(|node| node.id == source_id)
            .and_then(|node| node.kind.output_kind())
            .unwrap_or(PortKind::Midi);
        painter.line_segment([*from, to], egui::Stroke::new(1.5_f32, port_color(signal)));
    }
    changed
}

fn draw_patch_node_fields(
    ui: &mut egui::Ui,
    kind: &mut PatchNodeKind,
    id: u64,
    shared: &Shared,
    folder_picker: &mut Option<FolderPicker>,
    params: &RustelParams,
    patch_message: &mut String,
    midi_input_connected: bool,
) -> bool {
    let mut changed = false;
    match kind {
        PatchNodeKind::NotePattern { expression } => {
            ui.small("Rustel / Strudel pattern source");
            changed |= ui
                .add(
                    TextEdit::singleline(expression)
                        .hint_text("note(\"c3 e3 g3\")")
                        .desired_width(PATCH_NODE_WIDTH - 32.0),
                )
                .changed();
        }
        PatchNodeKind::MidiVst { plugin, is_effect } => {
            ui.horizontal(|ui| {
                ui.small("Plugin type");
                changed |= ui.checkbox(is_effect, "MIDI effect").changed();
            });
            if midi_input_connected {
                ui.small("The connected Note Pattern drives this MIDI processor.");
            } else {
                ui.small("Leave the MIDI input unconnected to run a free-running generator.");
                ui.small("Connect the MIDI source to a Rustel synth or scanned VSTi instrument.");
                ui.small("The VST editor opens by itself; streaming starts automatically.");
                if ui.button("Open MIDI VST editor").clicked() {
                    let entry = shared
                        .vst_plugins
                        .lock()
                        .iter()
                        .find(|entry| entry.name == *plugin && entry.midi_source)
                        .cloned();
                    match entry {
                        Some(entry) => match shared.open_live_midi_editor(&entry) {
                            Ok(()) => {
                                *patch_message =
                                    "The MIDI VST editor opened and is streaming to the connected instrument. Close it to stop.".into();
                            }
                            Err(error) => *patch_message = error,
                        },
                        None => {
                            *patch_message =
                                "Mark this scanned plugin as a MIDI source, then rescan.".into();
                        }
                    }
                }
            }
            changed |= draw_plugin_picker(ui, plugin, id, shared, PluginPickerFilter::MidiSource);
        }
        PatchNodeKind::Instrument {
            instrument,
            value,
            orc_file,
        } => {
            let previous = *instrument;
            egui::ComboBox::from_id_salt(("rustel-patch-instrument", id))
                .selected_text(instrument.label())
                .show_ui(ui, |ui| {
                    for choice in [
                        InstrumentKind::RustelSynth,
                        InstrumentKind::Sample,
                        InstrumentKind::Wavetable,
                        InstrumentKind::Graph,
                        InstrumentKind::Csound,
                        InstrumentKind::Vsti,
                    ] {
                        ui.selectable_value(instrument, choice, choice.label());
                    }
                });
            if previous != *instrument {
                *value = default_instrument_value(*instrument, shared);
                *orc_file = None;
                changed = true;
            }
            match instrument {
                InstrumentKind::Vsti => {
                    ui.small("Choose a scanned VST3 instrument or enter its name.");
                    changed |=
                        draw_plugin_picker(ui, value, id, shared, PluginPickerFilter::Instrument);
                }
                InstrumentKind::Graph => {
                    ui.small("KabelSalat expression");
                    changed |= ui
                        .add(
                            TextEdit::singleline(value)
                                .hint_text("sine(sFreq).mul(0.2).out()")
                                .desired_width(PATCH_NODE_WIDTH - 32.0),
                        )
                        .changed();
                }
                InstrumentKind::Csound => {
                    ui.small("Pick an ORC from disk, then enter its instrument name.");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Load ORC from disk…").clicked() {
                            start_orc_picker(folder_picker, id, params);
                        }
                        if let Some(path) = orc_file.as_deref() {
                            let filename = std::path::Path::new(path)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy();
                            ui.small(format!("ORC: {filename}"));
                            if ui.small_button("Clear ORC").clicked() {
                                *orc_file = None;
                                changed = true;
                            }
                        } else {
                            ui.small("No ORC file selected");
                        }
                    });
                    if let Some(path) = orc_file.as_deref() {
                        let root = params.csound_orchestra_root.lock().clone();
                        if !resolved_orc_path(path, &root).is_file() {
                            ui.colored_label(Color32::YELLOW, "The selected ORC file is missing.");
                        }
                    }
                    ui.small(
                        "Csound must be enabled in Transport. Only load orchestra files you trust.",
                    );
                    changed |= ui
                        .add(
                            TextEdit::singleline(value)
                                .hint_text("Csound instrument name")
                                .desired_width(PATCH_NODE_WIDTH - 32.0),
                        )
                        .changed();
                }
                _ => {
                    ui.small("Sound or sample name");
                    changed |= ui
                        .add(
                            TextEdit::singleline(value)
                                .hint_text("sine, bd, or wavetable name")
                                .desired_width(PATCH_NODE_WIDTH - 32.0),
                        )
                        .changed();
                }
            }
        }
        PatchNodeKind::Expression { signal, suffix } => {
            let previous = *signal;
            egui::ComboBox::from_id_salt(("rustel-patch-expression-signal", id))
                .selected_text(match signal {
                    ExpressionSignal::Midi => "MIDI / pattern",
                    ExpressionSignal::Audio => "Audio",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(signal, ExpressionSignal::Midi, "MIDI / pattern");
                    ui.selectable_value(signal, ExpressionSignal::Audio, "Audio");
                });
            if previous != *signal {
                changed = true;
            }
            ui.small("Rustel method suffix, for example .slow(2) or .gain(0.5)");
            changed |= ui
                .add(
                    TextEdit::singleline(suffix)
                        .hint_text(".slow(2)")
                        .desired_width(PATCH_NODE_WIDTH - 32.0),
                )
                .changed();
        }
        PatchNodeKind::VstEffect { plugin, parameters } => {
            ui.small("Scanned effect name");
            ui.small("The list includes all scanned plugins; choose one that accepts audio input.");
            changed |= draw_plugin_picker(ui, plugin, id, shared, PluginPickerFilter::Effect);
            ui.small("Optional Rustel parameter object");
            changed |= ui
                .add(
                    TextEdit::singleline(parameters)
                        .hint_text("{ mix: 0.5 }")
                        .desired_width(PATCH_NODE_WIDTH - 32.0),
                )
                .changed();
        }
        PatchNodeKind::Output => {
            ui.small("Mixes incoming audio branches to the plugin output.");
        }
    }
    changed
}

fn draw_plugin_picker(
    ui: &mut egui::Ui,
    selected: &mut String,
    id: u64,
    shared: &Shared,
    filter: PluginPickerFilter,
) -> bool {
    let plugins = shared.vst_plugins.lock().clone();
    let available_count = plugins
        .iter()
        .filter(|plugin| plugin_picker_matches(plugin, filter))
        .count();
    let selected_label = plugins
        .iter()
        .find(|plugin| plugin.name == *selected && plugin_picker_matches(plugin, filter))
        .map(|plugin| plugin_picker_label(plugin, filter))
        .unwrap_or(if selected.is_empty() {
            if available_count == 0 {
                format!("No scanned {} ({available_count})", filter.label())
            } else {
                format!("Choose {} ({available_count})", filter.label())
            }
        } else {
            selected.clone()
        });
    let mut changed = false;
    egui::ComboBox::from_id_salt(("rustel-patch-plugin", id))
        .selected_text(selected_label)
        .width(PATCH_NODE_WIDTH - 34.0)
        .show_ui(ui, |ui| {
            if available_count == 0 {
                ui.small(filter.empty_message());
                ui.small("Open Plugin list above to review scan results.");
            }
            for plugin in &plugins {
                if !plugin_picker_matches(plugin, filter) {
                    continue;
                }
                if ui
                    .selectable_label(
                        *selected == plugin.name,
                        plugin_picker_label(plugin, filter),
                    )
                    .clicked()
                {
                    *selected = plugin.name.clone();
                    changed = true;
                }
            }
        });
    changed |= ui
        .add(
            TextEdit::singleline(selected)
                .hint_text("Plugin name")
                .desired_width(PATCH_NODE_WIDTH - 32.0),
        )
        .changed();
    if available_count == 0 {
        ui.small(filter.empty_message());
    }
    changed
}

fn first_plugin_name(
    shared: &Shared,
    predicate: impl Fn(&VstPluginEntry) -> bool,
) -> Option<String> {
    shared
        .vst_plugins
        .lock()
        .iter()
        .find(|plugin| predicate(plugin))
        .map(|plugin| plugin.name.clone())
}

fn plugin_picker_label(plugin: &VstPluginEntry, filter: PluginPickerFilter) -> String {
    match filter {
        PluginPickerFilter::Effect => {
            let class = if plugin.instrument {
                "Instrument"
            } else {
                "Effect"
            };
            let midi = if plugin.midi_source { " · MIDI" } else { "" };
            format!("{class}{midi} · {}", plugin_display_label(plugin))
        }
        PluginPickerFilter::Instrument | PluginPickerFilter::MidiSource => {
            let midi = if plugin.midi_source { "MIDI · " } else { "" };
            format!("{midi}{}", plugin_display_label(plugin))
        }
    }
}

fn default_instrument_value(kind: InstrumentKind, shared: &Shared) -> String {
    match kind {
        InstrumentKind::RustelSynth => "sine".into(),
        InstrumentKind::Sample => "bd".into(),
        InstrumentKind::Wavetable => "wt".into(),
        InstrumentKind::Graph => "sine(sFreq).mul(0.2).out()".into(),
        InstrumentKind::Csound => "triangle".into(),
        InstrumentKind::Vsti => {
            first_plugin_name(shared, |plugin| plugin.instrument).unwrap_or_default()
        }
    }
}

fn port_color(kind: PortKind) -> Color32 {
    let [red, green, blue] = kind.color();
    Color32::from_rgb(red, green, blue)
}

fn replace_char_range(
    source: &str,
    selection: std::ops::Range<usize>,
    replacement: &str,
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
    let mut result =
        String::with_capacity(source.len() - (end_byte - start_byte) + replacement.len());
    result.push_str(&source[..start_byte]);
    result.push_str(replacement);
    result.push_str(&source[end_byte..]);
    (result, start + replacement.chars().count())
}

fn text_at_char_range(source: &str, selection: std::ops::Range<usize>) -> String {
    let start = selection.start.min(source.chars().count());
    let end = selection.end.min(source.chars().count()).max(start);
    source.chars().skip(start).take(end - start).collect()
}

fn find_vst_call_at_char(source: &str, cursor_char: usize) -> Option<VstCodeTarget> {
    let cursor_byte = source
        .char_indices()
        .nth(cursor_char)
        .map_or(source.len(), |(byte, _)| byte);
    for (start, _) in source.match_indices(".vst(") {
        if start > 0
            && source[..start]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_alphanumeric() || character == '_')
        {
            continue;
        }
        let open_paren = start + 4;
        let bytes = source.as_bytes();
        let mut quote_at = open_paren + 1;
        while bytes.get(quote_at).is_some_and(u8::is_ascii_whitespace) {
            quote_at += 1;
        }
        let Some(&quote) = bytes.get(quote_at) else {
            continue;
        };
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let name_start = quote_at + 1;
        let mut name_end = name_start;
        let mut escaped = false;
        while let Some(&byte) = bytes.get(name_end) {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                break;
            }
            name_end += 1;
        }
        if bytes.get(name_end) != Some(&quote) || cursor_byte < name_start || cursor_byte > name_end
        {
            continue;
        }
        let name = decode_js_string(&source[name_start..name_end]);
        let Some(call_close_byte) = matching_delimiter(source, open_paren, b'(', b')') else {
            continue;
        };
        let mut after_name = name_end + 1;
        while bytes.get(after_name).is_some_and(u8::is_ascii_whitespace) {
            after_name += 1;
        }
        let has_second_argument = bytes.get(after_name) == Some(&b',');
        let parameter_object_bytes = if has_second_argument {
            let mut argument = after_name + 1;
            while bytes.get(argument).is_some_and(u8::is_ascii_whitespace) {
                argument += 1;
            }
            (bytes.get(argument) == Some(&b'{'))
                .then(|| matching_delimiter(source, argument, b'{', b'}').map(|end| argument..end))
                .flatten()
        } else {
            None
        };
        return Some(VstCodeTarget {
            name,
            name_bytes: name_start..name_end,
            call_close_byte,
            has_second_argument,
            parameter_object_bytes,
        });
    }
    None
}

fn matching_delimiter(source: &str, open_at: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open_at) != Some(&open) {
        return None;
    }
    let mut depth = 0_usize;
    let mut quote = None;
    let mut escaped = false;
    let mut index = open_at;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' || byte == b'`' {
            quote = Some(byte);
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        } else if byte == open {
            depth += 1;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn decode_js_string(source: &str) -> String {
    let mut decoded = String::with_capacity(source.len());
    let mut characters = source.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some('t') => decoded.push('\t'),
            Some(escaped) => decoded.push(escaped),
            None => decoded.push('\\'),
        }
    }
    decoded
}

fn insert_vst_parameter(
    source: &str,
    target: &VstCodeTarget,
    parameter: &crate::engine::VstParameterChoice,
) -> Option<(String, usize)> {
    if target.has_second_argument && target.parameter_object_bytes.is_none() {
        return None;
    }
    let value = if parameter.default_value.is_finite() {
        let shown = format!("{:.4}", parameter.default_value.clamp(0.0, 1.0));
        shown.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        "0.5".to_owned()
    };
    let property = format!("{}: {value}", quoted_code_string(&parameter.key));
    let (position, insertion) = if let Some(object) = &target.parameter_object_bytes {
        let content = &source[object.start + 1..object.end];
        let position = object.start + 1 + content.trim_end().len();
        let insertion = if content.trim().is_empty() {
            format!(" {property}")
        } else {
            format!(", {property}")
        };
        (position, insertion)
    } else {
        (target.call_close_byte, format!(", {{ {property} }}"))
    };
    if position > source.len() || !source.is_char_boundary(position) {
        return None;
    }
    let mut updated = String::with_capacity(source.len() + insertion.len());
    updated.push_str(&source[..position]);
    updated.push_str(&insertion);
    updated.push_str(&source[position..]);
    let cursor = updated[..position + insertion.len()].chars().count();
    Some((updated, cursor))
}

fn clipboard_text() -> Result<String, String> {
    let mut clipboard = ClipboardContext::new()
        .map_err(|error| format!("Could not open the system clipboard: {error}"))?;
    clipboard
        .get_contents()
        .map_err(|error| format!("Could not read the system clipboard: {error}"))
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
    let target = match target {
        Some(target) => format!("vsti {}", quoted_code_string(target)),
        None => "rustel \"sine\"".into(),
    };
    plugin_midi_target_example(name, instrument, &target)
}

fn plugin_midi_target_example(name: &str, instrument: bool, target: &str) -> String {
    let method = if instrument { "vsti" } else { "vst" };
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
            if file_ui.midi_sample_target.is_empty() {
                file_ui.midi_sample_target = "bd".into();
            }
            if file_ui.midi_wavetable_target.is_empty() {
                file_ui.midi_wavetable_target = "wt_x".into();
            }
            if file_ui.midi_graph_target.is_empty() {
                file_ui.midi_graph_target = "sine(sFreq).mul(0.2).out()".into();
            }
            let mut plugins = shared.vst_plugins.lock().clone();
            let plugin_message = shared.vst_plugins_message.lock().clone();
            let saved_sort = *shared.vst_sort.lock();
            let mut sort = saved_sort;
            let midi_source_count = plugins.iter().filter(|plugin| plugin.midi_source).count();

            ui.horizontal(|ui| {
                ui.strong(format!("{} scanned", plugins.len()));
                ui.selectable_value(
                    &mut file_ui.plugin_directory_midi_only,
                    false,
                    format!("All plugins ({})", plugins.len()),
                );
                ui.selectable_value(
                    &mut file_ui.plugin_directory_midi_only,
                    true,
                    format!("MIDI index ({midi_source_count})"),
                );
            });
            ui.horizontal(|ui| {
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
                    let in_selected_view =
                        !file_ui.plugin_directory_midi_only || plugin.midi_source;
                    (in_selected_view && plugin_matches_search(plugin, &query)).then_some(index)
                })
                .collect::<Vec<_>>();
            ui.small(format!(
                "Showing {} of {}. MIDI sources also stay in All plugins with their instrument/effect type.",
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
                    } else if visible_indices.is_empty()
                        && file_ui.plugin_directory_midi_only
                        && midi_source_count == 0
                    {
                        ui.label("The MIDI index is empty.");
                        ui.small("Choose All plugins and use Add to MIDI index on plugins that output MIDI.");
                    } else if visible_indices.is_empty() {
                        ui.label("No plugins match that search.");
                    } else if matches!(
                        sort,
                        crate::vst_catalog::VstSort::Manual
                            | crate::vst_catalog::VstSort::MidiFirst
                    ) || file_ui.plugin_directory_midi_only
                    {
                        let can_reorder = sort == crate::vst_catalog::VstSort::Manual
                            && query.is_empty()
                            && !file_ui.plugin_directory_midi_only;
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
                        } else if sort == crate::vst_catalog::VstSort::Manual
                            && file_ui.plugin_directory_midi_only
                        {
                            ui.small("Choose All plugins to change the full manual order.");
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
            let midi_index_label = if plugin.midi_source {
                "Remove from MIDI index"
            } else {
                "Add to MIDI index"
            };
            if ui.small_button(midi_index_label).clicked() {
                plugin.midi_source = !plugin.midi_source;
                *directory_changed = true;
            }
            ui.menu_button("MIDI source", |ui| {
                ui.small("For a plugin that outputs MIDI; the tag is optional.");
                if ui.button("Route to Rustel sine").clicked() {
                    file_ui.pending_insert =
                        Some(plugin_midi_example(&plugin.name, plugin.instrument, None));
                    *close_after_insert = true;
                    ui.close_menu();
                }
                ui.menu_button("Rustel sample", |ui| {
                    ui.label("Sample name available to Rustel:");
                    ui.text_edit_singleline(&mut file_ui.midi_sample_target);
                    if ui.button("Insert sample route").clicked()
                        && !file_ui.midi_sample_target.trim().is_empty()
                    {
                        file_ui.pending_insert = Some(plugin_midi_target_example(
                            &plugin.name,
                            plugin.instrument,
                            &format!(
                                "sample {}",
                                quoted_code_string(file_ui.midi_sample_target.trim())
                            ),
                        ));
                        *close_after_insert = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Rustel wavetable", |ui| {
                    ui.label("Wavetable name available to Rustel:");
                    ui.text_edit_singleline(&mut file_ui.midi_wavetable_target);
                    if ui.button("Insert wavetable route").clicked()
                        && !file_ui.midi_wavetable_target.trim().is_empty()
                    {
                        file_ui.pending_insert = Some(plugin_midi_target_example(
                            &plugin.name,
                            plugin.instrument,
                            &format!(
                                "wavetable {}",
                                quoted_code_string(file_ui.midi_wavetable_target.trim())
                            ),
                        ));
                        *close_after_insert = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("KabelSalat graph", |ui| {
                    ui.label("Graph expression; use sFreq/sGate and send sound to .out():");
                    ui.text_edit_singleline(&mut file_ui.midi_graph_target);
                    if ui.button("Insert graph route").clicked()
                        && !file_ui.midi_graph_target.trim().is_empty()
                    {
                        file_ui.pending_insert = Some(plugin_midi_target_example(
                            &plugin.name,
                            plugin.instrument,
                            &format!("graph {}", file_ui.midi_graph_target.trim()),
                        ));
                        *close_after_insert = true;
                        ui.close_menu();
                    }
                });
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
        ui.collapsing("Edit display label", |ui| {
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
        purpose: FolderPickerPurpose::Vst3,
        directory,
        directories,
        selected: None,
        orc_files: Vec::new(),
        selected_orc: None,
        error: None,
    });
}

fn start_orc_picker(folder_picker: &mut Option<FolderPicker>, node_id: u64, params: &RustelParams) {
    let trusted_root = std::path::PathBuf::from(params.csound_orchestra_root.lock().as_str());
    let portable_scores = crate::score_files::prepare_scores_directory().ok();
    let directory = orc_picker_start_directory(&trusted_root, portable_scores);
    let directories = directory
        .as_deref()
        .map(list_child_directories)
        .unwrap_or_default();
    let orc_files = directory.as_deref().map(list_orc_files).unwrap_or_default();
    *folder_picker = Some(FolderPicker {
        purpose: FolderPickerPurpose::CsoundOrc {
            target_node_id: node_id,
        },
        directory,
        directories,
        selected: None,
        orc_files,
        selected_orc: None,
        error: None,
    });
}

fn orc_picker_start_directory(
    trusted_root: &std::path::Path,
    portable_scores: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    portable_scores
        .filter(|directory| directory.is_dir())
        .or_else(|| trusted_root.is_dir().then(|| trusted_root.to_path_buf()))
}

fn should_use_portable_orchestra_root(saved_root: &str, portable_scores: &std::path::Path) -> bool {
    let saved_root = saved_root.trim();
    if saved_root.is_empty() {
        return true;
    }
    let saved_path = std::path::Path::new(saved_root);
    if !saved_path.is_dir() || is_filesystem_root(saved_path) {
        return true;
    }
    let is_scores_folder = saved_path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Scores"));
    is_scores_folder && saved_path.canonicalize().ok() != portable_scores.canonicalize().ok()
}

fn is_filesystem_root(path: &std::path::Path) -> bool {
    use std::path::Component;

    let mut has_root = false;
    for component in path.components() {
        match component {
            Component::Prefix(_) => {}
            Component::RootDir => has_root = true,
            _ => return false,
        }
    }
    has_root
}

fn portable_orc_reference(path: &str, scores_root: &std::path::Path) -> Option<String> {
    let path = std::path::Path::new(path);
    if let Some(relative) = normalized_relative_orc_path(path) {
        return Some(relative);
    }

    let canonical_root = scores_root
        .canonicalize()
        .unwrap_or_else(|_| scores_root.into());
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Ok(relative) = canonical_path.strip_prefix(&canonical_root) {
        return normalized_relative_orc_path(relative);
    }

    let relative = relative_after_scores_folder(path)?;
    canonical_root
        .join(&relative)
        .is_file()
        .then(|| normalized_relative_orc_path(&relative))
        .flatten()
}

fn normalized_relative_orc_path(path: &std::path::Path) -> Option<String> {
    use std::path::Component;

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn relative_after_scores_folder(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::Component;

    let components = path.components().collect::<Vec<_>>();
    let scores_index = components.iter().rposition(|component| {
        matches!(
            component,
            Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case("Scores")
        )
    })?;
    let mut relative = std::path::PathBuf::new();
    for component in components.iter().skip(scores_index + 1) {
        match component {
            Component::Normal(part) => relative.push(part),
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn rebase_portable_orc_references(graph: &mut PatchGraph, scores_root: &std::path::Path) -> bool {
    let mut changed = false;
    for node in &mut graph.nodes {
        if let PatchNodeKind::Instrument {
            instrument: InstrumentKind::Csound,
            orc_file: Some(path),
            ..
        } = &mut node.kind
            && let Some(relative) = portable_orc_reference(path, scores_root)
            && *path != relative
        {
            *path = relative;
            changed = true;
        }
    }
    changed
}

fn graph_has_relative_orc(graph: &PatchGraph) -> bool {
    graph.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            PatchNodeKind::Instrument {
                instrument: InstrumentKind::Csound,
                orc_file: Some(path),
                ..
            } if !std::path::Path::new(path).is_absolute()
        )
    })
}

fn resolved_orc_path(path: &str, trusted_root: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::Path::new(trusted_root).join(path)
    }
}

fn set_trusted_orchestra_root_value(folder: &std::path::Path, params: &RustelParams) -> bool {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let folder = folder.to_string_lossy().into_owned();
    let mut current = params.csound_orchestra_root.lock();
    if *current == folder {
        return false;
    }
    *current = folder;
    true
}

#[cfg(test)]
mod vst_code_generation_tests {
    use super::{
        PluginPickerFilter, find_vst_call_at_char, insert_code_at_char_range, insert_vst_parameter,
        list_orc_files, next_open_patch_position, orc_picker_start_directory,
        plugin_category_group, plugin_example, plugin_matches_search, plugin_midi_example,
        plugin_midi_target_example, plugin_picker_label, plugin_picker_matches,
        portable_orc_reference, rebase_portable_orc_references, replace_char_range,
        resolved_orc_path, should_use_portable_orchestra_root, sort_vst_plugins,
        text_at_char_range, trusted_orchestra_folder,
    };
    use crate::{
        engine::{VstParameterChoice, VstPluginEntry},
        patch::PatchGraph,
        vst_catalog::VstSort,
    };
    use std::fs;

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
    fn midi_target_choices_generate_sample_wavetable_and_graph_directives() {
        assert_eq!(
            plugin_midi_target_example("Arp", true, "sample \"bd\""),
            "// MIDI target: sample \"bd\"\nnote(\"c4\").vsti(\"Arp\")"
        );
        assert_eq!(
            plugin_midi_target_example("Arp", true, "wavetable \"wt_x\""),
            "// MIDI target: wavetable \"wt_x\"\nnote(\"c4\").vsti(\"Arp\")"
        );
        assert_eq!(
            plugin_midi_target_example("Arp", true, "graph sine(sFreq).mul(0.2).out()"),
            "// MIDI target: graph sine(sFreq).mul(0.2).out()\nnote(\"c4\").vsti(\"Arp\")"
        );
    }

    #[test]
    fn insertion_replaces_the_selected_characters_and_handles_unicode() {
        let (result, cursor) = insert_code_at_char_range("α\nβ", 2..3, "PLUGIN");
        assert_eq!(result, "α\nPLUGIN");
        assert_eq!(cursor, 8);
    }

    #[test]
    fn clipboard_selection_replacement_preserves_unicode() {
        let source = "α beta";
        assert_eq!(text_at_char_range(source, 2..6), "beta");
        assert_eq!(
            replace_char_range(source, 2..6, "sound"),
            ("α sound".into(), 7)
        );
    }

    #[test]
    fn right_click_target_finds_vst_effect_name_and_ignores_vsti() {
        let source = r#"note("c3").vsti("Keys").vst("Room Verb")"#;
        let cursor = source
            .chars()
            .position(|character| character == 'V')
            .unwrap();
        let target = find_vst_call_at_char(source, cursor).expect("effect name");
        assert_eq!(target.name, "Room Verb");

        let instrument = r#"note("c3").vsti("Keys")"#;
        let cursor = instrument
            .chars()
            .position(|character| character == 'K')
            .unwrap();
        assert!(find_vst_call_at_char(instrument, cursor).is_none());
    }

    #[test]
    fn parameter_choice_adds_a_normalized_value_to_effect_call() {
        let source = r#"note("c3").vst("Room Verb")"#;
        let cursor = source
            .chars()
            .position(|character| character == 'R')
            .unwrap();
        let target = find_vst_call_at_char(source, cursor).expect("effect name");
        let parameter = VstParameterChoice {
            key: "mix".into(),
            name: "Mix".into(),
            group: String::new(),
            default_value: 0.375,
            default_shown: "0.375".into(),
        };
        let (updated, _) =
            insert_vst_parameter(source, &target, &parameter).expect("updated effect");
        assert_eq!(updated, r#"note("c3").vst("Room Verb", { "mix": 0.375 })"#);
    }

    #[test]
    fn parameter_choice_appends_to_an_existing_effect_parameter_object() {
        let source = r#"note("c3").vst("Room Verb", { decay: 0.7 })"#;
        let cursor = source
            .chars()
            .position(|character| character == 'R')
            .unwrap();
        let target = find_vst_call_at_char(source, cursor).expect("effect name");
        let parameter = VstParameterChoice {
            key: "mix".into(),
            name: "Mix".into(),
            group: String::new(),
            default_value: 0.5,
            default_shown: "0.5".into(),
        };
        let (updated, _) =
            insert_vst_parameter(source, &target, &parameter).expect("updated effect");
        assert_eq!(
            updated,
            r#"note("c3").vst("Room Verb", { decay: 0.7, "mix": 0.5 })"#
        );
    }

    #[test]
    fn directory_sorting_uses_custom_labels_and_optional_midi_tags() {
        let plugin = |key: &str, name: &str, label: &str, midi_source, order| VstPluginEntry {
            key: key.into(),
            name: name.into(),
            bundle: std::path::PathBuf::new(),
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
            bundle: std::path::PathBuf::new(),
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

    #[test]
    fn patch_plugin_pickers_include_scanned_effect_candidates() {
        let instrument = VstPluginEntry {
            key: "keys".into(),
            name: "Keys".into(),
            bundle: std::path::PathBuf::new(),
            vendor: String::new(),
            categories: "Instrument|Synth".into(),
            instrument: true,
            display_label: String::new(),
            midi_source: false,
            order: 0,
        };
        let midi_effect = VstPluginEntry {
            key: "arp".into(),
            name: "Arpeggiator".into(),
            bundle: std::path::PathBuf::new(),
            vendor: String::new(),
            categories: "Fx|MIDI".into(),
            instrument: false,
            display_label: String::new(),
            midi_source: true,
            order: 1,
        };
        let audio_effect = VstPluginEntry {
            key: "verb".into(),
            name: "Reverb".into(),
            bundle: std::path::PathBuf::new(),
            vendor: String::new(),
            categories: "Fx|Reverb".into(),
            instrument: false,
            display_label: String::new(),
            midi_source: false,
            order: 2,
        };

        assert!(plugin_picker_matches(
            &instrument,
            PluginPickerFilter::Instrument
        ));
        assert!(!plugin_picker_matches(
            &midi_effect,
            PluginPickerFilter::Instrument
        ));
        assert!(plugin_picker_matches(
            &midi_effect,
            PluginPickerFilter::MidiSource
        ));
        assert!(!plugin_picker_matches(
            &audio_effect,
            PluginPickerFilter::MidiSource
        ));
        assert!(plugin_picker_matches(
            &audio_effect,
            PluginPickerFilter::Effect
        ));
        assert!(plugin_picker_matches(
            &instrument,
            PluginPickerFilter::Effect
        ));
        assert_eq!(
            plugin_picker_label(&instrument, PluginPickerFilter::Effect),
            "Instrument · Keys"
        );
    }

    #[test]
    fn new_patch_nodes_use_an_open_canvas_position_after_removals() {
        let mut graph = PatchGraph::default();
        let middle_node = graph.nodes[1].id;
        assert!(graph.remove_node(middle_node));

        assert_eq!(next_open_patch_position(&graph), [192.0, 24.0]);
    }

    #[test]
    fn orc_picker_lists_only_orchestra_files() {
        let directory =
            std::env::temp_dir().join(format!("rustel-orc-picker-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create ORC picker folder");
        let first = directory.join("voice.orc");
        let second = directory.join("zeta.ORC");
        fs::write(&first, "instr voice\nendin").expect("write ORC file");
        fs::write(&second, "instr zeta\nendin").expect("write ORC file");
        fs::write(directory.join("score.strudel"), "note(\"c3\")").expect("write score");

        assert_eq!(list_orc_files(&directory), vec![first, second]);

        fs::remove_dir_all(directory).expect("remove ORC picker folder");
    }

    #[test]
    fn trusted_orchestra_folder_follows_loaded_or_saved_orc_file() {
        assert_eq!(
            trusted_orchestra_folder(std::path::Path::new("Scores/fm.orc")),
            Some(std::path::Path::new("Scores"))
        );
        assert_eq!(
            trusted_orchestra_folder(std::path::Path::new("Scores/fm.ORC")),
            Some(std::path::Path::new("Scores"))
        );
        assert_eq!(
            trusted_orchestra_folder(std::path::Path::new("Scores/project.csd")),
            None
        );
    }

    #[test]
    fn orc_picker_prefers_the_portable_scores_folder() {
        let base =
            std::env::temp_dir().join(format!("rustel-orc-picker-start-{}", std::process::id()));
        let portable_scores = base.join("Portable").join("Scores");
        let stale_root = base.join("C-drive-root");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&portable_scores).expect("create portable Scores folder");
        fs::create_dir_all(&stale_root).expect("create stale trusted folder");

        assert_eq!(
            orc_picker_start_directory(&stale_root, Some(portable_scores.clone())),
            Some(portable_scores)
        );

        fs::remove_dir_all(base).expect("remove temporary ORC picker folders");
    }

    #[test]
    fn stale_scores_and_drive_roots_are_replaced_with_portable_scores() {
        let base =
            std::env::temp_dir().join(format!("rustel-orc-root-migration-{}", std::process::id()));
        let portable_scores = base.join("Current").join("Scores");
        let old_scores = base.join("Old").join("Scores");
        let custom_folder = base.join("ExternalOrchestra");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&portable_scores).expect("create current Scores folder");
        fs::create_dir_all(&old_scores).expect("create old Scores folder");
        fs::create_dir_all(&custom_folder).expect("create custom orchestra folder");

        assert!(should_use_portable_orchestra_root("", &portable_scores));
        assert!(should_use_portable_orchestra_root("/", &portable_scores));
        assert!(should_use_portable_orchestra_root(
            &old_scores.to_string_lossy(),
            &portable_scores
        ));
        assert!(!should_use_portable_orchestra_root(
            &portable_scores.to_string_lossy(),
            &portable_scores
        ));
        assert!(!should_use_portable_orchestra_root(
            &custom_folder.to_string_lossy(),
            &portable_scores
        ));

        fs::remove_dir_all(base).expect("remove temporary orchestra folders");
    }

    #[test]
    fn saved_portable_orc_path_rebases_after_the_bundle_moves() {
        let base = std::env::temp_dir().join(format!(
            "rustel-portable-orc-relocation-{}",
            std::process::id()
        ));
        let old_file = base
            .join("OldLocation")
            .join("RustelPortable")
            .join("Scores")
            .join("voices")
            .join("fm.orc");
        let new_scores = base
            .join("NewLocation")
            .join("RustelPortable")
            .join("Scores");
        fs::create_dir_all(old_file.parent().expect("old ORC parent"))
            .expect("create old package path");
        fs::create_dir_all(new_scores.join("voices")).expect("create moved package path");
        fs::write(
            new_scores.join("voices").join("fm.orc"),
            "instr voice\nendin",
        )
        .expect("write portable orchestra");

        assert_eq!(
            portable_orc_reference(&old_file.to_string_lossy(), &new_scores),
            Some("voices/fm.orc".into())
        );

        let mut graph = PatchGraph::default();
        let instrument = graph
            .nodes
            .iter_mut()
            .find(|node| matches!(node.kind, crate::patch::PatchNodeKind::Instrument { .. }))
            .expect("default instrument");
        instrument.kind = crate::patch::PatchNodeKind::Instrument {
            instrument: crate::patch::InstrumentKind::Csound,
            value: "voice".into(),
            orc_file: Some(old_file.to_string_lossy().into_owned()),
        };
        assert!(rebase_portable_orc_references(&mut graph, &new_scores));
        let rebased = graph
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                crate::patch::PatchNodeKind::Instrument { orc_file, .. } => orc_file.as_deref(),
                _ => None,
            })
            .expect("rebased ORC path");
        assert_eq!(rebased, "voices/fm.orc");
        assert_eq!(
            resolved_orc_path(rebased, &new_scores.to_string_lossy()),
            new_scores.join("voices").join("fm.orc")
        );

        fs::remove_dir_all(base).expect("remove temporary portable packages");
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

fn list_orc_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let path = entry.path();
            (file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("orc")))
            .then_some(path)
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    files
}

#[cfg(windows)]
fn path_is_within(root: &std::path::Path, path: &std::path::Path) -> bool {
    let root = root
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    let path = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    path.starts_with(&root)
}

#[cfg(not(windows))]
fn path_is_within(root: &std::path::Path, path: &std::path::Path) -> bool {
    path.starts_with(root)
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

fn attach_orc_file(
    selected: std::path::PathBuf,
    node_id: u64,
    file_ui: &mut ScoreFileUi,
    params: &RustelParams,
    setter: &ParamSetter,
) -> Result<(), String> {
    let selected = std::fs::canonicalize(&selected)
        .map_err(|error| format!("Could not open the selected ORC file: {error}"))?;
    if !selected
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("orc"))
    {
        return Err("Choose a .orc orchestra file.".into());
    }
    let metadata = std::fs::metadata(&selected)
        .map_err(|error| format!("Could not inspect the selected ORC file: {error}"))?;
    if metadata.len() > 1024 * 1024 {
        return Err("ORC files must be 1 MiB or smaller.".into());
    }
    std::fs::read_to_string(&selected)
        .map_err(|error| format!("The ORC file must be readable UTF-8 text: {error}"))?;
    let parent = selected
        .parent()
        .ok_or_else(|| "The selected ORC file has no parent folder.".to_owned())?;
    let portable_scores = crate::score_files::prepare_scores_directory()
        .ok()
        .and_then(|directory| directory.canonicalize().ok());
    let portable_reference = portable_scores
        .as_deref()
        .and_then(|root| portable_orc_reference(&selected.to_string_lossy(), root));
    let trusted_root = if portable_reference.is_some() {
        portable_scores
            .clone()
            .expect("portable reference has a portable Scores directory")
    } else {
        parent.to_path_buf()
    };
    let stored_path = portable_reference.unwrap_or_else(|| selected.to_string_lossy().into_owned());
    let mut graph = file_ui.patch_graph.clone();
    let current_root = std::path::PathBuf::from(params.csound_orchestra_root.lock().as_str())
        .canonicalize()
        .ok();
    let existing_orcs = graph
        .nodes
        .iter()
        .filter(|node| node.id != node_id)
        .filter_map(|node| match &node.kind {
            PatchNodeKind::Instrument {
                instrument: InstrumentKind::Csound,
                orc_file: Some(path),
                ..
            } => {
                let path = std::path::PathBuf::from(path);
                let portable_path = portable_scores
                    .as_deref()
                    .and_then(|root| portable_orc_reference(&path.to_string_lossy(), root))
                    .zip(portable_scores.as_deref())
                    .map(|(relative, root)| root.join(relative));
                Some(if let Some(portable_path) = portable_path {
                    portable_path
                } else if path.is_absolute() {
                    path
                } else if let Some(root) = current_root.as_deref() {
                    root.join(path)
                } else {
                    path
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let update_root = current_root.as_deref() != Some(trusted_root.as_path());
    if update_root
        && existing_orcs
            .iter()
            .any(|path| !path_is_within(&trusted_root, path))
    {
        return Err(
            "Keep selected ORC files in one trusted folder. Clear other ORC selections before choosing a different folder."
                .into(),
        );
    }
    let node = graph
        .nodes
        .iter_mut()
        .find(|node| node.id == node_id)
        .ok_or_else(|| "The Csound block no longer exists.".to_owned())?;
    match &mut node.kind {
        PatchNodeKind::Instrument {
            instrument: InstrumentKind::Csound,
            orc_file,
            ..
        } => *orc_file = Some(stored_path),
        _ => return Err("The selected block is no longer a Csound instrument.".into()),
    }
    let saved = graph.to_json()?;
    if update_root {
        *params.csound_orchestra_root.lock() = trusted_root.to_string_lossy().into_owned();
    }
    *params.patch_graph.lock() = saved;
    file_ui.patch_graph = graph;
    mark_dirty(setter, params);
    file_ui.patch_message = format!(
        "Attached {} to the Csound block. Generate Code adds its loadOrc call.",
        selected.file_name().unwrap_or_default().to_string_lossy()
    );
    Ok(())
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
    let purpose = picker.purpose;
    let is_orc_picker = matches!(purpose, FolderPickerPurpose::CsoundOrc { .. });
    let mut open = true;
    let mut selected_folder = None;
    let mut selected_orc = None;
    let mut close = false;
    let title = if is_orc_picker {
        "Load a Csound ORC file"
    } else {
        "Choose a VST3 plugin folder"
    };
    let window_id = if is_orc_picker {
        "rustel-csound-orc-picker"
    } else {
        "rustel-vst3-folder-picker"
    };
    egui::Window::new(title)
        .id(egui::Id::new(window_id))
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .show(ctx, |ui| {
            if is_orc_picker {
                ui.label("Choose a local .orc file for this Csound instrument block.");
                ui.small("Starts in the portable Scores folder. Portable ORCs are saved as relative paths.");
            } else {
                ui.label("Choose a folder that contains .vst3 plugins.");
            }
            ui.horizontal(|ui| {
                if ui.button("Computer / drives").clicked() {
                    picker.directory = None;
                    picker.directories.clear();
                    picker.orc_files.clear();
                    picker.selected = None;
                    picker.selected_orc = None;
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
                        picker.orc_files = picker
                            .directory
                            .as_deref()
                            .map(list_orc_files)
                            .unwrap_or_default();
                        picker.selected = None;
                        picker.selected_orc = None;
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
                                    picker.orc_files = list_orc_files(&child);
                                    picker.selected = None;
                                    picker.selected_orc = None;
                                    picker.error = None;
                                }
                            });
                        }
                        if is_orc_picker {
                            ui.separator();
                            ui.label("ORC files in this folder");
                            if picker.orc_files.is_empty() {
                                ui.small("No .orc files found here.");
                            }
                            for path in picker.orc_files.clone() {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                if ui
                                    .selectable_label(
                                        picker.selected_orc.as_ref() == Some(&path),
                                        name.to_string(),
                                    )
                                    .clicked()
                                {
                                    picker.selected_orc = Some(path);
                                }
                            }
                        }
                    } else {
                        for drive in folder_picker_roots() {
                            let label = drive.display().to_string();
                            if ui.button(label).clicked() {
                                picker.directory = Some(drive.clone());
                                picker.directories = list_child_directories(&drive);
                                picker.orc_files = list_orc_files(&drive);
                                picker.selected = None;
                                picker.selected_orc = None;
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
                if is_orc_picker {
                    if ui
                        .add_enabled(
                            picker.selected_orc.is_some(),
                            egui::Button::new("Load selected ORC"),
                        )
                        .clicked()
                    {
                        selected_orc = picker.selected_orc.clone();
                    }
                } else {
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
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    let target_node_id = match purpose {
        FolderPickerPurpose::Vst3 => None,
        FolderPickerPurpose::CsoundOrc { target_node_id } => Some(target_node_id),
    };
    let _ = picker;
    if !open || close {
        file_ui.folder_picker = None;
    } else if let Some(selected) = selected_folder {
        if selected.is_dir() {
            add_vst3_folder(selected, file_ui, params, setter);
        } else if let Some(picker) = file_ui.folder_picker.as_mut() {
            picker.error = Some("That folder is no longer available.".into());
        }
    } else if let (Some(selected), Some(node_id)) = (selected_orc, target_node_id) {
        match attach_orc_file(selected, node_id, file_ui, params, setter) {
            Ok(()) => file_ui.folder_picker = None,
            Err(error) => {
                if let Some(picker) = file_ui.folder_picker.as_mut() {
                    picker.error = Some(error);
                }
            }
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

fn draw_csound_editor(
    ctx: &egui::Context,
    file_ui: &mut ScoreFileUi,
    params: &RustelParams,
    setter: &ParamSetter,
) {
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
                "Loading or saving an .orc here automatically sets Trusted .orc folder to this folder. In a score, call await loadOrc(\"local:voice.orc\").",
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
                    set_trusted_orchestra_folder(&path, params, setter);
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
                    set_trusted_orchestra_folder(&path, params, setter);
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

fn trusted_orchestra_folder(path: &std::path::Path) -> Option<&std::path::Path> {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("orc"))
        .then(|| path.parent())
        .flatten()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn set_trusted_orchestra_folder(
    path: &std::path::Path,
    params: &RustelParams,
    setter: &ParamSetter,
) {
    let Some(folder) = trusted_orchestra_folder(path) else {
        return;
    };
    if set_trusted_orchestra_root_value(folder, params) {
        mark_dirty(setter, params);
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
