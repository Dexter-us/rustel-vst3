use crate::vst_catalog::{VstCatalog, VstSort};
use crate::{compiler, compiler::PreparedPlan, params::RustelParams};
use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::*;
use parking_lot::Mutex;
use rustel_engine::{
    QUERY_WORKER_STACK_BYTES,
    audio::{
        AudioBackend, Envelope, OnsetEvent, ScalarBackend, SynthSource, Waveform, zzfx::ZzfxParams,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiInputEvent {
    NoteOn {
        timing: u32,
        channel: u8,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        timing: u32,
        channel: u8,
        note: u8,
    },
}

impl MidiInputEvent {
    fn timing(self) -> u32 {
        match self {
            Self::NoteOn { timing, .. } | Self::NoteOff { timing, .. } => timing,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HostTransport {
    pub playing: bool,
    pub tempo: Option<f64>,
    pub position_beats: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct VstPluginEntry {
    pub key: String,
    pub name: String,
    pub vendor: String,
    pub categories: String,
    pub instrument: bool,
    pub display_label: String,
    pub midi_source: bool,
    pub order: u64,
}

#[derive(Clone, Debug)]
pub struct VstParameterChoice {
    pub key: String,
    pub name: String,
    pub group: String,
    pub default_value: f64,
    pub default_shown: String,
}

#[derive(Clone, Debug, Default)]
pub struct VstParameterState {
    pub plugin: String,
    pub loading: bool,
    pub ready: bool,
    pub parameters: Vec<VstParameterChoice>,
    pub error: Option<String>,
}

pub struct Shared {
    pub status: Mutex<String>,
    pub vst_plugins: Mutex<Vec<VstPluginEntry>>,
    pub vst_plugins_message: Mutex<String>,
    pub vst_sort: Mutex<VstSort>,
    pub vst_parameter_request: Mutex<Option<String>>,
    pub vst_parameter_state: Mutex<VstParameterState>,
    pub peak: AtomicF32,
    pub bpm: AtomicU64,
    pub beat: AtomicU64,
    pub refused: AtomicU64,
    pub requested: AtomicU64,
    pub sample_rate: AtomicU32,
    pub scan_vst_plugins: AtomicBool,
    shutdown: AtomicBool,
    ready: ArrayQueue<Box<PreparedPlan>>,
    retired: ArrayQueue<Box<PreparedPlan>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            status: Mutex::new("Waiting for the DAW to initialize audio.".into()),
            vst_plugins: Mutex::new(Vec::new()),
            vst_plugins_message: Mutex::new("Loading saved VST3 scan results on startup…".into()),
            vst_sort: Mutex::new(VstSort::Name),
            vst_parameter_request: Mutex::new(None),
            vst_parameter_state: Mutex::new(VstParameterState::default()),
            peak: AtomicF32::new(0.0),
            bpm: AtomicU64::new(120.0_f64.to_bits()),
            beat: AtomicU64::new(0.0_f64.to_bits()),
            refused: AtomicU64::new(0),
            requested: AtomicU64::new(0),
            sample_rate: AtomicU32::new(0),
            // Restore the saved scan cache and refresh changed bundles as soon
            // as the plugin worker starts; users should not need to rescan
            // manually every time the DAW loads Rustel.
            scan_vst_plugins: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            ready: ArrayQueue::new(1),
            retired: ArrayQueue::new(8),
        }
    }

    pub fn request_vst_parameters(&self, plugin: &str) {
        let mut state = self.vst_parameter_state.lock();
        if state.plugin == plugin && (state.loading || state.ready) {
            return;
        }
        *state = VstParameterState {
            plugin: plugin.to_owned(),
            loading: true,
            ..VstParameterState::default()
        };
        *self.vst_parameter_request.lock() = Some(plugin.to_owned());
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    params: Arc<RustelParams>,
    worker: Option<JoinHandle<()>>,
    plan: Option<Box<PreparedPlan>>,
    midi_backend: Option<ScalarBackend>,
    hosted_midi_backend: Option<ScalarBackend>,
    deferred_retirement: Option<Box<PreparedPlan>>,
    frame: u64,
    sample_rate: f64,
    expected_beat: Option<f64>,
    free_beat: f64,
    was_playing: bool,
    last_midi_source_frame: Option<usize>,
    gate: f32,
    midi_gate: f32,
    was_enabled: bool,
}

impl Engine {
    pub fn new(params: Arc<RustelParams>) -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            params,
            worker: None,
            plan: None,
            midi_backend: None,
            hosted_midi_backend: None,
            deferred_retirement: None,
            frame: 0,
            sample_rate: 48_000.0,
            expected_beat: None,
            free_beat: 0.0,
            was_playing: false,
            last_midi_source_frame: None,
            gate: 0.0,
            midi_gate: 0.0,
            was_enabled: true,
        }
    }

    pub fn initialize(&mut self, sample_rate: f32) -> bool {
        if !sample_rate.is_finite() || !(8_000.0..=192_000.0).contains(&sample_rate) {
            *self.shared.status.lock() = "Supported sample rates: 8–192 kHz.".into();
            return false;
        }
        self.sample_rate = f64::from(sample_rate);
        match ScalarBackend::prepared(sample_rate.round() as u32, 64) {
            Ok(backend) => {
                self.midi_backend = Some(backend);
                self.hosted_midi_backend =
                    match prepare_hosted_midi_backend(sample_rate.round() as u32) {
                        Ok(backend) => Some(backend),
                        Err(error) => {
                            *self.shared.status.lock() =
                                format!("Could not prepare hosted MIDI instrument: {error}");
                            return false;
                        }
                    };
            }
            Err(error) => {
                *self.shared.status.lock() = format!("Could not prepare MIDI instrument: {error}");
                return false;
            }
        }
        self.shared
            .sample_rate
            .store(sample_rate.round() as u32, Ordering::Release);
        if self
            .plan
            .as_ref()
            .is_some_and(|plan| plan.sample_rate == sample_rate.round() as u32)
        {
            self.reset();
            return true;
        }
        *self.shared.status.lock() = "Preparing score…".into();
        if self.worker.is_none() {
            let shared = self.shared.clone();
            let params = self.params.clone();
            match thread::Builder::new()
                .name("rustel-score".into())
                .stack_size(QUERY_WORKER_STACK_BYTES)
                .spawn(move || worker(shared, params))
            {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    *self.shared.status.lock() = format!("Could not start score worker: {error}");
                    return false;
                }
            }
        }
        // Initialization is not the real-time callback. Prime the first score
        // here so a DAW's immediate offline bounce need not race a worker.
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Some(plan) = self.shared.ready.pop() {
                if plan.sample_rate == sample_rate.round() as u32 {
                    self.plan = Some(plan);
                    self.reset();
                    return true;
                }
            }
            if self.shared.status.lock().starts_with("Score refused.") {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        // Keep the editor accessible for correcting a refused saved score.
        *self.shared.status.lock() =
            "No prepared score. Open the editor, correct the score, and press Evaluate.".into();
        true
    }

    pub fn reset(&mut self) {
        if let Some(plan) = &mut self.plan {
            plan.backend.reset_at(0);
        }
        self.frame = 0;
        self.expected_beat = None;
        self.free_beat = 0.0;
        self.was_playing = false;
        self.last_midi_source_frame = None;
        self.gate = 0.0;
        self.midi_gate = 0.0;
        self.was_enabled = self.params.enabled.value();
        if let Some(backend) = &mut self.midi_backend {
            backend.reset_at(0);
        }
        if let Some(backend) = &mut self.hosted_midi_backend {
            backend.reset_at(0);
        }
    }

    fn receive_plan(&mut self) {
        if let Some(retired) = self.deferred_retirement.take() {
            if let Err(retired) = self.shared.retired.push(retired) {
                self.deferred_retirement = Some(retired);
                return;
            }
        }
        if let Some(mut plan) = self.shared.ready.pop() {
            plan.backend.reset_at(self.frame);
            if let Some(backend) = &mut self.hosted_midi_backend {
                backend.reset_at(self.frame);
            }
            if let Some(old) = self.plan.replace(plan) {
                if let Err(old) = self.shared.retired.push(old) {
                    self.deferred_retirement = Some(old);
                }
            }
            self.gate = 0.0;
            self.last_midi_source_frame = None;
        }
    }

    pub fn process(
        &mut self,
        buffer: &mut Buffer,
        transport: &HostTransport,
        next_midi_event: &mut impl FnMut() -> Option<MidiInputEvent>,
    ) {
        self.receive_plan();
        let bpm = transport
            .tempo
            .filter(|value| value.is_finite() && (1.0..=999.0).contains(value))
            .unwrap_or(120.0);
        self.shared.bpm.store(bpm.to_bits(), Ordering::Relaxed);
        let beats_per_frame = bpm / (60.0 * self.sample_rate);
        let enabled = self.params.enabled.value();
        let follow = self.params.follow_host.value();
        let playing = enabled && (!follow || transport.playing);
        if !enabled && self.was_enabled {
            if let Some(backend) = &mut self.midi_backend {
                backend.reset_at(self.frame);
            }
        }
        self.was_enabled = enabled;
        let start_beat = if follow {
            transport
                .position_beats
                .filter(|value| value.is_finite())
                .unwrap_or(self.free_beat)
        } else {
            self.free_beat
        };
        self.shared
            .beat
            .store(start_beat.to_bits(), Ordering::Relaxed);
        let seek = self
            .expected_beat
            .is_some_and(|expected| (start_beat - expected).abs() > beats_per_frame * 2.0);
        if playing && (!self.was_playing || seek) {
            if let Some(plan) = &mut self.plan {
                plan.backend.reset_at(self.frame);
            }
            if let Some(backend) = &mut self.hosted_midi_backend {
                backend.reset_at(self.frame);
            }
            self.last_midi_source_frame = None;
            self.gate = 0.0;
        }
        if !playing && self.was_playing {
            if let Some(backend) = &mut self.hosted_midi_backend {
                backend.reset_at(self.frame);
            }
        }
        let count = buffer.samples();
        let channels = buffer.as_slice();
        let mut peak = 0.0_f32;
        let mut offset = 0;
        let mut next_event = next_midi_event();
        let midi_style = self
            .plan
            .as_ref()
            .and_then(|plan| {
                plan.notes.iter().find(|note| {
                    note.event.sample.is_none()
                        && note.event.wavetable.is_none()
                        && note.event.synth.is_none()
                        && note.event.controls.kabel.is_none()
                })
            })
            .map(|note| (note.event.controls.waveform, note.event.controls.envelope))
            .unwrap_or((Waveform::Sine, Envelope::default()));
        let hosted_midi_synth = self
            .plan
            .as_ref()
            .map(|plan| plan.hosted_midi_synth)
            .unwrap_or_default();
        while offset < count {
            let frames = (count - offset).min(64);
            let mut pcm = [0.0_f32; 128];
            let mut midi_pcm = [0.0_f32; 128];
            let mut hosted_midi_pcm = [0.0_f32; 128];
            let block_end = offset + frames;
            while let Some(event) = next_event {
                if event.timing() as usize > block_end {
                    break;
                }
                next_event = next_midi_event();
                let event_frame = self.frame + u64::from(event.timing());
                match event {
                    MidiInputEvent::NoteOn {
                        channel,
                        note,
                        velocity,
                        ..
                    } if enabled => {
                        let velocity = if velocity.is_finite() {
                            velocity.clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        if velocity > 0.0 {
                            if !queue_midi_note(
                                &mut self.midi_backend,
                                channel,
                                note,
                                velocity,
                                event_frame,
                                midi_style,
                                None,
                            ) {
                                self.shared.refused.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    MidiInputEvent::NoteOn { channel, note, .. }
                    | MidiInputEvent::NoteOff { channel, note, .. } => {
                        if !queue_midi_release(&mut self.midi_backend, channel, note, event_frame) {
                            self.shared.refused.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            if let Some(plan) = &mut self.plan {
                if plan.sample_rate == self.sample_rate.round() as u32 {
                    if let Some(phrase) = &plan.rendered_phrase {
                        // A rendered Csound phrase is never rate-shifted to
                        // follow a tempo change. The worker prepares a new
                        // phrase for the new tempo; until then this path is
                        // silent rather than changing the instrument's pitch.
                        let csound_allowed =
                            !plan.requires_csound || self.params.allow_csound.value();
                        let tempo_matches =
                            csound_allowed && (plan.rendered_bpm - bpm).abs() < 0.01;
                        if playing && tempo_matches && plan.period_beats > 0.0 {
                            let phrase_frames = phrase.len() / 2;
                            for index in 0..frames {
                                let beat = start_beat + (offset + index) as f64 * beats_per_frame;
                                let source_frame =
                                    rendered_frame_at(beat, plan.period_beats, phrase_frames);
                                pcm[index * 2] = phrase[source_frame * 2];
                                pcm[index * 2 + 1] = phrase[source_frame * 2 + 1];
                                if should_emit_midi_events(
                                    &mut self.last_midi_source_frame,
                                    source_frame,
                                ) {
                                    let output_frame = source_frame as i64;
                                    let first = plan
                                        .midi_output_events
                                        .partition_point(|event| event.frame < output_frame);
                                    for event in plan.midi_output_events[first..]
                                        .iter()
                                        .take_while(|event| event.frame == output_frame)
                                    {
                                        let at = self.frame + (offset + index) as u64;
                                        let queued = if event.is_note_on && event.velocity > 0.0 {
                                            queue_midi_note(
                                                &mut self.hosted_midi_backend,
                                                event.channel,
                                                event.note,
                                                event.velocity,
                                                at,
                                                midi_style,
                                                Some(hosted_midi_synth),
                                            )
                                        } else {
                                            queue_midi_release(
                                                &mut self.hosted_midi_backend,
                                                event.channel,
                                                event.note,
                                                at,
                                            )
                                        };
                                        if !queued {
                                            self.shared.refused.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    self.last_midi_source_frame = Some(source_frame);
                                }
                            }
                        }
                    } else {
                        if playing {
                            let begin = start_beat + offset as f64 * beats_per_frame;
                            admit(
                                plan,
                                begin,
                                begin + frames as f64 * beats_per_frame,
                                beats_per_frame,
                                self.frame + offset as u64,
                                bpm,
                                &self.shared.refused,
                            );
                        }
                        plan.backend.process_block(&mut pcm[..frames * 2], frames);
                    }
                }
            }
            if let Some(backend) = &mut self.midi_backend {
                backend.process_block(&mut midi_pcm[..frames * 2], frames);
            }
            if let Some(backend) = &mut self.hosted_midi_backend {
                backend.process_block(&mut hosted_midi_pcm[..frames * 2], frames);
            }
            for index in 0..frames {
                let target = if playing { 1.0 } else { 0.0 };
                let step = (self.sample_rate * 0.005).recip() as f32;
                self.gate += (target - self.gate).clamp(-step, step);
                let midi_target = if enabled { 1.0 } else { 0.0 };
                self.midi_gate += (midi_target - self.midi_gate).clamp(-step, step);
                let gain = self.params.gain.smoothed.next();
                for channel in 0..channels.len() {
                    let value = pcm[index * 2 + channel.min(1)] * (gain * self.gate)
                        + midi_pcm[index * 2 + channel.min(1)] * (gain * self.midi_gate);
                    let value =
                        value + hosted_midi_pcm[index * 2 + channel.min(1)] * (gain * self.gate);
                    // Contain a failed DSP voice instead of passing NaN to a DAW.
                    let value = if value.is_finite() { value } else { 0.0 };
                    channels[channel][offset + index] = value;
                    peak = peak.max(value.abs());
                }
            }
            offset += frames;
        }
        let end = start_beat + count as f64 * beats_per_frame;
        self.free_beat = if playing { end } else { start_beat };
        self.expected_beat = playing.then_some(end);
        self.was_playing = playing;
        self.frame = self.frame.saturating_add(count as u64);
        self.shared.peak.store(peak, Ordering::Relaxed);
    }
}

fn midi_group(channel: u8, note: u8) -> f32 {
    f32::from(u16::from(channel) * 128 + u16::from(note) + 1)
}

fn prepare_hosted_midi_backend(sample_rate: u32) -> Result<ScalarBackend, String> {
    let synths = [
        compiler::RustelMidiSynth::Sine,
        compiler::RustelMidiSynth::Triangle,
        compiler::RustelMidiSynth::Square,
        compiler::RustelMidiSynth::Sawtooth,
        compiler::RustelMidiSynth::Supersaw,
        compiler::RustelMidiSynth::Pulse,
        compiler::RustelMidiSynth::Sbd,
        compiler::RustelMidiSynth::WhiteNoise,
        compiler::RustelMidiSynth::PinkNoise,
        compiler::RustelMidiSynth::BrownNoise,
        compiler::RustelMidiSynth::Crackle,
        compiler::RustelMidiSynth::ByteBeat,
        compiler::RustelMidiSynth::ZzFx,
        compiler::RustelMidiSynth::One,
    ];
    let mut templates = Vec::with_capacity(64);
    for index in 0..64 {
        let synth = synths[index % synths.len()];
        let mut event = OnsetEvent::new(0, 440.0, 0.4, 1.0);
        apply_rustel_midi_synth(&mut event, synth);
        templates.push(event);
    }
    let mut backend = ScalarBackend::prepared(sample_rate, 64)?;
    backend.prepare_finite_playback(&templates)?;
    Ok(backend)
}

fn apply_rustel_midi_synth(event: &mut OnsetEvent, synth: compiler::RustelMidiSynth) {
    match synth {
        compiler::RustelMidiSynth::Sine => event.controls.waveform = Waveform::Sine,
        compiler::RustelMidiSynth::Triangle => event.controls.waveform = Waveform::Triangle,
        compiler::RustelMidiSynth::Square => event.controls.waveform = Waveform::Square,
        compiler::RustelMidiSynth::Sawtooth => event.controls.waveform = Waveform::Sawtooth,
        compiler::RustelMidiSynth::Supersaw => {
            event.synth = Some(SynthSource::Supersaw {
                voices: 7.0,
                freqspread: 0.18,
                panspread: 0.6,
            });
        }
        compiler::RustelMidiSynth::Pulse => {
            event.synth = Some(SynthSource::Pulse {
                pulsewidth: 0.5,
                width_lfo: None,
            });
        }
        compiler::RustelMidiSynth::Sbd => {
            event.synth = Some(SynthSource::Sbd {
                decay_secs: 0.5,
                pdecay_secs: 0.5,
                penv_semitones: 36.0,
                stop_secs: 0.5,
            });
        }
        compiler::RustelMidiSynth::WhiteNoise => {
            event.synth = Some(SynthSource::Noise {
                kind: 0,
                density: 0.02,
            });
        }
        compiler::RustelMidiSynth::PinkNoise => {
            event.synth = Some(SynthSource::Noise {
                kind: 1,
                density: 0.02,
            });
        }
        compiler::RustelMidiSynth::BrownNoise => {
            event.synth = Some(SynthSource::Noise {
                kind: 2,
                density: 0.02,
            });
        }
        compiler::RustelMidiSynth::Crackle => {
            event.synth = Some(SynthSource::Noise {
                kind: 3,
                density: 0.02,
            });
        }
        compiler::RustelMidiSynth::ByteBeat => {
            event.synth = Some(SynthSource::ByteBeat {
                expression: 0,
                program: None,
                start_offset: None,
            });
        }
        compiler::RustelMidiSynth::ZzFx => {
            event.synth = Some(SynthSource::ZzFx {
                params: ZzfxParams::default(),
            });
        }
        compiler::RustelMidiSynth::One => event.synth = Some(SynthSource::One),
    }
}

fn queue_midi_note(
    backend: &mut Option<ScalarBackend>,
    channel: u8,
    note: u8,
    velocity: f32,
    frame: u64,
    style: (Waveform, Envelope),
    synth: Option<compiler::RustelMidiSynth>,
) -> bool {
    let Some(backend) = backend else {
        return false;
    };
    let frequency = 440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0);
    let mut event = OnsetEvent::new(frame, frequency, velocity * 0.4, f32::INFINITY)
        .with_cut(Some(midi_group(channel, note)));
    event.controls.envelope = style.1;
    match synth {
        Some(synth) => apply_rustel_midi_synth(&mut event, synth),
        None => event.controls.waveform = style.0,
    }
    let event = event.into_prepared();
    backend.try_note_prepared(event)
}

fn queue_midi_release(
    backend: &mut Option<ScalarBackend>,
    channel: u8,
    note: u8,
    frame: u64,
) -> bool {
    let Some(backend) = backend else {
        return false;
    };
    let mut event =
        OnsetEvent::new(frame, 0.0, 0.0, 0.01).with_cut(Some(midi_group(channel, note)));
    event.controls.choke_only = true;
    backend.try_note_prepared(event.into_prepared())
}

#[cfg(test)]
mod midi_tests {
    use super::*;

    #[test]
    fn midi_note_on_sounds_and_note_off_releases_the_matching_note() {
        let mut backend = Some(ScalarBackend::prepared(48_000, 64).expect("prepared backend"));
        assert!(queue_midi_note(
            &mut backend,
            0,
            69,
            1.0,
            0,
            (Waveform::Sine, Envelope::default()),
            None,
        ));

        let mut pcm = [0.0_f32; 128];
        backend
            .as_mut()
            .expect("MIDI backend")
            .process_block(&mut pcm, 64);
        let held_peak = pcm
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(held_peak > 0.001, "MIDI note-on should produce audio");

        assert!(queue_midi_release(&mut backend, 0, 69, 64));
        let mut released_peak = held_peak;
        for _ in 0..12 {
            backend
                .as_mut()
                .expect("MIDI backend")
                .process_block(&mut pcm, 64);
            released_peak = pcm
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        }
        assert!(
            released_peak < held_peak * 0.01,
            "MIDI note-off should release the Rustel voice"
        );
    }

    #[test]
    fn midi_note_groups_are_unique_per_channel_and_key() {
        assert_ne!(midi_group(0, 60), midi_group(0, 61));
        assert_ne!(midi_group(0, 60), midi_group(1, 60));
    }

    #[test]
    fn midi_note_on_keeps_its_sample_offset() {
        let mut backend = Some(ScalarBackend::prepared(48_000, 64).expect("prepared backend"));
        assert!(queue_midi_note(
            &mut backend,
            0,
            69,
            1.0,
            32,
            (Waveform::Sine, Envelope::default()),
            None,
        ));
        let mut pcm = [0.0_f32; 128];
        backend
            .as_mut()
            .expect("MIDI backend")
            .process_block(&mut pcm, 64);
        assert!(pcm[..64].iter().all(|sample| *sample == 0.0));
        assert!(pcm[64..].iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn hosted_midi_can_drive_builtin_rustel_synth_sources() {
        for synth in [
            compiler::RustelMidiSynth::Triangle,
            compiler::RustelMidiSynth::Square,
            compiler::RustelMidiSynth::Sawtooth,
            compiler::RustelMidiSynth::Supersaw,
            compiler::RustelMidiSynth::Pulse,
            compiler::RustelMidiSynth::Sbd,
            compiler::RustelMidiSynth::WhiteNoise,
            compiler::RustelMidiSynth::PinkNoise,
            compiler::RustelMidiSynth::BrownNoise,
            compiler::RustelMidiSynth::ByteBeat,
            compiler::RustelMidiSynth::ZzFx,
            compiler::RustelMidiSynth::One,
        ] {
            let mut backend =
                Some(prepare_hosted_midi_backend(48_000).expect("prepared hosted backend"));
            assert!(
                queue_midi_note(
                    &mut backend,
                    0,
                    60,
                    0.8,
                    0,
                    (Waveform::Sine, Envelope::default()),
                    Some(synth),
                ),
                "{synth:?} should accept MIDI notes"
            );
            let mut pcm = [0.0_f32; 128];
            let mut peak = 0.0_f32;
            for _ in 0..8 {
                backend
                    .as_mut()
                    .expect("MIDI backend")
                    .process_block(&mut pcm, 64);
                peak = pcm.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
            }
            assert!(
                pcm.iter().all(|sample| sample.is_finite()),
                "{synth:?} should produce finite audio"
            );
            assert!(
                peak > 0.000001,
                "{synth:?} should produce audio, peak={peak}"
            );
        }
    }

    #[test]
    fn releasing_one_midi_key_does_not_stop_another() {
        let mut backend = Some(ScalarBackend::prepared(48_000, 64).expect("prepared backend"));
        let style = (Waveform::Sine, Envelope::default());
        assert!(queue_midi_note(&mut backend, 0, 60, 1.0, 0, style, None));
        assert!(queue_midi_note(&mut backend, 0, 67, 1.0, 0, style, None));

        let mut pcm = [0.0_f32; 128];
        backend
            .as_mut()
            .expect("MIDI backend")
            .process_block(&mut pcm, 64);
        assert!(queue_midi_release(&mut backend, 0, 60, 64));
        for _ in 0..12 {
            backend
                .as_mut()
                .expect("MIDI backend")
                .process_block(&mut pcm, 64);
        }
        assert!(
            pcm.iter().any(|sample| sample.abs() > 0.001),
            "the second note should remain held"
        );
    }
}

fn rendered_frame_at(beat: f64, period_beats: f64, phrase_frames: usize) -> usize {
    let local_beat = beat.rem_euclid(period_beats);
    (((local_beat / period_beats) * phrase_frames as f64).floor() as usize)
        .min(phrase_frames.saturating_sub(1))
}

fn should_emit_midi_events(last_frame: &mut Option<usize>, source_frame: usize) -> bool {
    if *last_frame == Some(source_frame) {
        return false;
    }
    *last_frame = Some(source_frame);
    true
}

fn admit(
    plan: &mut PreparedPlan,
    begin: f64,
    end: f64,
    beats_per_frame: f64,
    frame: u64,
    bpm: f64,
    refused: &AtomicU64,
) {
    let period = plan.period_beats;
    let mut base = (begin / period).floor() * period;
    while base < end {
        let local_begin = (begin - base).max(0.0);
        let index = plan.notes.partition_point(|note| note.beat < local_begin);
        for note in &plan.notes[index..] {
            let beat = base + note.beat;
            if beat >= end {
                break;
            }
            if beat < begin || beat < 0.0 {
                continue;
            }
            let mut event = note.event;
            event.onset_frame = frame + ((beat - begin) / beats_per_frame).ceil() as u64;
            event.onset_lead = 0.0;
            event.duration_secs = (note.duration_beats * 60.0 / bpm) as f32;
            if !plan.backend.try_note_prepared(event) {
                refused.fetch_add(1, Ordering::Relaxed);
            }
        }
        base += period;
    }
}

#[cfg(test)]
mod rendered_phrase_tests {
    use super::{rendered_frame_at, should_emit_midi_events};

    #[test]
    fn rendered_phrase_wraps_at_cycle_boundary_and_handles_seek() {
        assert_eq!(rendered_frame_at(0.0, 4.0, 400), 0);
        assert_eq!(rendered_frame_at(2.0, 4.0, 400), 200);
        assert_eq!(rendered_frame_at(4.0, 4.0, 400), 0);
        assert_eq!(rendered_frame_at(5.0, 4.0, 400), 100);
        assert_eq!(rendered_frame_at(-1.0, 4.0, 400), 300);
    }

    #[test]
    fn hosted_midi_events_only_fire_once_per_source_frame_and_repeat_after_wrap() {
        let mut last_frame = None;
        assert!(should_emit_midi_events(&mut last_frame, 10));
        assert!(!should_emit_midi_events(&mut last_frame, 10));
        assert!(should_emit_midi_events(&mut last_frame, 11));
        assert!(should_emit_midi_events(&mut last_frame, 0));
    }
}

#[cfg(test)]
mod startup_scan_tests {
    use super::Shared;
    use std::sync::atomic::Ordering;

    #[test]
    fn plugin_startup_requests_a_vst_scan() {
        let shared = Shared::new();
        assert!(shared.scan_vst_plugins.load(Ordering::Acquire));
        assert!(
            shared
                .vst_plugins_message
                .lock()
                .contains("Loading saved VST3 scan results")
        );
    }
}

fn worker(shared: Arc<Shared>, params: Arc<RustelParams>) {
    let mut last_request = u64::MAX;
    let mut last_key = (0, 0, 0, 0_u64, false, String::new(), String::new());
    let mut last_applied = String::new();
    let mut parameter_lookup = None;
    while !shared.shutdown.load(Ordering::Acquire) {
        while shared.retired.pop().is_some() {}
        if shared.scan_vst_plugins.swap(false, Ordering::AcqRel) {
            *shared.status.lock() = "Scanning standard VST3 folders and any added folders…".into();
            let folders = params.vst3_folders.lock().clone();
            match crate::portable_vst::configure(&folders, &shared.shutdown) {
                Ok(()) => {
                    let (catalog, catalog_warning) = match VstCatalog::load() {
                        Ok(catalog) => (catalog, None),
                        Err(error) => (VstCatalog::default(), Some(error)),
                    };
                    let plugins = rustel_runtime::vst::host()
                        .plugins()
                        .into_iter()
                        .enumerate()
                        .map(|(index, plugin)| {
                            let key = if plugin.class_id.is_empty() {
                                format!("{}\t{}\t{}", plugin.vendor, plugin.name, plugin.categories)
                            } else {
                                plugin.class_id
                            };
                            let preference = catalog.preference(&key, index as u64 * 1024);
                            VstPluginEntry {
                                key,
                                name: plugin.name,
                                vendor: plugin.vendor,
                                categories: plugin.categories,
                                instrument: plugin.instrument,
                                display_label: preference.label,
                                midi_source: preference.midi_source,
                                order: preference.order,
                            }
                        })
                        .collect::<Vec<_>>();
                    *shared.vst_sort.lock() = catalog.sort;
                    *shared.vst_plugins_message.lock() = catalog_warning.unwrap_or_else(|| {
                        if plugins.is_empty() {
                            "No VST3 plugins found in the standard or selected folders.".into()
                        } else {
                            String::new()
                        }
                    });
                    *shared.vst_plugins.lock() = plugins;
                    *shared.status.lock() = "VST3 plugin scan finished.".into();
                }
                Err(error) => {
                    shared.vst_plugins.lock().clear();
                    *shared.vst_plugins_message.lock() = error.clone();
                    *shared.status.lock() = error;
                }
            }
        }
        refresh_vst_parameter_lookup(&shared, &mut parameter_lookup);
        let request = shared.requested.load(Ordering::Acquire);
        let sample_rate = shared.sample_rate.load(Ordering::Acquire);
        let bpm = f64::from_bits(shared.bpm.load(Ordering::Relaxed));
        let cycles = params.cycles.value();
        let beats = params.beats_per_cycle.value();
        let allow_csound = params.allow_csound.value();
        let orchestra_root = params.csound_orchestra_root.lock().clone();
        let vst3_folders = params.vst3_folders.lock().clone();
        let key = (
            sample_rate,
            cycles,
            beats,
            (bpm * 100.0).round() as u64,
            allow_csound,
            orchestra_root.clone(),
            vst3_folders.clone(),
        );
        let applied = params.applied.lock().clone();
        if sample_rate != 0
            && (request != last_request || key != last_key || applied != last_applied)
        {
            let explicit = request != last_request && last_request != u64::MAX;
            let source = if explicit {
                params.draft.lock().clone()
            } else {
                applied.clone()
            };
            *shared.status.lock() = if allow_csound {
                "Preparing score and rendering trusted Csound phrase…".into()
            } else {
                "Preparing score…".into()
            };
            let result = compiler::compile(
                &source,
                sample_rate,
                bpm,
                cycles,
                beats,
                allow_csound,
                &orchestra_root,
                &vst3_folders,
                &shared.shutdown,
            );
            match result {
                Ok(plan) => {
                    let count = plan.notes.len();
                    let diagnostics = plan.diagnostics.clone();
                    let rendered_source = plan.rendered_source;
                    let rendered_bpm = plan.rendered_bpm;
                    // Only the worker publishes plans. Removing an obsolete,
                    // unconsumed plan here frees it away from the audio thread.
                    let _ = shared.ready.pop();
                    match shared.ready.push(plan) {
                        Ok(()) => {
                            *params.applied.lock() = source.clone();
                            last_applied = source;
                            *shared.status.lock() = if let Some(rendered_source) = rendered_source
                                && diagnostics.is_empty()
                            {
                                format!(
                                    "Ready · {rendered_source} · {cycles}-cycle repeat · {:.1} BPM",
                                    rendered_bpm
                                )
                            } else if let Some(rendered_source) = rendered_source {
                                format!(
                                    "Ready · {rendered_source} · {cycles}-cycle repeat\n{diagnostics}"
                                )
                            } else if diagnostics.is_empty() {
                                format!("Ready · {count} notes · {cycles}-cycle repeating phrase")
                            } else {
                                format!("Ready · {count} notes\n{diagnostics}")
                            };
                        }
                        Err(_) => {
                            *shared.status.lock() = "Could not publish the prepared score.".into();
                        }
                    }
                }
                Err(error) => {
                    last_applied = applied;
                    *shared.status.lock() =
                        format!("Score refused. Last good phrase kept.\n{error}");
                }
            }
            last_request = request;
            last_key = key;
        }
        thread::sleep(Duration::from_millis(20));
    }
    while shared.retired.pop().is_some() {}
}

fn refresh_vst_parameter_lookup(shared: &Shared, active_plugin: &mut Option<String>) {
    if let Some(plugin) = shared.vst_parameter_request.lock().take() {
        *active_plugin = Some(plugin);
    }
    let Some(name) = active_plugin.clone() else {
        return;
    };

    match rustel_runtime::vst::host().resolve(&name, false) {
        rustel_runtime::vst::Resolved::Ready(_, plugin) => {
            let parameters = plugin
                .params()
                .iter()
                .map(|parameter| VstParameterChoice {
                    key: parameter.key.clone(),
                    name: parameter.name.clone(),
                    group: parameter.group.clone(),
                    default_value: if parameter.default.is_finite() {
                        parameter.default.clamp(0.0, 1.0)
                    } else {
                        0.5
                    },
                    default_shown: parameter.default_shown(),
                })
                .collect();
            *shared.vst_parameter_state.lock() = VstParameterState {
                plugin: name,
                loading: false,
                ready: true,
                parameters,
                error: None,
            };
            *active_plugin = None;
        }
        rustel_runtime::vst::Resolved::Pending => {}
        rustel_runtime::vst::Resolved::Missing => {
            set_vst_parameter_error(shared, &name, "Plugin was not found in the VST3 scan.");
            *active_plugin = None;
        }
        rustel_runtime::vst::Resolved::Failed(error) => {
            set_vst_parameter_error(
                shared,
                &name,
                &format!("Could not inspect the plugin: {error}"),
            );
            *active_plugin = None;
        }
    }
}

fn set_vst_parameter_error(shared: &Shared, plugin: &str, error: &str) {
    *shared.vst_parameter_state.lock() = VstParameterState {
        plugin: plugin.to_owned(),
        loading: false,
        ready: true,
        parameters: Vec::new(),
        error: Some(error.to_owned()),
    };
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
