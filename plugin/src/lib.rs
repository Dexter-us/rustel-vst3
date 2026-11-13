//! Rustel's native engine as a DAW-owned stereo instrument.
mod compiler;
mod editor;
mod engine;
mod params;

use nih_plug::prelude::*;
use std::sync::Arc;

use engine::Engine;
use params::RustelParams;

pub struct RustelPlugin {
    params: Arc<RustelParams>,
    engine: Engine,
}

impl Default for RustelPlugin {
    fn default() -> Self {
        let params = Arc::new(RustelParams::default());
        Self {
            engine: Engine::new(params.clone()),
            params,
        }
    }
}

impl Plugin for RustelPlugin {
    const NAME: &'static str = "Rustel";
    const VENDOR: &'static str = "Dexter U.S.";
    const URL: &'static str = "https://github.com/Dexter-us/rustel-vst3";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.engine.shared.clone())
    }

    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        config: &BufferConfig,
        _: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine.initialize(config.sample_rate)
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.engine.process(buffer, context.transport());
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for RustelPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"RustelVST3Win001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_vst3!(RustelPlugin);

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn reports_requested_manufacturer() {
        assert_eq!(RustelPlugin::VENDOR, "Dexter U.S.");
    }
}
