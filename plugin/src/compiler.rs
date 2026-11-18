//! Compile an explicit repeating phrase away from the audio callback.
use rustel_engine::{
    ScoreImportAccess, Session, SessionConfig,
    audio::{CompactKabelControls, InsertNote, InsertSlot, OnsetEvent, ScalarBackend},
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub const MAX_EVENTS: usize = 16_384;

pub struct PreparedNote {
    pub beat: f64,
    pub duration_beats: f64,
    pub event: OnsetEvent<CompactKabelControls>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedMidiEvent {
    pub frame: i64,
    pub channel: u8,
    pub note: u8,
    pub velocity: f32,
    pub is_note_on: bool,
}

pub enum LiveVstiCommand<'a> {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: f32,
        frame_offset: u32,
    },
    NoteOff {
        channel: u8,
        note: u8,
        frame_offset: u32,
    },
    Submit {
        left: &'a mut [f32],
        right: &'a mut [f32],
        beats: f64,
        tempo: f32,
    },
    Collect {
        left: &'a mut [f32],
        right: &'a mut [f32],
    },
    Reset,
}

pub type LiveVstiProcessor = Box<dyn for<'a> FnMut(LiveVstiCommand<'a>) -> bool + Send>;

pub struct PreparedPlan {
    pub backend: ScalarBackend,
    pub notes: Vec<PreparedNote>,
    pub rendered_phrase: Option<Vec<f32>>,
    pub midi_output_events: Vec<PreparedMidiEvent>,
    pub hosted_midi_synth: RustelMidiSynth,
    pub requires_csound: bool,
    pub rendered_source: Option<&'static str>,
    pub live_midi_plugin: Option<String>,
    pub live_vsti: Option<LiveVstiProcessor>,
    pub period_beats: f64,
    pub rendered_bpm: f64,
    pub sample_rate: u32,
    pub diagnostics: String,
}

const MAX_RENDERED_PHRASE_BYTES: usize = 128 * 1024 * 1024;
static MIDI_TARGET_CHAIN: AtomicU64 = AtomicU64::new(u64::MAX / 2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RustelMidiSynth {
    #[default]
    Sine,
    Triangle,
    Square,
    Sawtooth,
    Supersaw,
    Pulse,
    Sbd,
    WhiteNoise,
    PinkNoise,
    BrownNoise,
    Crackle,
    ByteBeat,
    ZzFx,
    One,
}

impl RustelMidiSynth {
    fn parse(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "sine" => Ok(Self::Sine),
            "triangle" => Ok(Self::Triangle),
            "square" => Ok(Self::Square),
            "saw" | "sawtooth" => Ok(Self::Sawtooth),
            "supersaw" => Ok(Self::Supersaw),
            "pulse" => Ok(Self::Pulse),
            "sbd" => Ok(Self::Sbd),
            "white" | "whitenoise" => Ok(Self::WhiteNoise),
            "pink" | "pinknoise" => Ok(Self::PinkNoise),
            "brown" | "brownnoise" => Ok(Self::BrownNoise),
            "crackle" => Ok(Self::Crackle),
            "bytebeat" => Ok(Self::ByteBeat),
            "zzfx" => Ok(Self::ZzFx),
            "one" => Ok(Self::One),
            _ => Err(format!(
                "Unknown Rustel MIDI synth '{name}'. Use sine, triangle, square, sawtooth, supersaw, pulse, sbd, white, pink, brown, crackle, bytebeat, zzfx, or one."
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HostedMidiTarget {
    Rustel(RustelMidiSynth),
    Vsti(String),
    Csound(String),
    Sample(String),
    Wavetable(String),
    Graph(String),
}

struct MidiTargetNote {
    start: usize,
    length: u32,
    pitch: u8,
    velocity: f32,
}

fn midi_target(source: &str) -> Result<Option<HostedMidiTarget>, String> {
    let mut target = None;
    for line in source.lines() {
        let Some(value) = line.trim().strip_prefix("// MIDI target: ") else {
            continue;
        };
        let (kind, payload) = value
            .trim()
            .split_once(char::is_whitespace)
            .ok_or_else(|| "MIDI target needs a type and a name.".to_owned())?;
        let payload = payload.trim();
        let kind = kind.to_ascii_lowercase();
        let payload = unquote_target_payload(payload);
        if payload.is_empty() {
            return Err("MIDI target is missing its name.".into());
        }
        if payload.chars().any(char::is_control) {
            return Err("MIDI target values cannot contain control characters.".into());
        }
        let parsed = match kind.as_str() {
            "vsti" => HostedMidiTarget::Vsti(payload.to_owned()),
            "csound" => HostedMidiTarget::Csound(payload.to_owned()),
            "rustel" => HostedMidiTarget::Rustel(RustelMidiSynth::parse(payload)?),
            "sample" => HostedMidiTarget::Sample(payload.to_owned()),
            "wavetable" => HostedMidiTarget::Wavetable(payload.to_owned()),
            "graph" if payload.len() <= 16 * 1024 => HostedMidiTarget::Graph(payload.to_owned()),
            "graph" => {
                return Err("KabelSalat MIDI target expressions are limited to 16 KiB.".into());
            }
            _ => {
                return Err(format!(
                    "Unknown MIDI target type '{kind}'. Use rustel, vsti, csound, sample, wavetable, or graph."
                ));
            }
        };
        if target.replace(parsed).is_some() {
            return Err("Score may specify only one MIDI target.".into());
        }
    }
    Ok(target)
}

fn live_midi_source(source: &str) -> Result<Option<String>, String> {
    let mut selected = None;
    for line in source.lines() {
        let Some(value) = line.trim().strip_prefix("// LIVE MIDI SOURCE: ") else {
            continue;
        };
        let name = serde_json::from_str::<String>(value.trim())
            .map_err(|_| "The live MIDI source name is invalid.".to_owned())?;
        if name.trim().is_empty() || name.chars().any(char::is_control) {
            return Err("The live MIDI source name is invalid.".into());
        }
        if selected.replace(name).is_some() {
            return Err("A score can contain only one live MIDI source.".into());
        }
    }
    Ok(selected)
}

fn unquote_target_payload(payload: &str) -> &str {
    let Some(quote @ ('"' | '\'')) = payload.chars().next() else {
        return payload;
    };
    if payload.ends_with(quote) && payload.len() >= quote.len_utf8() * 2 {
        &payload[quote.len_utf8()..payload.len() - quote.len_utf8()]
    } else {
        payload
    }
}

fn source_uses_csound(source: &str) -> bool {
    let source = source.to_ascii_lowercase();
    [".csound", "loadcsound", "loadorc"]
        .iter()
        .any(|needle| source.contains(needle))
}

fn pair_midi_notes(events: &[PreparedMidiEvent], phrase_frames: usize) -> Vec<MidiTargetNote> {
    let mut open: HashMap<(u8, u8), VecDeque<(i64, f32)>> = HashMap::new();
    let mut notes = Vec::new();
    for event in events {
        let key = (event.channel, event.note);
        if event.is_note_on && event.velocity > 0.0 {
            open.entry(key)
                .or_default()
                .push_back((event.frame.max(0), event.velocity));
        } else if let Some((start, velocity)) = open.get_mut(&key).and_then(VecDeque::pop_front) {
            let start = start.min(phrase_frames as i64) as usize;
            let end = event.frame.max(0).min(phrase_frames as i64) as usize;
            if end > start {
                notes.push(MidiTargetNote {
                    start,
                    length: (end - start).min(u32::MAX as usize) as u32,
                    pitch: event.note,
                    velocity,
                });
            }
        }
    }
    for ((_, pitch), pending) in open {
        for (start, velocity) in pending {
            let start = start.max(0).min(phrase_frames as i64) as usize;
            if start < phrase_frames {
                notes.push(MidiTargetNote {
                    start,
                    length: (phrase_frames - start).min(u32::MAX as usize) as u32,
                    pitch,
                    velocity,
                });
            }
        }
    }
    notes.sort_by_key(|note| note.start);
    notes
}

fn csound_midi_source(
    target_name: &str,
    notes: &[MidiTargetNote],
    phrase_cycles: i32,
    phrase_frames: usize,
) -> Result<String, String> {
    if phrase_cycles < 1 || phrase_frames == 0 {
        return Err("Csound MIDI routing needs a nonempty phrase.".into());
    }
    if notes.is_empty() {
        return Err("The hosted MIDI VST emitted no notes for the Csound target.".into());
    }
    if notes.len() > 1_024 {
        return Err("Csound MIDI routing is limited to 1,024 notes per prepared phrase.".into());
    }
    if target_name.chars().any(char::is_control) {
        return Err("Csound instrument names cannot contain control characters.".into());
    }
    let target_literal = format!(
        "\"{}\"",
        target_name.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let mut patterns = Vec::with_capacity(notes.len());
    for note in notes {
        let start_cycles = note.start as f64 / phrase_frames as f64 * f64::from(phrase_cycles);
        let gate_ratio = note.length as f64 / phrase_frames as f64;
        patterns.push(format!(
            "note({}).csoundm({target_literal}).gain({:.8}).slow({phrase_cycles}).late({start_cycles:.12}).legato({gate_ratio:.12})",
            note.pitch, note.velocity
        ));
    }
    let source = format!("$: stack({})\n", patterns.join(",\n"));
    if source.len() > 128 * 1024 {
        return Err("Generated Csound MIDI score exceeds the 128 KiB limit.".into());
    }
    Ok(source)
}

fn render_midi_to_csound(
    session: &mut Session,
    target_name: &str,
    events: &[PreparedMidiEvent],
    sample_rate: u32,
    bpm: f64,
    phrase_cycles: i32,
    phrase_frames: usize,
    duration_secs: f64,
    cancelled: &AtomicBool,
) -> Result<Vec<f32>, String> {
    let notes = pair_midi_notes(events, phrase_frames);
    let source = csound_midi_source(target_name, &notes, phrase_cycles, phrase_frames)?;
    session
        .evaluate_no_fallback_cancellable(&source, cancelled)
        .map_err(|error| format!("Could not prepare Csound MIDI target: {error}"))?;
    let cps = f64::from(phrase_cycles) / duration_secs;
    session.retime(0.0, cps, 0.0);
    let audio = session
        .render_pcm(duration_secs)
        .map_err(|error| format!("Could not render the Csound MIDI target: {error}"))?;
    if audio.is_empty() || audio.len() % 2 != 0 {
        return Err("Csound MIDI target returned invalid stereo audio.".into());
    }
    if audio.len() != phrase_frames.saturating_mul(2) {
        return Err(format!(
            "Csound MIDI target rendered {} frames; expected {phrase_frames} at {sample_rate} Hz and {bpm:.2} BPM.",
            audio.len() / 2
        ));
    }
    Ok(audio)
}

fn rustel_midi_source(
    target: &HostedMidiTarget,
    notes: &[MidiTargetNote],
    phrase_cycles: i32,
    phrase_frames: usize,
) -> Result<String, String> {
    if phrase_cycles < 1 || phrase_frames == 0 {
        return Err("Rustel MIDI routing needs a nonempty phrase.".into());
    }
    if notes.is_empty() {
        return Err("The hosted MIDI VST emitted no notes for the Rustel target.".into());
    }
    if notes.len() > 1_024 {
        return Err("Rustel MIDI routing is limited to 1,024 notes per prepared phrase.".into());
    }

    let mut patterns = Vec::with_capacity(notes.len());
    for note in notes {
        let start_cycles = note.start as f64 / phrase_frames as f64 * f64::from(phrase_cycles);
        let gate_ratio = note.length as f64 / phrase_frames as f64;
        let event = match target {
            HostedMidiTarget::Sample(name) | HostedMidiTarget::Wavetable(name) => {
                format!("note({}).s({})", note.pitch, js_string_literal(name))
            }
            HostedMidiTarget::Graph(expression) => {
                format!("K({expression}).note({})", note.pitch)
            }
            _ => return Err("This MIDI target is not a Rustel sound.".into()),
        };
        patterns.push(format!(
            "{event}.gain({:.8}).slow({phrase_cycles}).late({start_cycles:.12}).legato({gate_ratio:.12})",
            note.velocity
        ));
    }
    let source = format!("$: stack({})\n", patterns.join(",\n"));
    if source.len() > 128 * 1024 {
        return Err("Generated Rustel MIDI score exceeds the 128 KiB limit.".into());
    }
    Ok(source)
}

fn js_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn render_midi_to_rustel(
    session: &mut Session,
    target: &HostedMidiTarget,
    events: &[PreparedMidiEvent],
    phrase_cycles: i32,
    phrase_frames: usize,
    sample_rate: u32,
    bpm: f64,
    duration_secs: f64,
    cancelled: &AtomicBool,
) -> Result<Vec<f32>, String> {
    let notes = pair_midi_notes(events, phrase_frames);
    let source = rustel_midi_source(target, &notes, phrase_cycles, phrase_frames)?;
    if let HostedMidiTarget::Sample(name) | HostedMidiTarget::Wavetable(name) = target
        && name != "bd"
    {
        session
            .enable_default_samples()
            .map_err(|error| format!("Could not enable Rustel's sample library: {error}"))?;
        session
            .prefetch_sounds_checked(std::slice::from_ref(name))
            .map_err(|error| format!("Could not load Rustel MIDI target '{name}': {error}"))?;
        let (_, timed_out) = session.wait_for_sample_loads_until(Duration::from_secs(30), || {
            cancelled.load(Ordering::Relaxed)
        });
        if cancelled.load(Ordering::Relaxed) {
            return Err("Score evaluation was cancelled.".into());
        }
        if timed_out {
            return Err(format!(
                "Timed out loading Rustel MIDI target '{name}'. Check the sample name and network connection."
            ));
        }
    }

    session
        .evaluate_no_fallback_cancellable(&source, cancelled)
        .map_err(|error| format!("Could not prepare the Rustel MIDI target: {error}"))?;
    session.retime(0.0, f64::from(phrase_cycles) / duration_secs, 0.0);
    let audio = session
        .render_pcm(duration_secs)
        .map_err(|error| format!("Could not render the Rustel MIDI target: {error}"))?;
    if audio.is_empty() || audio.len() % 2 != 0 {
        return Err("Rustel MIDI target returned invalid stereo audio.".into());
    }
    if audio.len() != phrase_frames.saturating_mul(2) {
        return Err(format!(
            "Rustel MIDI target rendered {} frames; expected {phrase_frames} at {sample_rate} Hz and {bpm:.2} BPM.",
            audio.len() / 2
        ));
    }
    Ok(audio)
}

fn close_midi_notes_at_phrase_end(events: &mut Vec<PreparedMidiEvent>, phrase_frames: usize) {
    if phrase_frames == 0 {
        events.clear();
        return;
    }
    let last_frame = phrase_frames.saturating_sub(1) as i64;
    events.retain_mut(|event| {
        if event.frame < 0 {
            return false;
        }
        if event.is_note_on && event.velocity > 0.0 {
            return event.frame < last_frame;
        }
        event.frame = event.frame.min(last_frame);
        true
    });
    events.sort_by_key(|event| (event.frame, event.is_note_on));

    let mut open: HashMap<(u8, u8), VecDeque<PreparedMidiEvent>> = HashMap::new();
    for event in events.iter() {
        let key = (event.channel, event.note);
        if event.is_note_on && event.velocity > 0.0 {
            open.entry(key).or_default().push_back(*event);
        } else if let Some(queue) = open.get_mut(&key) {
            queue.pop_front();
        }
    }
    for ((channel, note), pending) in open {
        for _ in pending {
            events.push(PreparedMidiEvent {
                frame: last_frame,
                channel,
                note,
                velocity: 0.0,
                is_note_on: false,
            });
        }
    }
    events.sort_by_key(|event| (event.frame, event.is_note_on));
}

fn prepare_live_vsti_target(
    target_name: &str,
    sample_rate: u32,
    cancelled: &AtomicBool,
) -> Result<LiveVstiProcessor, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Score evaluation was cancelled.".into());
    }
    let host = rustel_runtime::vst::host();
    match host.resolve(target_name, true) {
        rustel_runtime::vst::Resolved::Ready(_, plugin) if plugin.is_instrument() => {}
        rustel_runtime::vst::Resolved::Ready(_, _) => {
            return Err(format!(
                "{target_name} is an effect; live MIDI targets must be VSTi instruments."
            ));
        }
        rustel_runtime::vst::Resolved::Pending => {
            return Err(format!(
                "{target_name} is still loading; evaluate the score again."
            ));
        }
        rustel_runtime::vst::Resolved::Missing => {
            return Err(format!("No VST3 instrument has the name '{target_name}'."));
        }
        rustel_runtime::vst::Resolved::Failed(reason) => {
            return Err(format!("{target_name} did not load: {reason}"));
        }
    }

    let key = match host
        .key(target_name, None, true)
        .map_err(|error| error.to_string())?
    {
        Some((key, _)) => key,
        None => {
            host.wait_idle();
            host.key(target_name, None, true)
                .map_err(|error| error.to_string())?
                .map(|(key, _)| key)
                .ok_or_else(|| format!("{target_name} is not ready to receive live MIDI."))?
        }
    };
    if cancelled.load(Ordering::Relaxed) {
        return Err("Score evaluation was cancelled.".into());
    }
    let slot = InsertSlot::instrument(MIDI_TARGET_CHAIN.fetch_add(1, Ordering::Relaxed));
    let mut instrument = host
        .insert_live_midi(key, sample_rate, slot)
        .ok_or_else(|| format!("Could not prepare live VSTi target {target_name}."))?;

    Ok(Box::new(move |command| match command {
        LiveVstiCommand::NoteOn {
            channel,
            note,
            velocity,
            frame_offset,
        } => instrument.note_on(channel, note, velocity, frame_offset),
        LiveVstiCommand::NoteOff {
            channel,
            note,
            frame_offset,
        } => instrument.note_off(channel, note, frame_offset),
        LiveVstiCommand::Submit {
            left,
            right,
            beats,
            tempo,
        } => {
            instrument.sync(beats, tempo, 0);
            instrument.submit(left, right);
            true
        }
        LiveVstiCommand::Collect { left, right } => {
            instrument.collect(left, right);
            true
        }
        LiveVstiCommand::Reset => {
            instrument.reset();
            true
        }
    }))
}

fn render_midi_to_vsti(
    target_name: &str,
    events: &[PreparedMidiEvent],
    sample_rate: u32,
    bpm: f64,
    phrase_frames: usize,
    cancelled: &AtomicBool,
) -> Result<Vec<f32>, String> {
    let notes = pair_midi_notes(events, phrase_frames);
    if notes.is_empty() {
        return Err(
            "The hosted MIDI VST emitted no complete notes for the requested VSTi target.".into(),
        );
    }

    let host = rustel_runtime::vst::host();
    match host.resolve(target_name, true) {
        rustel_runtime::vst::Resolved::Ready(_, plugin) if plugin.is_instrument() => {}
        rustel_runtime::vst::Resolved::Ready(_, _) => {
            return Err(format!(
                "{target_name} is an effect; MIDI targets must be VSTi instruments."
            ));
        }
        rustel_runtime::vst::Resolved::Pending => {
            return Err(format!(
                "{target_name} is still loading; evaluate the score again."
            ));
        }
        rustel_runtime::vst::Resolved::Missing => {
            return Err(format!("No VST3 instrument has the name '{target_name}'."));
        }
        rustel_runtime::vst::Resolved::Failed(reason) => {
            return Err(format!("{target_name} did not load: {reason}"));
        }
    }
    let key = match host
        .key(target_name, None, true)
        .map_err(|error| error.to_string())?
    {
        Some((key, _)) => key,
        None => {
            host.wait_idle();
            host.key(target_name, None, true)
                .map_err(|error| error.to_string())?
                .map(|(key, _)| key)
                .ok_or_else(|| format!("{target_name} is not ready to render MIDI."))?
        }
    };

    let chain = MIDI_TARGET_CHAIN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slot = InsertSlot::instrument(chain);
    let rendered = (|| {
        let ready = host
            .prepare_render(key, sample_rate, slot, &[], true)
            .map_err(|error| format!("Could not prepare {target_name}: {error:?}"))?;
        match ready.recv_timeout(Duration::from_secs(120)) {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return Err(format!("{target_name} did not start: {error}")),
            Err(error) => return Err(format!("Timed out waiting for {target_name}: {error}")),
        }
        let stop = || cancelled.load(Ordering::Relaxed);
        let mut instrument = host
            .insert_render(key, sample_rate, slot, Some(&stop))
            .ok_or_else(|| format!("Could not start rendering {target_name}."))?;
        let mut audio = vec![0.0; phrase_frames.saturating_mul(2)];
        let mut cursor = 0;
        let mut note_index = 0;
        while cursor < phrase_frames {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Score evaluation was cancelled.".into());
            }
            let frames = (phrase_frames - cursor).min(128);
            instrument.sync(
                cursor as f64 * bpm / 60.0 / f64::from(sample_rate),
                bpm as f32,
                0,
            );
            while let Some(note) = notes.get(note_index)
                && note.start < cursor + frames
            {
                if note.start >= cursor {
                    instrument.note(
                        InsertNote {
                            pitch: f32::from(note.pitch),
                            velocity: note.velocity,
                            frames: note.length,
                        },
                        (note.start - cursor) as u32,
                    );
                }
                note_index += 1;
            }
            let mut left = [0.0f32; 128];
            let mut right = [0.0f32; 128];
            instrument.process(&mut left[..frames], &mut right[..frames]);
            for index in 0..frames {
                audio[(cursor + index) * 2] = left[index];
                audio[(cursor + index) * 2 + 1] = right[index];
            }
            cursor += frames;
        }
        Ok(audio)
    })();
    let faults = host.take_render_faults(&[chain]);
    let errors = host.take_render_errors(&[chain]);
    host.keep(chain, Vec::new(), Vec::new(), Vec::new(), vec![chain]);
    let audio = rendered?;
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    if !faults.refusals.is_empty() {
        return Err(faults.refusals.join("\n"));
    }
    if !faults.warnings.is_empty() {
        return Err(faults.warnings.join("\n"));
    }
    Ok(audio)
}

pub fn compile(
    source: &str,
    sample_rate: u32,
    bpm: f64,
    cycles: i32,
    beats_per_cycle: i32,
    allow_csound: bool,
    orchestra_root: &str,
    vst3_folders: &str,
    cancelled: &AtomicBool,
) -> Result<Box<PreparedPlan>, String> {
    if source.len() > 128 * 1024 {
        return Err("Score exceeds the 128 KiB limit.".into());
    }
    let live_source = live_midi_source(source)?;
    let midi_target = midi_target(source)?;
    let uses_vst = !rustel_runtime::vst::names_in_source(source).is_empty();
    if midi_target.is_some() && !uses_vst && live_source.is_none() {
        return Err("A MIDI target needs a hosted VST3 MIDI source in the score.".into());
    }
    if let Some(source_name) = live_source {
        let period_beats = f64::from(cycles * beats_per_cycle);
        if !period_beats.is_finite()
            || period_beats <= 0.0
            || !bpm.is_finite()
            || bpm <= 0.0
            || !(8_000..=192_000).contains(&sample_rate)
        {
            return Err(
                "Live MIDI playback needs a valid tempo, phrase length, and sample rate.".into(),
            );
        }
        let (synth, live_vsti) = match midi_target {
            Some(HostedMidiTarget::Rustel(synth)) => (synth, None),
            Some(HostedMidiTarget::Vsti(target_name)) => {
                crate::portable_vst::configure(vst3_folders, cancelled)?;
                let live_vsti = prepare_live_vsti_target(&target_name, sample_rate, cancelled)?;
                (RustelMidiSynth::Sine, Some(live_vsti))
            }
            Some(_) => {
                return Err(
                    "Free-running MIDI can target a Rustel synth or a hosted VSTi. Connect one of those instrument types."
                        .into(),
                );
            }
            None => {
                return Err("Choose a Rustel synth or hosted VSTi as the live MIDI target.".into());
            }
        };
        return Ok(Box::new(PreparedPlan {
            backend: ScalarBackend::prepared(sample_rate, 64)?,
            notes: Vec::new(),
            rendered_phrase: None,
            midi_output_events: Vec::new(),
            hosted_midi_synth: synth,
            requires_csound: false,
            rendered_source: Some("Free-running MIDI VST3"),
            live_midi_plugin: Some(source_name.clone()),
            live_vsti,
            period_beats,
            rendered_bpm: bpm,
            sample_rate,
            diagnostics: format!(
                "Live MIDI source: {source_name}. Open its editor and start streaming."
            ),
        }));
    }
    if matches!(midi_target.as_ref(), Some(HostedMidiTarget::Csound(_))) && !allow_csound {
        return Err("Enable Csound support before routing hosted MIDI to Csound.".into());
    }
    let requires_csound = (allow_csound && source_uses_csound(source))
        || matches!(midi_target.as_ref(), Some(HostedMidiTarget::Csound(_)));
    let period_beats = f64::from(cycles * beats_per_cycle);
    let beats_per_second = bpm / 60.0;
    let cps = beats_per_second / f64::from(beats_per_cycle);
    let duration = period_beats / beats_per_second;
    if uses_vst {
        crate::portable_vst::configure(vst3_folders, cancelled)?;
    }
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
    if allow_csound || uses_vst {
        let frame_count = (duration * f64::from(sample_rate)).round();
        let samples = frame_count as usize * 2;
        let bytes = samples
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or("Rendered phrase is too large")?;
        if bytes > MAX_RENDERED_PHRASE_BYTES {
            return Err(
                "Rendered phrase exceeds the 128 MiB limit. Reduce phrase length or raise the tempo."
                    .into(),
            );
        }
        let vst_host = rustel_runtime::vst::host();
        if uses_vst {
            vst_host.begin_midi_output_capture();
        }
        let rendered_result = session.render_pcm(duration);
        if uses_vst {
            vst_host.end_midi_output_capture();
        }
        let mut midi_output_events = if uses_vst {
            vst_host
                .take_midi_output_events()
                .into_iter()
                .map(|event| PreparedMidiEvent {
                    frame: event.frame,
                    channel: event.channel,
                    note: event.note,
                    velocity: event.velocity,
                    is_note_on: event.is_note_on,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut rendered_phrase = rendered_result.map_err(|error| {
            if uses_vst {
                format!("Could not render the score with VST3 plugins: {error}")
            } else {
                format!("Could not render the score with Csound: {error}")
            }
        })?;
        if rendered_phrase.is_empty() || rendered_phrase.len() % 2 != 0 {
            return Err("Phrase renderer returned invalid stereo audio.".into());
        }
        close_midi_notes_at_phrase_end(&mut midi_output_events, rendered_phrase.len() / 2);
        let phrase_frames = rendered_phrase.len() / 2;
        let mut notices = session.take_diagnostics();
        if uses_vst {
            let plugin_errors = notices
                .iter()
                .filter(|diagnostic| diagnostic.kind == "plugin")
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>();
            if !plugin_errors.is_empty() {
                return Err(plugin_errors.join("\n"));
            }
        }
        let target_audio = match midi_target.as_ref() {
            Some(HostedMidiTarget::Vsti(target_name)) => Some(render_midi_to_vsti(
                target_name,
                &midi_output_events,
                sample_rate,
                bpm,
                phrase_frames,
                cancelled,
            )?),
            Some(HostedMidiTarget::Csound(target_name)) => Some(render_midi_to_csound(
                &mut session,
                target_name,
                &midi_output_events,
                sample_rate,
                bpm,
                cycles,
                phrase_frames,
                duration,
                cancelled,
            )?),
            Some(
                target @ (HostedMidiTarget::Sample(_)
                | HostedMidiTarget::Wavetable(_)
                | HostedMidiTarget::Graph(_)),
            ) => Some(render_midi_to_rustel(
                &mut session,
                target,
                &midi_output_events,
                cycles,
                phrase_frames,
                sample_rate,
                bpm,
                duration,
                cancelled,
            )?),
            Some(HostedMidiTarget::Rustel(_)) | None => None,
        };
        if let Some(target_audio) = target_audio {
            for (output, target) in rendered_phrase.iter_mut().zip(target_audio) {
                *output += target;
            }
        }
        notices.extend(session.take_diagnostics());
        if matches!(
            midi_target.as_ref(),
            Some(
                HostedMidiTarget::Sample(_)
                    | HostedMidiTarget::Wavetable(_)
                    | HostedMidiTarget::Graph(_)
            )
        ) && let Some(refused) = notices.iter().find(|diagnostic| {
            matches!(diagnostic.kind.as_str(), "voice-refused" | "sample-loading")
        }) {
            return Err(format!(
                "Rustel MIDI target could not be prepared: {}",
                refused.message
            ));
        }
        let diagnostics = notices
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("\n");
        let backend = ScalarBackend::prepared(sample_rate, 64)?;
        return Ok(Box::new(PreparedPlan {
            backend,
            notes: Vec::new(),
            rendered_phrase: Some(rendered_phrase),
            midi_output_events: if matches!(
                midi_target.as_ref(),
                Some(
                    HostedMidiTarget::Vsti(_)
                        | HostedMidiTarget::Csound(_)
                        | HostedMidiTarget::Sample(_)
                        | HostedMidiTarget::Wavetable(_)
                        | HostedMidiTarget::Graph(_)
                )
            ) {
                Vec::new()
            } else {
                midi_output_events
            },
            hosted_midi_synth: match midi_target.as_ref() {
                Some(HostedMidiTarget::Rustel(synth)) => *synth,
                _ => RustelMidiSynth::Sine,
            },
            requires_csound,
            rendered_source: Some(match midi_target.as_ref() {
                Some(HostedMidiTarget::Csound(_)) => "VST3 + Csound MIDI phrase",
                Some(HostedMidiTarget::Sample(_)) => "VST3 + Rustel sample MIDI phrase",
                Some(HostedMidiTarget::Wavetable(_)) => "VST3 + Rustel wavetable MIDI phrase",
                Some(HostedMidiTarget::Graph(_)) => "VST3 + KabelSalat MIDI phrase",
                _ if uses_vst => "VST3-rendered phrase",
                _ => "Csound-rendered phrase",
            }),
            live_midi_plugin: None,
            live_vsti: None,
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
        midi_output_events: Vec::new(),
        hosted_midi_synth: RustelMidiSynth::Sine,
        requires_csound: false,
        rendered_source: None,
        live_midi_plugin: None,
        live_vsti: None,
        period_beats,
        rendered_bpm: bpm,
        sample_rate,
        diagnostics,
    }))
}

#[cfg(test)]
mod midi_routing_tests {
    use super::*;
    use rustel_engine::QUERY_WORKER_STACK_BYTES;

    #[test]
    fn parses_explicit_midi_targets() {
        assert_eq!(
            midi_target("// MIDI target: vsti \"Serum 2\"\n$: note('c4').vst('Arp')")
                .expect("target parses"),
            Some(HostedMidiTarget::Vsti("Serum 2".into()))
        );
        assert_eq!(
            midi_target("// MIDI target: csound \"triangle organ\"").expect("target parses"),
            Some(HostedMidiTarget::Csound("triangle organ".into()))
        );
        assert_eq!(
            midi_target("// MIDI target: rustel \"supersaw\"").expect("target parses"),
            Some(HostedMidiTarget::Rustel(RustelMidiSynth::Supersaw))
        );
        assert_eq!(
            midi_target("// MIDI target: sample \"bd\"").expect("target parses"),
            Some(HostedMidiTarget::Sample("bd".into()))
        );
        assert_eq!(
            midi_target("// MIDI target: wavetable \"wt_x\"").expect("target parses"),
            Some(HostedMidiTarget::Wavetable("wt_x".into()))
        );
        assert_eq!(
            midi_target("// MIDI target: graph sine(sFreq).mul(0.2).out()").expect("target parses"),
            Some(HostedMidiTarget::Graph("sine(sFreq).mul(0.2).out()".into()))
        );
        assert!(midi_target("// MIDI target: vsti \"\"").is_err());
        assert!(midi_target("// MIDI target: rustel \"unknown\"").is_err());
        assert!(midi_target("// MIDI target: vsti \"one\"\n// MIDI target: vsti \"two\"").is_err());
    }

    #[test]
    fn parses_a_single_live_midi_source_marker() {
        assert_eq!(
            live_midi_source("// LIVE MIDI SOURCE: \"MIDI \\\"Generator\\\"\""),
            Ok(Some("MIDI \"Generator\"".into()))
        );
        assert!(
            live_midi_source(
                "// LIVE MIDI SOURCE: \"Generator A\"\n// LIVE MIDI SOURCE: \"Generator B\""
            )
            .is_err()
        );
        assert!(live_midi_source("// LIVE MIDI SOURCE: not-json").is_err());
    }

    #[test]
    fn free_running_midi_source_builds_a_live_rustel_synth_plan() {
        let plan = compile(
            "// LIVE MIDI SOURCE: \"MIDI Generator\"\n// MIDI target: rustel \"triangle\"",
            48_000,
            120.0,
            2,
            4,
            false,
            "",
            "",
            &AtomicBool::new(false),
        )
        .expect("live MIDI source plan");
        assert!(plan.rendered_phrase.is_none());
        assert_eq!(plan.hosted_midi_synth, RustelMidiSynth::Triangle);
        assert_eq!(plan.rendered_source, Some("Free-running MIDI VST3"));
    }

    #[test]
    fn live_midi_source_requires_a_supported_live_target() {
        let error = match compile(
            "// LIVE MIDI SOURCE: \"MIDI Generator\"\n// MIDI target: sample \"bd\"",
            48_000,
            120.0,
            2,
            4,
            false,
            "",
            "",
            &AtomicBool::new(false),
        ) {
            Err(error) => error,
            Ok(_) => panic!("live sample target should be rejected"),
        };
        assert!(error.contains("Rustel synth or a hosted VSTi"));
    }

    #[test]
    fn pairs_generated_notes_by_channel_and_pitch() {
        let events = [
            PreparedMidiEvent {
                frame: 12,
                channel: 0,
                note: 60,
                velocity: 0.75,
                is_note_on: true,
            },
            PreparedMidiEvent {
                frame: 20,
                channel: 1,
                note: 60,
                velocity: 0.5,
                is_note_on: true,
            },
            PreparedMidiEvent {
                frame: 48,
                channel: 0,
                note: 60,
                velocity: 0.0,
                is_note_on: false,
            },
            PreparedMidiEvent {
                frame: 72,
                channel: 1,
                note: 60,
                velocity: 0.0,
                is_note_on: false,
            },
        ];
        let notes = pair_midi_notes(&events, 100);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].start, 12);
        assert_eq!(notes[0].length, 36);
        assert_eq!(notes[0].velocity, 0.75);
        assert_eq!(notes[1].start, 20);
        assert_eq!(notes[1].length, 52);
        assert_eq!(notes[1].velocity, 0.5);
    }

    #[test]
    fn closes_unreleased_generated_notes_before_the_phrase_wraps() {
        let mut events = vec![PreparedMidiEvent {
            frame: 30,
            channel: 0,
            note: 64,
            velocity: 0.8,
            is_note_on: true,
        }];
        close_midi_notes_at_phrase_end(&mut events, 100);
        assert_eq!(events.len(), 2);
        assert!(!events[1].is_note_on);
        assert_eq!(events[1].frame, 99);
        assert_eq!(pair_midi_notes(&events, 100)[0].length, 69);
    }

    #[test]
    fn creates_a_timed_csound_pattern_from_captured_midi() {
        let source = csound_midi_source(
            "triangle organ",
            &[MidiTargetNote {
                start: 25,
                length: 50,
                pitch: 60,
                velocity: 0.75,
            }],
            2,
            100,
        )
        .expect("target pattern");
        assert!(source.contains("note(60).csoundm(\"triangle organ\")"));
        assert!(
            source
                .contains(".gain(0.75000000).slow(2).late(0.500000000000).legato(0.500000000000)")
        );
    }

    #[test]
    fn creates_timed_sample_wavetable_and_graph_patterns_from_midi() {
        let note = MidiTargetNote {
            start: 25,
            length: 50,
            pitch: 60,
            velocity: 0.75,
        };
        let sample = rustel_midi_source(
            &HostedMidiTarget::Sample("bd".into()),
            std::slice::from_ref(&note),
            2,
            100,
        )
        .expect("sample target");
        assert!(sample.contains("note(60).s(\"bd\")"));
        assert!(sample.contains(".gain(0.75000000).slow(2).late(0.500000000000)"));

        let wavetable = rustel_midi_source(
            &HostedMidiTarget::Wavetable("wt_x".into()),
            std::slice::from_ref(&note),
            2,
            100,
        )
        .expect("wavetable target");
        assert!(wavetable.contains("note(60).s(\"wt_x\")"));

        let graph = rustel_midi_source(
            &HostedMidiTarget::Graph("sine(sFreq).mul(0.2).out()".into()),
            &[note],
            2,
            100,
        )
        .expect("graph target");
        assert!(graph.contains("K(sine(sFreq).mul(0.2).out()).note(60)"));
        assert!(graph.contains(".legato(0.500000000000)"));
    }

    #[test]
    fn bundled_sample_and_graph_midi_patterns_render() {
        std::thread::Builder::new()
            .stack_size(QUERY_WORKER_STACK_BYTES)
            .spawn(|| {
                let note = MidiTargetNote {
                    start: 0,
                    length: 24_000,
                    pitch: 60,
                    velocity: 0.8,
                };
                for target in [
                    HostedMidiTarget::Sample("bd".into()),
                    HostedMidiTarget::Graph("sine(sFreq).mul(0.2).out()".into()),
                ] {
                    let source =
                        rustel_midi_source(&target, std::slice::from_ref(&note), 1, 48_000)
                            .expect("Rustel target pattern");
                    let mut session = Session::with_config(
                        SessionConfig::default()
                            .with_sample_rate(48_000)
                            .with_cps(1.0)
                            .with_horizon(1.0),
                    )
                    .expect("session");
                    session
                        .evaluate_no_fallback_cancellable(&source, &AtomicBool::new(false))
                        .expect("target score evaluates");
                    session.retime(0.0, 1.0, 0.0);
                    let pcm = session.render_pcm(1.0).expect("target renders");
                    assert_eq!(pcm.len(), 96_000);
                    assert!(
                        pcm.iter().any(|sample| sample.abs() > 0.001),
                        "{target:?} MIDI target should produce audio"
                    );
                }
            })
            .expect("render test worker")
            .join()
            .expect("render test");
    }
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
                        "",
                        &cancelled
                    )
                    .is_err()
                );
                assert!(
                    compile(
                        r#"note("c3").csound("triangle")"#,
                        48_000,
                        120.0,
                        1,
                        4,
                        false,
                        "",
                        "",
                        &cancelled
                    )
                    .is_err(),
                    "native Csound scores must be refused unless the user opts in"
                );
                let mut plan = compile(
                    r#"note("c4 e4 g4 c5").s("sine").gain(0.2)"#,
                    48_000,
                    120.0,
                    1,
                    4,
                    false,
                    "",
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
    fn late_slow_and_legato_preserve_captured_midi_timing() {
        std::thread::Builder::new()
            .stack_size(QUERY_WORKER_STACK_BYTES)
            .spawn(|| {
                let cancelled = AtomicBool::new(false);
                let mut session = Session::with_config(
                    SessionConfig::default()
                        .with_sample_rate(48_000)
                        .with_cps(1.0)
                        .with_horizon(2.0),
                )
                .expect("test session");
                let source = csound_midi_source(
                    "tri",
                    &[MidiTargetNote {
                        start: 24_000,
                        length: 24_000,
                        pitch: 60,
                        velocity: 0.75,
                    }],
                    2,
                    96_000,
                )
                .expect("generated target source")
                .replace(".csoundm(\"tri\")", ".s(\"sine\")");
                session
                    .evaluate_no_fallback_cancellable(&source, &cancelled)
                    .expect("timed pattern");
                session.retime(0.0, 1.0, 0.0);
                let events = session
                    .schedule_audio_through(0.0, 2.0, 48_000)
                    .expect("scheduled pattern");
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].target_frame, 24_000);
                assert!((events[0].duration_secs - 0.5).abs() < 0.001);
            })
            .expect("compiler worker")
            .join()
            .expect("timing check");
    }

    #[test]
    fn csound_target_notes_keep_the_loaded_orchestra_across_edits() {
        std::thread::Builder::new()
            .stack_size(QUERY_WORKER_STACK_BYTES)
            .spawn(|| {
                let cancelled = AtomicBool::new(false);
                let mut access = ScoreImportAccess::denied();
                access.permit_csound();
                let mut session = Session::with_config(
                    SessionConfig::default()
                        .with_sample_rate(48_000)
                        .with_cps(1.0)
                        .with_horizon(2.0)
                        .with_score_import_access(access),
                )
                .expect("test session");
                let orchestra = session.evaluate_no_fallback_cancellable(
                    "loadCSound`instr tri\\na1 poscil p5, cpsmidinn(p4)\\nouts a1, a1\\nendin`",
                    &cancelled,
                );
                if orchestra
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains("Csound 6 is not available"))
                {
                    eprintln!("Skipping native Csound check: the Csound runtime is not installed.");
                    return;
                }
                orchestra.expect("orchestra definition");
                let source = csound_midi_source(
                    "tri",
                    &[MidiTargetNote {
                        start: 24_000,
                        length: 24_000,
                        pitch: 60,
                        velocity: 0.75,
                    }],
                    2,
                    96_000,
                )
                .expect("target source");
                session
                    .evaluate_no_fallback_cancellable(&source, &cancelled)
                    .expect("target score should reuse the orchestra");
                session.retime(0.0, 1.0, 0.0);
                let events = session
                    .schedule_audio_through(0.0, 2.0, 48_000)
                    .expect("scheduled Csound MIDI");
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].target_frame, 24_000);
                assert!((events[0].duration_secs - 0.5).abs() < 0.001);
            })
            .expect("compiler worker")
            .join()
            .expect("Csound snapshot check");
    }

    #[cfg(not(windows))]
    #[test]
    fn vsti_scores_report_the_missing_windows_host_helper() {
        std::thread::Builder::new()
            .stack_size(QUERY_WORKER_STACK_BYTES)
            .spawn(|| {
                let cancelled = AtomicBool::new(false);
                let error = compile(
                    r#"note("c2 e2 g2").vsti("Example instrument")"#,
                    48_000,
                    120.0,
                    1,
                    4,
                    false,
                    "",
                    "",
                    &cancelled,
                )
                .err()
                .expect("the Windows-hosted plugin bundle is unavailable on Linux");
                assert!(error.contains("Windows x64"));
            })
            .expect("compiler worker")
            .join()
            .expect("VST3 score check");
    }

    #[test]
    fn csound_permission_is_off_by_default() {
        assert!(!crate::params::RustelParams::default().allow_csound.value());
    }

    #[test]
    fn csound_midi_target_requires_the_csound_opt_in() {
        let cancelled = AtomicBool::new(false);
        let error = match compile(
            "// MIDI target: csound \"triangle\"\nnote(\"c4\").vsti(\"MIDI generator\")",
            48_000,
            120.0,
            1,
            4,
            false,
            "",
            "",
            &cancelled,
        ) {
            Err(error) => error,
            Ok(_) => panic!("Csound target must require explicit permission"),
        };
        assert!(error.contains("Enable Csound support"));
    }
}
