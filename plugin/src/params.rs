use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::sync::Arc;

pub const DEFAULT_SCORE: &str = r#"// One cycle = four quarter-note beats.
// Edit, then press Evaluate (Ctrl+Enter).
stack(
  note("c3 eb3 g3 bb3").s("triangle").gain(0.25),
  note("c2 ~ c2 g1").s("sine").gain(0.3)
)"#;

#[derive(Params)]
pub struct RustelParams {
    #[persist = "editor"]
    pub editor: Arc<EguiState>,
    #[persist = "draft"]
    pub draft: Arc<Mutex<String>>,
    #[persist = "applied-score"]
    pub applied: Arc<Mutex<String>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "enabled"]
    pub enabled: BoolParam,
    #[id = "follow-host"]
    pub follow_host: BoolParam,
    #[id = "cycles"]
    pub cycles: IntParam,
    #[id = "beats-per-cycle"]
    pub beats_per_cycle: IntParam,
    // Also marks a score edit as a host-visible state change.
    #[id = "score-revision"]
    pub score_revision: IntParam,
}

impl Default for RustelParams {
    fn default() -> Self {
        Self {
            editor: EguiState::from_size(900, 650),
            draft: Arc::new(Mutex::new(DEFAULT_SCORE.into())),
            applied: Arc::new(Mutex::new(DEFAULT_SCORE.into())),
            gain: FloatParam::new(
                "Output",
                util::db_to_gain(-6.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            enabled: BoolParam::new("Enabled", true),
            follow_host: BoolParam::new("Follow DAW", true),
            cycles: IntParam::new("Phrase cycles", 16, IntRange::Linear { min: 1, max: 64 }),
            beats_per_cycle: IntParam::new(
                "Beats per cycle",
                4,
                IntRange::Linear { min: 1, max: 16 },
            ),
            score_revision: IntParam::new(
                "Score revision",
                0,
                IntRange::Linear {
                    min: 0,
                    max: 1_000_000,
                },
            ),
        }
    }
}
