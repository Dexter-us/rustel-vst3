//! Compile an explicit repeating phrase away from the audio callback.
use rustel_engine::{
    ScoreImportAccess, Session, SessionConfig,
    audio::{CompactKabelControls, OnsetEvent, ScalarBackend},
};
use std::sync::atomic::AtomicBool;

pub const MAX_EVENTS: usize = 16_384;

pub struct PreparedNote {
    pub beat: f64,
    pub duration_beats: f64,
    pub event: OnsetEvent<CompactKabelControls>,
}

pub struct PreparedPlan {
    pub backend: ScalarBackend,
    pub notes: Vec<PreparedNote>,
    pub rendered_phrase: Option<Vec<f32>>,
    pub period_beats: f64,
    pub rendered_bpm: f64,
    pub sample_rate: u32,
    pub diagnostics: String,
}

const MAX_RENDERED_PHRASE_BYTES: usize = 128 * 1024 * 1024;

pub fn compile(
    source: &str,
    sample_rate: u32,
    bpm: f64,
    cycles: i32,
    beats_per_cycle: i32,
    allow_csound: bool,
    orchestra_root: &str,
    cancelled: &AtomicBool,
) -> Result<Box<PreparedPlan>, String> {
    if source.len() > 128 * 1024 {
        return Err("Score exceeds the 128 KiB limit.".into());
    }
    let period_beats = f64::from(cycles * beats_per_cycle);
    let beats_per_second = bpm / 60.0;
    let cps = beats_per_second / f64::from(beats_per_cycle);
    let duration = period_beats / beats_per_second;
    let mut import_access = ScoreImportAccess::denied();
    if allow_csound {
        import_access.permit_csound();
        let root = orchestra_root.trim();
        if !root.is_empty() {
            import_access
                .permit_local_root(root)
                .map_err(|error| format!("Could not trust the Csound orchestra folder: {error}"))?;
        }
    }
    let mut session = Session::with_config(
        SessionConfig::default()
            .with_sample_rate(sample_rate)
            .with_cps(cps)
            .with_max_polyphony(64)
            .with_horizon(duration)
            .with_score_import_access(import_access),
    )
    .map_err(|error| error.to_string())?;
    session
        .set_query_hap_budget(MAX_EVENTS as u64)
        .map_err(|error| error.to_string())?;
    session
        .evaluate_no_fallback_cancellable(source, cancelled)
        .map_err(|error| error.to_string())?;
    // The DAW, not setcps/setcpm in a score, owns the clock.
    session.retime(0.0, cps, 0.0);
    if allow_csound {
        let frame_count = (duration * f64::from(sample_rate)).round();
        let samples = frame_count as usize * 2;
        let bytes = samples
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or("Rendered phrase is too large")?;
        if bytes > MAX_RENDERED_PHRASE_BYTES {
            return Err("Csound phrase exceeds the 128 MiB render limit. Reduce phrase length or raise the tempo.".into());
        }
        let rendered_phrase = session
            .render_pcm(duration)
            .map_err(|error| format!("Could not render the score with Csound: {error}"))?;
        if rendered_phrase.is_empty() || rendered_phrase.len() % 2 != 0 {
            return Err("Csound renderer returned an invalid stereo phrase.".into());
        }
        let diagnostics = session
            .take_diagnostics()
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("\n");
        let backend = ScalarBackend::prepared(sample_rate, 64)?;
        return Ok(Box::new(PreparedPlan {
            backend,
            notes: Vec::new(),
            rendered_phrase: Some(rendered_phrase),
            period_beats,
            rendered_bpm: bpm,
            sample_rate,
            diagnostics,
        }));
    }
    let audio_events = session
        .schedule_audio_through(0.0, duration, sample_rate)
        .map_err(|error| error.to_string())?;
    if audio_events.len() > MAX_EVENTS {
        return Err(format!("Phrase exceeds the {MAX_EVENTS}-event limit."));
    }
    let notices = session.take_diagnostics();
    if let Some(refused) = notices.iter().find(|notice| {
        matches!(
            notice.kind.as_str(),
            "voice-refused" | "sample-loading" | "plugin"
        )
    }) {
        return Err(refused.message.clone());
    }
    let diagnostics = notices
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect::<Vec<_>>()
        .join("\n");
    let mut onsets = Vec::with_capacity(audio_events.len());
    for event in audio_events {
        if event.controls.reverb.is_some() {
            return Err(
                "Room/reverb is not supported in this prototype. Remove .room() and use a DAW reverb."
                    .into(),
            );
        }
        let mut onset = OnsetEvent::new(
            event.target_frame,
            event.freq_hz,
            event.gain,
            event.duration_secs,
        )
        .with_controls(event.controls);
        onset.onset_lead = event.onset_lead;
        onset.sample = event.sample;
        onset.wavetable = event.wavetable;
        onset.synth = event.synth;
        onset.cut = event.cut;
        onsets.push(onset);
    }
    let mut backend = ScalarBackend::prepared(sample_rate, 64)?;
    backend.prepare_finite_playback(&onsets)?;
    let mut notes = onsets
        .into_iter()
        .map(|event| PreparedNote {
            beat: (event.onset_frame as f64 - f64::from(event.onset_lead)) / f64::from(sample_rate)
                * beats_per_second,
            duration_beats: f64::from(event.duration_secs) * beats_per_second,
            event: event.into_prepared(),
        })
        .filter(|note| note.beat >= 0.0 && note.beat < period_beats)
        .collect::<Vec<_>>();
    notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    Ok(Box::new(PreparedPlan {
        backend,
        notes,
        rendered_phrase: None,
        period_beats,
        rendered_bpm: bpm,
        sample_rate,
        diagnostics,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustel_engine::{QUERY_WORKER_STACK_BYTES, audio::AudioBackend};

    #[test]
    fn prepares_audible_synth_notes_and_refuses_invalid_javascript() {
        std::thread::Builder::new()
            .stack_size(QUERY_WORKER_STACK_BYTES)
            .spawn(|| {
                let cancelled = AtomicBool::new(false);
                assert!(
                    compile(
                        "not_a_function()",
                        48_000,
                        120.0,
                        1,
                        4,
                        false,
                        "",
                        &cancelled
                    )
                    .is_err()
                );
                let mut plan = compile(
                    r#"note("c4 e4 g4 c5").s("sine").gain(0.2)"#,
                    48_000,
                    120.0,
                    1,
                    4,
                    false,
                    "",
                    &cancelled,
                )
                .expect("valid score");
                assert_eq!(plan.notes.len(), 4);
                assert!((plan.notes[1].beat - 1.0).abs() < 0.001);
                let mut note = plan.notes[0].event;
                note.onset_frame = 0;
                assert!(plan.backend.try_note_prepared(note));
                let mut pcm = [0.0_f32; 128];
                let mut peak = 0.0_f32;
                for _ in 0..100 {
                    plan.backend.process_block(&mut pcm, 64);
                    assert!(pcm.iter().all(|value| value.is_finite()));
                    peak = pcm.iter().fold(peak, |peak, value| peak.max(value.abs()));
                }
                assert!(peak > 0.001, "prepared notes must produce audible PCM");
                assert!(plan.rendered_phrase.is_none());
            })
            .expect("compiler worker")
            .join()
            .expect("audio smoke check");
    }

    #[test]
    fn csound_permission_is_off_by_default() {
        assert!(!crate::params::RustelParams::default().allow_csound.value());
    }
}
