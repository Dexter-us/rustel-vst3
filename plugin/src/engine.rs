use crate::{compiler, compiler::PreparedPlan, params::RustelParams};
use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::*;
use parking_lot::Mutex;
use rustel_engine::{QUERY_WORKER_STACK_BYTES, audio::AudioBackend};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Shared {
    pub status: Mutex<String>,
    pub peak: AtomicF32,
    pub bpm: AtomicU64,
    pub beat: AtomicU64,
    pub refused: AtomicU64,
    pub requested: AtomicU64,
    pub sample_rate: AtomicU32,
    shutdown: AtomicBool,
    ready: ArrayQueue<Box<PreparedPlan>>,
    retired: ArrayQueue<Box<PreparedPlan>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            status: Mutex::new("Waiting for the DAW to initialize audio.".into()),
            peak: AtomicF32::new(0.0),
            bpm: AtomicU64::new(120.0_f64.to_bits()),
            beat: AtomicU64::new(0.0_f64.to_bits()),
            refused: AtomicU64::new(0),
            requested: AtomicU64::new(0),
            sample_rate: AtomicU32::new(0),
            shutdown: AtomicBool::new(false),
            ready: ArrayQueue::new(1),
            retired: ArrayQueue::new(8),
        }
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    params: Arc<RustelParams>,
    worker: Option<JoinHandle<()>>,
    plan: Option<Box<PreparedPlan>>,
    deferred_retirement: Option<Box<PreparedPlan>>,
    frame: u64,
    sample_rate: f64,
    expected_beat: Option<f64>,
    free_beat: f64,
    was_playing: bool,
    gate: f32,
}

impl Engine {
    pub fn new(params: Arc<RustelParams>) -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            params,
            worker: None,
            plan: None,
            deferred_retirement: None,
            frame: 0,
            sample_rate: 48_000.0,
            expected_beat: None,
            free_beat: 0.0,
            was_playing: false,
            gate: 0.0,
        }
    }

    pub fn initialize(&mut self, sample_rate: f32) -> bool {
        if !sample_rate.is_finite() || !(8_000.0..=192_000.0).contains(&sample_rate) {
            *self.shared.status.lock() = "Supported sample rates: 8–192 kHz.".into();
            return false;
        }
        self.sample_rate = f64::from(sample_rate);
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
        self.gate = 0.0;
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
            if let Some(old) = self.plan.replace(plan) {
                if let Err(old) = self.shared.retired.push(old) {
                    self.deferred_retirement = Some(old);
                }
            }
            self.gate = 0.0;
        }
    }

    pub fn process(&mut self, buffer: &mut Buffer, transport: &Transport) {
        self.receive_plan();
        let bpm = transport
            .tempo
            .filter(|value| value.is_finite() && (1.0..=999.0).contains(value))
            .unwrap_or(120.0);
        self.shared.bpm.store(bpm.to_bits(), Ordering::Relaxed);
        let beats_per_frame = bpm / (60.0 * self.sample_rate);
        let follow = self.params.follow_host.value();
        let playing = self.params.enabled.value() && (!follow || transport.playing);
        let start_beat = if follow {
            transport
                .pos_beats()
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
            self.gate = 0.0;
        }
        let count = buffer.samples();
        let channels = buffer.as_slice();
        let mut peak = 0.0_f32;
        let mut offset = 0;
        while offset < count {
            let frames = (count - offset).min(64);
            let mut pcm = [0.0_f32; 128];
            if let Some(plan) = &mut self.plan {
                if plan.sample_rate == self.sample_rate.round() as u32 {
                    if let Some(phrase) = &plan.rendered_phrase {
                        // A rendered Csound phrase is never rate-shifted to
                        // follow a tempo change. The worker prepares a new
                        // phrase for the new tempo; until then this path is
                        // silent rather than changing the instrument's pitch.
                        let tempo_matches = self.params.allow_csound.value()
                            && (plan.rendered_bpm - bpm).abs() < 0.01;
                        if playing && tempo_matches && plan.period_beats > 0.0 {
                            let phrase_frames = phrase.len() / 2;
                            for index in 0..frames {
                                let beat = start_beat + (offset + index) as f64 * beats_per_frame;
                                let source_frame =
                                    rendered_frame_at(beat, plan.period_beats, phrase_frames);
                                pcm[index * 2] = phrase[source_frame * 2];
                                pcm[index * 2 + 1] = phrase[source_frame * 2 + 1];
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
            for index in 0..frames {
                let target = if playing { 1.0 } else { 0.0 };
                let step = (self.sample_rate * 0.005).recip() as f32;
                self.gate += (target - self.gate).clamp(-step, step);
                let gain = self.params.gain.smoothed.next() * self.gate;
                for channel in 0..channels.len() {
                    let value = pcm[index * 2 + channel.min(1)] * gain;
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

fn rendered_frame_at(beat: f64, period_beats: f64, phrase_frames: usize) -> usize {
    let local_beat = beat.rem_euclid(period_beats);
    (((local_beat / period_beats) * phrase_frames as f64).floor() as usize)
        .min(phrase_frames.saturating_sub(1))
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
    use super::rendered_frame_at;

    #[test]
    fn rendered_phrase_wraps_at_cycle_boundary_and_handles_seek() {
        assert_eq!(rendered_frame_at(0.0, 4.0, 400), 0);
        assert_eq!(rendered_frame_at(2.0, 4.0, 400), 200);
        assert_eq!(rendered_frame_at(4.0, 4.0, 400), 0);
        assert_eq!(rendered_frame_at(5.0, 4.0, 400), 100);
        assert_eq!(rendered_frame_at(-1.0, 4.0, 400), 300);
    }
}

fn worker(shared: Arc<Shared>, params: Arc<RustelParams>) {
    let mut last_request = u64::MAX;
    let mut last_key = (0, 0, 0, 0_u64, false, String::new());
    let mut last_applied = String::new();
    while !shared.shutdown.load(Ordering::Acquire) {
        while shared.retired.pop().is_some() {}
        let request = shared.requested.load(Ordering::Acquire);
        let sample_rate = shared.sample_rate.load(Ordering::Acquire);
        let bpm = f64::from_bits(shared.bpm.load(Ordering::Relaxed));
        let cycles = params.cycles.value();
        let beats = params.beats_per_cycle.value();
        let allow_csound = params.allow_csound.value();
        let orchestra_root = params.csound_orchestra_root.lock().clone();
        let key = (
            sample_rate,
            cycles,
            beats,
            (bpm * 100.0).round() as u64,
            allow_csound,
            orchestra_root.clone(),
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
                &shared.shutdown,
            );
            match result {
                Ok(plan) => {
                    let count = plan.notes.len();
                    let diagnostics = plan.diagnostics.clone();
                    let rendered = plan.rendered_phrase.is_some();
                    let rendered_bpm = plan.rendered_bpm;
                    // Only the worker publishes plans. Removing an obsolete,
                    // unconsumed plan here frees it away from the audio thread.
                    let _ = shared.ready.pop();
                    match shared.ready.push(plan) {
                        Ok(()) => {
                            *params.applied.lock() = source.clone();
                            last_applied = source;
                            *shared.status.lock() = if rendered && diagnostics.is_empty() {
                                format!(
                                    "Ready · Csound-rendered phrase · {cycles}-cycle repeat · {:.1} BPM",
                                    rendered_bpm
                                )
                            } else if rendered {
                                format!(
                                    "Ready · Csound-rendered phrase · {cycles}-cycle repeat\n{diagnostics}"
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

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
