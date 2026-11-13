use crate::{engine::Shared, params::RustelParams};
use nih_plug::prelude::*;
use nih_plug_egui::{
    EguiState, create_egui_editor,
    egui::{self, Color32, FontId, RichText, TextEdit},
    widgets,
};
use std::{sync::Arc, sync::atomic::Ordering, time::Duration};

pub fn create(params: Arc<RustelParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let state: Arc<EguiState> = params.editor.clone();
    create_egui_editor(
        state,
        (),
        |ctx, _| {
            ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*ctx.style()).clone();
            style.visuals.selection.bg_fill = Color32::from_rgb(87, 56, 125);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(112, 75, 164);
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            ctx.set_style(style);
        },
        move |ctx, setter, _| {
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
        },
    )
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
