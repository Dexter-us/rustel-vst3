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
            egui::TopBottomPanel::top("header").show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("rustel")
                            .size(28.0)
                            .strong()
                            .color(Color32::from_rgb(199, 167, 244)),
                    );
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
                ui.small("Prototype · Built-in synths + bd only · No room/reverb, external samples, MIDI out, or nested plugins");
                ui.add_space(6.0);
            });
            egui::SidePanel::right("controls")
                .exact_width(195.0)
                .show(ctx, |ui| {
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
                ui.small(
                    "Click the editor below to type. Load and Save use .strudel or .txt files.",
                );
                if !file_ui.message.is_empty() {
                    ui.colored_label(Color32::LIGHT_BLUE, &file_ui.message);
                }
                let mut draft = params.draft.lock().clone();
                let mut changed = false;
                egui::ScrollArea::both().show(ui, |ui| {
                    let response = ui.add(
                        TextEdit::multiline(&mut draft)
                            .font(FontId::monospace(15.0))
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(28),
                    );
                    if response.changed() {
                        changed = true;
                    }
                });
                if changed {
                    *params.draft.lock() = draft;
                    mark_dirty(setter, &params);
                }
            });
            draw_score_dialog(ctx, file_ui, &params, setter);
        },
    )
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
