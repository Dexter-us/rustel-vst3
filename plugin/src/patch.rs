use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MAX_PATCH_NODES: usize = 64;
pub const MAX_PATCH_CONNECTIONS: usize = 128;
pub const MAX_PATCH_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_PATCH_STATE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    Midi,
    Audio,
}

impl PortKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Midi => "MIDI",
            Self::Audio => "Audio",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Midi => [225, 173, 80],
            Self::Audio => [89, 190, 173],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionSignal {
    Midi,
    Audio,
}

impl ExpressionSignal {
    pub fn port_kind(self) -> PortKind {
        match self {
            Self::Midi => PortKind::Midi,
            Self::Audio => PortKind::Audio,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    RustelSynth,
    Sample,
    Wavetable,
    Graph,
    Csound,
    Vsti,
}

impl Default for InstrumentKind {
    fn default() -> Self {
        Self::RustelSynth
    }
}

impl InstrumentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RustelSynth => "Rustel synth",
            Self::Sample => "Sample",
            Self::Wavetable => "Wavetable",
            Self::Graph => "KabelSalat graph",
            Self::Csound => "Csound instrument",
            Self::Vsti => "VST3 instrument",
        }
    }

    fn midi_target(&self, value: &str) -> Result<String, String> {
        match self {
            Self::RustelSynth => Ok(format!("rustel {}", js_string_literal(value))),
            Self::Sample => Ok(format!("sample {}", js_string_literal(value))),
            Self::Wavetable => Ok(format!("wavetable {}", js_string_literal(value))),
            Self::Graph => {
                if value.trim().is_empty() {
                    Err("Enter a KabelSalat graph expression.".into())
                } else {
                    Ok(format!("graph {}", value.trim()))
                }
            }
            Self::Csound => Ok(format!("csound {}", js_string_literal(value))),
            Self::Vsti => Ok(format!("vsti {}", js_string_literal(value))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "node_type", rename_all = "snake_case")]
pub enum PatchNodeKind {
    NotePattern {
        expression: String,
    },
    MidiVst {
        plugin: String,
        is_effect: bool,
    },
    Instrument {
        instrument: InstrumentKind,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orc_file: Option<String>,
    },
    Expression {
        signal: ExpressionSignal,
        suffix: String,
    },
    VstEffect {
        plugin: String,
        parameters: String,
    },
    Output,
}

impl PatchNodeKind {
    pub fn title(&self) -> &'static str {
        match self {
            Self::NotePattern { .. } => "Note pattern",
            Self::MidiVst { .. } => "MIDI VST3 source",
            Self::Instrument { instrument, .. } => instrument.label(),
            Self::Expression {
                signal: ExpressionSignal::Midi,
                ..
            } => "Pattern expression",
            Self::Expression {
                signal: ExpressionSignal::Audio,
                ..
            } => "Audio expression",
            Self::VstEffect { .. } => "VST3 effect",
            Self::Output => "Audio output",
        }
    }

    pub fn input_kind(&self) -> Option<PortKind> {
        match self {
            Self::NotePattern { .. } => None,
            Self::MidiVst { .. } => Some(PortKind::Midi),
            Self::Instrument { .. } => Some(PortKind::Midi),
            Self::Expression { signal, .. } => Some(signal.port_kind()),
            Self::VstEffect { .. } => Some(PortKind::Audio),
            Self::Output => Some(PortKind::Audio),
        }
    }

    pub fn output_kind(&self) -> Option<PortKind> {
        match self {
            Self::NotePattern { .. } | Self::MidiVst { .. } => Some(PortKind::Midi),
            Self::Instrument { .. } | Self::VstEffect { .. } => Some(PortKind::Audio),
            Self::Output => None,
            Self::Expression { signal, .. } => Some(signal.port_kind()),
        }
    }

    fn requires_input(&self) -> bool {
        !matches!(self, Self::NotePattern { .. } | Self::Output)
    }

    fn allows_multiple_inputs(&self) -> bool {
        matches!(self, Self::Output)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PatchNode {
    pub id: u64,
    pub position: [f32; 2],
    pub kind: PatchNodeKind,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PatchConnection {
    pub source: u64,
    pub target: u64,
    pub signal: PortKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PatchGraph {
    version: u32,
    next_id: u64,
    pub nodes: Vec<PatchNode>,
    pub connections: Vec<PatchConnection>,
}

impl Default for PatchGraph {
    fn default() -> Self {
        let mut graph = Self {
            version: 1,
            next_id: 1,
            nodes: Vec::new(),
            connections: Vec::new(),
        };
        let pattern = graph
            .add_node(
                PatchNodeKind::NotePattern {
                    expression: "note(\"c3 e3 g3 b3\")".into(),
                },
                [14.0, 24.0],
            )
            .expect("default patch node");
        let synth = graph
            .add_node(
                PatchNodeKind::Instrument {
                    instrument: InstrumentKind::RustelSynth,
                    value: "sine".into(),
                    orc_file: None,
                },
                [240.0, 24.0],
            )
            .expect("default patch node");
        let output = graph
            .add_node(PatchNodeKind::Output, [466.0, 24.0])
            .expect("default patch node");
        graph.connect(pattern, synth).expect("default MIDI wire");
        graph.connect(synth, output).expect("default audio wire");
        graph
    }
}

impl PatchGraph {
    pub fn add_node(&mut self, kind: PatchNodeKind, position: [f32; 2]) -> Result<u64, String> {
        if self.nodes.len() >= MAX_PATCH_NODES {
            return Err(format!(
                "A patch can contain up to {MAX_PATCH_NODES} nodes."
            ));
        }
        let mut id = self.next_id.max(1);
        while self.nodes.iter().any(|node| node.id == id) {
            id = id
                .checked_add(1)
                .ok_or_else(|| "No unused patch node IDs remain.".to_owned())?;
        }
        self.next_id = id.saturating_add(1);
        self.nodes.push(PatchNode {
            id,
            position: [finite_position(position[0]), finite_position(position[1])],
            kind,
        });
        Ok(id)
    }

    pub fn remove_node(&mut self, id: u64) -> bool {
        let original_len = self.nodes.len();
        self.nodes.retain(|node| node.id != id);
        if self.nodes.len() == original_len {
            return false;
        }
        self.connections
            .retain(|connection| connection.source != id && connection.target != id);
        true
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.connections.clear();
        self.next_id = 1;
    }

    pub fn connect(&mut self, source_id: u64, target_id: u64) -> Result<(), String> {
        if source_id == target_id {
            return Err("A node cannot connect to itself.".into());
        }
        let source = self
            .nodes
            .iter()
            .find(|node| node.id == source_id)
            .ok_or_else(|| "The cable source node no longer exists.".to_owned())?;
        let target = self
            .nodes
            .iter()
            .find(|node| node.id == target_id)
            .ok_or_else(|| "The cable destination node no longer exists.".to_owned())?;
        let signal = source
            .kind
            .output_kind()
            .ok_or_else(|| format!("{} has no output port.", source.kind.title()))?;
        let target_signal = target
            .kind
            .input_kind()
            .ok_or_else(|| format!("{} has no input port.", target.kind.title()))?;
        if signal != target_signal {
            return Err(format!(
                "{} output cannot connect to {} input. MIDI and audio cables are different.",
                signal.label(),
                target_signal.label()
            ));
        }
        if self
            .connections
            .iter()
            .any(|wire| wire.source == source_id && wire.target == target_id)
        {
            return Err("Those nodes are already connected.".into());
        }
        if !target.kind.allows_multiple_inputs()
            && self.connections.iter().any(|wire| wire.target == target_id)
        {
            return Err(
                "That input already has a connection. Remove it before adding another.".into(),
            );
        }
        if self.connections.len() >= MAX_PATCH_CONNECTIONS {
            return Err(format!(
                "A patch can contain up to {MAX_PATCH_CONNECTIONS} cables."
            ));
        }
        if self.can_reach(target_id, source_id) {
            return Err(
                "This cable would create a loop. Patch connections must flow forward.".into(),
            );
        }
        self.connections.push(PatchConnection {
            source: source_id,
            target: target_id,
            signal,
        });
        Ok(())
    }

    pub fn disconnect(&mut self, source_id: u64, target_id: u64) -> bool {
        let original_len = self.connections.len();
        self.connections
            .retain(|wire| !(wire.source == source_id && wire.target == target_id));
        self.connections.len() != original_len
    }

    pub fn reconcile_connections(&mut self) -> usize {
        let original_len = self.connections.len();
        let nodes = &self.nodes;
        self.connections.retain(|wire| {
            let source = nodes.iter().find(|node| node.id == wire.source);
            let target = nodes.iter().find(|node| node.id == wire.target);
            matches!(
                (source.and_then(|node| node.kind.output_kind()),
                 target.and_then(|node| node.kind.input_kind())),
                (Some(source_kind), Some(target_kind))
                    if source_kind == target_kind && wire.signal == source_kind
            )
        });
        original_len - self.connections.len()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "This patch uses an unsupported saved format (version {}).",
                self.version
            ));
        }
        if self.nodes.is_empty() || self.nodes.len() > MAX_PATCH_NODES {
            return Err(format!(
                "A patch needs nodes and may contain at most {MAX_PATCH_NODES}."
            ));
        }
        if self.connections.len() > MAX_PATCH_CONNECTIONS {
            return Err(format!(
                "A patch may contain at most {MAX_PATCH_CONNECTIONS} cables."
            ));
        }
        let mut ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if node.id == 0 || !ids.insert(node.id) {
                return Err("Each patch node must have a unique ID.".into());
            }
            if !node.position[0].is_finite() || !node.position[1].is_finite() {
                return Err("A node has an invalid canvas position.".into());
            }
            validate_node_text(&node.kind)?;
        }
        let output_nodes = self
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, PatchNodeKind::Output))
            .count();
        if output_nodes != 1 {
            return Err("Add exactly one Audio output node to complete the patch.".into());
        }
        let mut incoming = HashMap::<u64, usize>::new();
        let mut seen_wires = HashSet::with_capacity(self.connections.len());
        for wire in &self.connections {
            let source = self
                .nodes
                .iter()
                .find(|node| node.id == wire.source)
                .ok_or_else(|| "A cable points to a missing node.".to_owned())?;
            let target = self
                .nodes
                .iter()
                .find(|node| node.id == wire.target)
                .ok_or_else(|| "A cable points to a missing node.".to_owned())?;
            if source.kind.output_kind() != Some(wire.signal)
                || target.kind.input_kind() != Some(wire.signal)
            {
                return Err(format!(
                    "The cable between {} and {} has incompatible ports.",
                    source.kind.title(),
                    target.kind.title()
                ));
            }
            if !seen_wires.insert((wire.source, wire.target)) {
                return Err("The patch contains a duplicate cable.".into());
            }
            if self.can_reach(wire.target, wire.source) {
                return Err("The patch contains a loop. Connections must flow forward.".into());
            }
            if *incoming.entry(wire.target).or_default() > 0
                && !target.kind.allows_multiple_inputs()
            {
                return Err(format!(
                    "{} has more than one input cable.",
                    target.kind.title()
                ));
            }
            *incoming.entry(wire.target).or_default() += 1;
        }
        for node in &self.nodes {
            if node.kind.requires_input() && incoming.get(&node.id).copied().unwrap_or(0) == 0 {
                return Err(format!(
                    "Connect an input to the {} node.",
                    node.kind.title()
                ));
            }
        }
        for source in &self.nodes {
            if source.kind.output_kind().is_some() && !self.reaches_output(source.id) {
                return Err(format!(
                    "Connect the {} node to the Audio output or remove it.",
                    source.kind.title()
                ));
            }
        }
        let midi_generators = self
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, PatchNodeKind::MidiVst { .. }))
            .count();
        if midi_generators > 1 {
            return Err(
                "This plugin currently routes one hosted MIDI source per score. Keep one MIDI VST3 source in the patch."
                    .into(),
            );
        }
        let output = self
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::Output))
            .expect("validated output");
        if incoming.get(&output.id).copied().unwrap_or(0) == 0 {
            return Err("Connect at least one instrument or effect to Audio output.".into());
        }
        Ok(())
    }

    pub fn generate_code(&self) -> Result<String, String> {
        self.validate()?;
        let output = self
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::Output))
            .expect("validated output");
        let mut branches = Vec::new();
        let mut midi_target = None;
        for wire in self
            .connections
            .iter()
            .filter(|wire| wire.target == output.id)
        {
            let branch = self.render_node(wire.source, &mut HashSet::new())?;
            if branch.signal != PortKind::Audio {
                return Err("Only audio can connect to Audio output.".into());
            }
            if let Some(target) = branch.midi_target {
                if midi_target.replace(target).is_some() {
                    return Err(
                        "A score can have only one hosted MIDI target. Connect one MIDI source to one instrument."
                            .into(),
                    );
                }
            }
            branches.push(branch.code);
        }
        branches.sort();
        let body = match branches.as_slice() {
            [] => return Err("Connect at least one instrument to Audio output.".into()),
            [only] => only.clone(),
            many => format!("stack(\n  {}\n)", many.join(",\n  ")),
        };
        let mut lines = Vec::new();
        if let Some(target) = midi_target {
            lines.push(format!("// MIDI target: {target}"));
        }
        let mut orc_files = self
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                PatchNodeKind::Instrument {
                    instrument: InstrumentKind::Csound,
                    orc_file: Some(path),
                    ..
                } => Some(path.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        orc_files.sort_unstable();
        orc_files.dedup();
        for path in orc_files {
            lines.push(format!(
                "await loadOrc({});",
                js_string_literal(&format!("local:{path}"))
            ));
        }
        lines.push(body);
        let code = lines.join("\n");
        if code.len() > 128 * 1024 {
            return Err("Generated score exceeds the Rustel score limit of 128 KiB.".into());
        }
        Ok(code)
    }

    pub fn to_json(&self) -> Result<String, String> {
        if self.nodes.len() > MAX_PATCH_NODES || self.connections.len() > MAX_PATCH_CONNECTIONS {
            return Err("The patch is too large to save.".into());
        }
        let json = serde_json::to_string(self)
            .map_err(|error| format!("Could not save the patch: {error}"))?;
        if json.len() > MAX_PATCH_STATE_BYTES {
            return Err("The patch is too large to save with the plugin state.".into());
        }
        Ok(json)
    }

    pub fn from_json(source: &str) -> Result<Self, String> {
        if source.len() > MAX_PATCH_STATE_BYTES {
            return Err("The saved patch is larger than the supported limit.".into());
        }
        let mut graph: Self = serde_json::from_str(source)
            .map_err(|error| format!("Could not read the saved patch: {error}"))?;
        if graph.version != 1 {
            return Err(format!(
                "The saved patch format version {} is not supported.",
                graph.version
            ));
        }
        if graph.nodes.len() > MAX_PATCH_NODES || graph.connections.len() > MAX_PATCH_CONNECTIONS {
            return Err("The saved patch exceeds the node or cable limit.".into());
        }
        let largest_id = graph.nodes.iter().map(|node| node.id).max().unwrap_or(0);
        graph.next_id = graph.next_id.max(largest_id.saturating_add(1));
        Ok(graph)
    }

    fn render_node(
        &self,
        node_id: u64,
        visiting: &mut HashSet<u64>,
    ) -> Result<GeneratedExpression, String> {
        if !visiting.insert(node_id) {
            return Err("The patch contains a loop.".into());
        }
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| "The patch references a missing node.".to_owned())?;
        let incoming = self.connections.iter().find(|wire| wire.target == node_id);
        let mut upstream = || {
            let wire = incoming
                .ok_or_else(|| format!("Connect an input to the {} node.", node.kind.title()))?;
            self.render_node(wire.source, visiting)
        };
        let generated = match &node.kind {
            PatchNodeKind::NotePattern { expression } => GeneratedExpression {
                code: expression.trim().to_owned(),
                signal: PortKind::Midi,
                midi_target: None,
                hosted_midi_source: false,
            },
            PatchNodeKind::MidiVst { plugin, is_effect } => {
                let input = upstream()?;
                if input.signal != PortKind::Midi {
                    return Err("A MIDI VST source needs a MIDI input.".into());
                }
                let method = if *is_effect { "vst" } else { "vsti" };
                GeneratedExpression {
                    code: format!("({}).{method}({})", input.code, js_string_literal(plugin)),
                    signal: PortKind::Midi,
                    midi_target: input.midi_target,
                    hosted_midi_source: true,
                }
            }
            PatchNodeKind::Instrument {
                instrument, value, ..
            } => {
                let input = upstream()?;
                if input.signal != PortKind::Midi {
                    return Err("An instrument needs a MIDI input.".into());
                }
                if input.hosted_midi_source {
                    if input.midi_target.is_some() {
                        return Err(
                            "Only one MIDI source can feed an instrument in this patch.".into()
                        );
                    }
                    GeneratedExpression {
                        code: input.code,
                        signal: PortKind::Audio,
                        midi_target: Some(instrument.midi_target(value.trim())?),
                        hosted_midi_source: true,
                    }
                } else {
                    let code = match instrument {
                        InstrumentKind::RustelSynth
                        | InstrumentKind::Sample
                        | InstrumentKind::Wavetable => {
                            format!("({}).s({})", input.code, js_string_literal(value.trim()))
                        }
                        InstrumentKind::Graph => {
                            format!("K({}).note({})", value.trim(), input.code)
                        }
                        InstrumentKind::Csound => format!(
                            "({}).csoundm({})",
                            input.code,
                            js_string_literal(value.trim())
                        ),
                        InstrumentKind::Vsti => {
                            format!("({}).vsti({})", input.code, js_string_literal(value.trim()))
                        }
                    };
                    GeneratedExpression {
                        code,
                        signal: PortKind::Audio,
                        midi_target: None,
                        hosted_midi_source: false,
                    }
                }
            }
            PatchNodeKind::Expression { signal, suffix } => {
                let input = upstream()?;
                let port = signal.port_kind();
                if input.signal != port {
                    return Err(format!(
                        "This expression requires a {} input.",
                        port.label()
                    ));
                }
                if input.midi_target.is_some() && port == PortKind::Audio {
                    return Err("An effect cannot be added after a hosted MIDI target in the current score renderer.".into());
                }
                GeneratedExpression {
                    code: format!("({}){}", input.code, suffix.trim()),
                    signal: port,
                    midi_target: input.midi_target,
                    hosted_midi_source: input.hosted_midi_source,
                }
            }
            PatchNodeKind::VstEffect { plugin, parameters } => {
                let input = upstream()?;
                if input.signal != PortKind::Audio {
                    return Err("A VST3 effect needs an audio input.".into());
                }
                if input.midi_target.is_some() {
                    return Err("A VST3 effect cannot follow a hosted MIDI target in the current score renderer.".into());
                }
                let parameters = parameters.trim();
                let args = if parameters.is_empty() {
                    js_string_literal(plugin)
                } else {
                    format!("{}, {parameters}", js_string_literal(plugin))
                };
                GeneratedExpression {
                    code: format!("({}).vst({args})", input.code),
                    signal: PortKind::Audio,
                    midi_target: None,
                    hosted_midi_source: false,
                }
            }
            PatchNodeKind::Output => {
                return Err("Audio output is the end of a patch, not a source.".into());
            }
        };
        visiting.remove(&node_id);
        Ok(generated)
    }

    fn can_reach(&self, start: u64, target: u64) -> bool {
        let mut visited = HashSet::new();
        let mut pending = vec![start];
        while let Some(current) = pending.pop() {
            if current == target {
                return true;
            }
            if visited.insert(current) {
                pending.extend(
                    self.connections
                        .iter()
                        .filter(|wire| wire.source == current)
                        .map(|wire| wire.target),
                );
            }
        }
        false
    }

    fn reaches_output(&self, source: u64) -> bool {
        let mut visited = HashSet::new();
        let mut pending = vec![source];
        while let Some(current) = pending.pop() {
            if self
                .nodes
                .iter()
                .any(|node| node.id == current && matches!(node.kind, PatchNodeKind::Output))
            {
                return true;
            }
            if visited.insert(current) {
                pending.extend(
                    self.connections
                        .iter()
                        .filter(|wire| wire.source == current)
                        .map(|wire| wire.target),
                );
            }
        }
        false
    }
}

struct GeneratedExpression {
    code: String,
    signal: PortKind,
    midi_target: Option<String>,
    hosted_midi_source: bool,
}

fn validate_node_text(kind: &PatchNodeKind) -> Result<(), String> {
    let mut values: Vec<&str> = match kind {
        PatchNodeKind::NotePattern { expression } => vec![expression],
        PatchNodeKind::MidiVst { plugin, .. } => vec![plugin],
        PatchNodeKind::Instrument { value, .. } => vec![value],
        PatchNodeKind::Expression { suffix, .. } => vec![suffix],
        PatchNodeKind::VstEffect { plugin, parameters } => vec![plugin, parameters],
        PatchNodeKind::Output => Vec::new(),
    };
    if let PatchNodeKind::Instrument {
        orc_file: Some(path),
        ..
    } = kind
    {
        values.push(path);
    }
    for value in values {
        if value.len() > MAX_PATCH_TEXT_BYTES {
            return Err(format!(
                "Node text exceeds the {MAX_PATCH_TEXT_BYTES}-byte limit."
            ));
        }
        if value.chars().any(char::is_control) {
            return Err("Node values cannot contain control characters.".into());
        }
    }
    match kind {
        PatchNodeKind::NotePattern { expression } if expression.trim().is_empty() => {
            Err("Enter a note pattern expression.".into())
        }
        PatchNodeKind::MidiVst { plugin, .. } | PatchNodeKind::VstEffect { plugin, .. }
            if plugin.trim().is_empty() =>
        {
            Err("Enter a scanned VST3 plugin name.".into())
        }
        PatchNodeKind::Instrument {
            instrument, value, ..
        } if value.trim().is_empty() => Err(format!(
            "Enter a value for the {} node.",
            instrument.label()
        )),
        PatchNodeKind::Instrument {
            instrument,
            orc_file: Some(_),
            ..
        } if *instrument != InstrumentKind::Csound => {
            Err("An ORC file can only be attached to a Csound instrument.".into())
        }
        PatchNodeKind::Instrument {
            instrument: InstrumentKind::Csound,
            orc_file: Some(path),
            ..
        } if !std::path::Path::new(path).is_absolute() => {
            Err("Choose an ORC file from disk before generating this patch.".into())
        }
        PatchNodeKind::Expression { suffix, .. }
            if suffix.trim().is_empty() || !suffix.trim().starts_with('.') =>
        {
            Err("Enter a Rustel method suffix, such as .slow(2) or .gain(0.5).".into())
        }
        PatchNodeKind::VstEffect { parameters, .. }
            if !parameters.trim().is_empty()
                && !(parameters.trim().starts_with('{') && parameters.trim().ends_with('}')) =>
        {
            Err("VST effect parameters must be a literal object such as { mix: 0.5 }.".into())
        }
        _ => Ok(()),
    }
}

fn js_string_literal(value: &str) -> String {
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

fn finite_position(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-4096.0, 4096.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_audio_effect(graph: &mut PatchGraph, source: u64) -> u64 {
        let effect = graph
            .add_node(
                PatchNodeKind::VstEffect {
                    plugin: "Room Verb".into(),
                    parameters: "{ mix: 0.5 }".into(),
                },
                [400.0, 240.0],
            )
            .expect("effect node");
        let output = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::Output))
            .expect("output")
            .id;
        graph.disconnect(source, output);
        graph.connect(source, effect).expect("effect cable");
        graph.connect(effect, output).expect("output cable");
        effect
    }

    #[test]
    fn default_patch_generates_a_note_to_synth_audio_chain() {
        let code = PatchGraph::default()
            .generate_code()
            .expect("default patch generates");
        assert!(code.contains("note(\"c3 e3 g3 b3\")"));
        assert!(code.contains(".s(\"sine\")"));
    }

    #[test]
    fn audio_chain_generates_instrument_effect_and_output_code() {
        let mut graph = PatchGraph::default();
        let synth = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::Instrument { .. }))
            .expect("synth")
            .id;
        add_audio_effect(&mut graph, synth);
        let code = graph.generate_code().expect("audio chain generates");
        assert!(code.contains(".s(\"sine\")"));
        assert!(code.contains(".vst(\"Room Verb\", { mix: 0.5 })"));
    }

    #[test]
    fn selected_csound_orc_is_loaded_before_generated_notes() {
        let mut graph = PatchGraph::default();
        let instrument = graph
            .nodes
            .iter_mut()
            .find(|node| matches!(node.kind, PatchNodeKind::Instrument { .. }))
            .expect("instrument");
        instrument.kind = PatchNodeKind::Instrument {
            instrument: InstrumentKind::Csound,
            value: "fm_voice".into(),
            orc_file: Some("/trusted/orchestra/fm.orc".into()),
        };

        let code = graph.generate_code().expect("Csound patch generates");
        assert!(code.starts_with("await loadOrc(\"local:/trusted/orchestra/fm.orc\");"));
        assert!(code.contains(".csoundm(\"fm_voice\")"));
    }

    #[test]
    fn hosted_midi_source_targets_an_instrument_and_preserves_note_expression() {
        let mut graph = PatchGraph::default();
        graph.nodes.clear();
        graph.connections.clear();
        let pattern = graph
            .add_node(
                PatchNodeKind::NotePattern {
                    expression: "note(\"c4 e4\")".into(),
                },
                [20.0, 24.0],
            )
            .expect("pattern");
        let source = graph
            .add_node(
                PatchNodeKind::MidiVst {
                    plugin: "MIDI Arpeggiator".into(),
                    is_effect: true,
                },
                [270.0, 24.0],
            )
            .expect("MIDI VST");
        let target = graph
            .add_node(
                PatchNodeKind::Instrument {
                    instrument: InstrumentKind::Vsti,
                    value: "Soft Keys".into(),
                    orc_file: None,
                },
                [520.0, 24.0],
            )
            .expect("instrument");
        let output = graph
            .add_node(PatchNodeKind::Output, [770.0, 24.0])
            .expect("output");
        graph.connect(pattern, source).expect("pattern to MIDI VST");
        graph.connect(source, target).expect("MIDI VST to target");
        graph.connect(target, output).expect("instrument to output");
        let code = graph.generate_code().expect("MIDI graph generates");
        assert!(code.contains("// MIDI target: vsti \"Soft Keys\""));
        assert!(code.contains(".vst(\"MIDI Arpeggiator\")"));
        assert!(code.contains("note(\"c4 e4\")"));
    }

    #[test]
    fn hosted_midi_can_target_every_supported_instrument_family() {
        let targets = [
            (InstrumentKind::RustelSynth, "sine", "rustel \"sine\""),
            (InstrumentKind::Sample, "bd", "sample \"bd\""),
            (InstrumentKind::Wavetable, "wt_x", "wavetable \"wt_x\""),
            (
                InstrumentKind::Graph,
                "sine(sFreq).mul(0.2).out()",
                "graph sine(sFreq).mul(0.2).out()",
            ),
            (InstrumentKind::Csound, "triangle", "csound \"triangle\""),
            (InstrumentKind::Vsti, "Soft Keys", "vsti \"Soft Keys\""),
        ];
        for (instrument, value, expected_target) in targets {
            let mut graph = PatchGraph::default();
            graph.nodes.clear();
            graph.connections.clear();
            let pattern = graph
                .add_node(
                    PatchNodeKind::NotePattern {
                        expression: "note(\"c4 e4\")".into(),
                    },
                    [20.0, 24.0],
                )
                .expect("pattern");
            let source = graph
                .add_node(
                    PatchNodeKind::MidiVst {
                        plugin: "MIDI Generator".into(),
                        is_effect: false,
                    },
                    [250.0, 24.0],
                )
                .expect("MIDI VST");
            let target = graph
                .add_node(
                    PatchNodeKind::Instrument {
                        instrument,
                        value: value.into(),
                        orc_file: None,
                    },
                    [480.0, 24.0],
                )
                .expect("instrument");
            let output = graph
                .add_node(PatchNodeKind::Output, [710.0, 24.0])
                .expect("output");
            graph.connect(pattern, source).expect("pattern cable");
            graph.connect(source, target).expect("MIDI cable");
            graph.connect(target, output).expect("audio cable");
            let code = graph.generate_code().expect("MIDI route generates");
            assert!(
                code.contains(&format!("// MIDI target: {expected_target}")),
                "missing target {expected_target} in {code}"
            );
            assert!(code.contains(".vsti(\"MIDI Generator\")"));
        }
    }

    #[test]
    fn default_patch_generated_score_is_accepted_by_the_rustel_compiler() {
        let code = PatchGraph::default()
            .generate_code()
            .expect("default patch generates");
        std::thread::Builder::new()
            .stack_size(rustel_engine::QUERY_WORKER_STACK_BYTES)
            .spawn(move || {
                let cancelled = std::sync::atomic::AtomicBool::new(false);
                crate::compiler::compile(&code, 48_000, 120.0, 1, 4, false, "", "", &cancelled)
                    .expect("generated default patch compiles");
            })
            .expect("compiler test worker")
            .join()
            .expect("compiler test");
    }

    #[test]
    fn generated_kabelsalat_instrument_score_is_accepted_by_the_rustel_compiler() {
        let mut graph = PatchGraph::default();
        let instrument = graph
            .nodes
            .iter_mut()
            .find(|node| matches!(node.kind, PatchNodeKind::Instrument { .. }))
            .expect("instrument");
        instrument.kind = PatchNodeKind::Instrument {
            instrument: InstrumentKind::Graph,
            value: "sine(sFreq).mul(0.2).out()".into(),
            orc_file: None,
        };
        let code = graph.generate_code().expect("graph patch generates");
        std::thread::Builder::new()
            .stack_size(rustel_engine::QUERY_WORKER_STACK_BYTES)
            .spawn(move || {
                let cancelled = std::sync::atomic::AtomicBool::new(false);
                crate::compiler::compile(&code, 48_000, 120.0, 1, 4, false, "", "", &cancelled)
                    .expect("generated KabelSalat patch compiles");
            })
            .expect("compiler test worker")
            .join()
            .expect("compiler test");
    }

    #[test]
    fn graph_rejects_incompatible_connections_and_saved_cycles() {
        let mut graph = PatchGraph::default();
        let synth = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::Instrument { .. }))
            .expect("synth")
            .id;
        let effect = graph
            .add_node(
                PatchNodeKind::VstEffect {
                    plugin: "Room Verb".into(),
                    parameters: String::new(),
                },
                [400.0, 240.0],
            )
            .expect("effect");
        assert!(graph.connect(synth, effect).is_ok());
        assert!(graph.connect(effect, synth).is_err());
        let note = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, PatchNodeKind::NotePattern { .. }))
            .expect("note")
            .id;
        assert!(graph.connect(note, effect).is_err());

        let mut cyclic = PatchGraph::default();
        cyclic.nodes.clear();
        cyclic.connections.clear();
        let first = cyclic
            .add_node(
                PatchNodeKind::Expression {
                    signal: ExpressionSignal::Midi,
                    suffix: ".slow(2)".into(),
                },
                [20.0, 24.0],
            )
            .expect("first transform");
        let second = cyclic
            .add_node(
                PatchNodeKind::Expression {
                    signal: ExpressionSignal::Midi,
                    suffix: ".fast(2)".into(),
                },
                [250.0, 24.0],
            )
            .expect("second transform");
        cyclic
            .add_node(PatchNodeKind::Output, [480.0, 24.0])
            .expect("output");
        cyclic.connections.extend([
            PatchConnection {
                source: first,
                target: second,
                signal: PortKind::Midi,
            },
            PatchConnection {
                source: second,
                target: first,
                signal: PortKind::Midi,
            },
        ]);
        assert!(
            cyclic
                .validate()
                .expect_err("saved loop is refused")
                .contains("loop")
        );
    }

    #[test]
    fn patch_state_round_trips_and_keeps_connections() {
        let graph = PatchGraph::default();
        let saved = graph.to_json().expect("serialize patch");
        assert_eq!(PatchGraph::from_json(&saved).expect("restore patch"), graph);
    }

    #[test]
    fn old_csound_instrument_nodes_restore_without_an_orc_file() {
        let node = serde_json::from_str::<PatchNodeKind>(
            r#"{"node_type":"instrument","instrument":"csound","value":"voice"}"#,
        )
        .expect("restore a saved Csound node");
        assert!(matches!(
            node,
            PatchNodeKind::Instrument {
                instrument: InstrumentKind::Csound,
                orc_file: None,
                ..
            }
        ));
    }

    #[test]
    fn cleared_patch_persists_as_an_empty_canvas_and_accepts_new_nodes() {
        let mut graph = PatchGraph::default();
        graph.clear();
        assert!(graph.nodes.is_empty());
        assert!(graph.connections.is_empty());

        let encoded = graph.to_json().expect("save empty canvas");
        let mut restored = PatchGraph::from_json(&encoded).expect("restore empty canvas");
        let note = restored
            .add_node(
                PatchNodeKind::NotePattern {
                    expression: "note(\"c3\")".into(),
                },
                [14.0, 24.0],
            )
            .expect("add a block after clearing");

        assert_eq!(restored.nodes[0].id, note);
        assert_eq!(note, 1);
    }

    #[test]
    fn restore_advances_the_node_id_counter_past_saved_nodes() {
        let mut graph = PatchGraph::default();
        graph.next_id = 1;
        let restored = PatchGraph::from_json(&graph.to_json().expect("serialize patch"))
            .expect("restore patch");
        let mut restored = restored;
        let id = restored
            .add_node(
                PatchNodeKind::NotePattern {
                    expression: "note(\"a3\")".into(),
                },
                [10.0, 10.0],
            )
            .expect("new node");
        assert!(!restored.nodes[..3].iter().any(|node| node.id == id));
    }
}
