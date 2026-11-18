#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_audio_utils/juce_audio_utils.h>
#include <juce_gui_extra/juce_gui_extra.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cmath>
#include <functional>
#include <memory>
#include <stdexcept>
#include <vector>

namespace
{
using namespace juce;

struct CaptureConfig
{
    File pluginPath;
    File inputMidiPath;
    File outputMidiPath;
    File statusPath;
    String pluginName;
    String streamHost;
    String streamToken;
    int streamPort = 0;
    int durationSeconds = 30;
    int ticksPerBeat = 480;
    int tempoMicroseconds = 500000;

    static CaptureConfig load(const File& configPath)
    {
        const auto parsed = JSON::parse(configPath.loadFileAsString());
        auto* object = parsed.getDynamicObject();
        if (object == nullptr)
            throw std::runtime_error("The capture config is not valid JSON.");

        auto readString = [object](const Identifier& key)
        {
            return object->getProperty(key).toString();
        };

        CaptureConfig config;
        config.pluginPath = File(readString("plugin_path"));
        config.inputMidiPath = File(readString("input_midi_path"));
        config.outputMidiPath = File(readString("output_midi_path"));
        config.statusPath = File(readString("status_path"));
        config.pluginName = readString("plugin_name");
        config.streamHost = readString("stream_host");
        config.streamToken = readString("stream_token");
        config.streamPort = static_cast<int>(object->getProperty("stream_port"));
        config.durationSeconds =
            static_cast<int>(object->getProperty("duration_seconds"));
        config.ticksPerBeat =
            static_cast<int>(object->getProperty("ticks_per_beat"));
        config.tempoMicroseconds =
            static_cast<int>(object->getProperty("tempo_microseconds"));
        const auto streaming = config.streamPort > 0;

        if (config.pluginPath == File()
            || !config.pluginPath.exists())
            throw std::runtime_error("The selected VST3 bundle was not found.");
        if (config.outputMidiPath == File() && !streaming)
            throw std::runtime_error("The output MIDI path is missing.");
        if (config.streamPort < 0)
            throw std::runtime_error("The MIDI stream port is invalid.");
        if (config.durationSeconds < (streaming ? 0 : 1)
            || config.durationSeconds > 600)
            throw std::runtime_error(
                "Capture duration must be between 1 and 600 seconds, or 0 for an unlimited MIDI stream."
            );
        if (config.ticksPerBeat < 1 || config.tempoMicroseconds < 1)
            throw std::runtime_error("The target MIDI timing is invalid.");
        if (streaming
            && (config.streamHost.isEmpty()
                || config.streamPort > 65535
                || config.streamToken.length() != 32))
            throw std::runtime_error("The live MIDI stream settings are invalid.");

        return config;
    }
};

void writeStatus(const File& statusPath, bool ok, const String& message)
{
    if (statusPath == File())
        return;

    DynamicObject::Ptr object = new DynamicObject();
    object->setProperty("ok", ok);
    object->setProperty("message", message);

    statusPath.getParentDirectory().createDirectory();
    FileOutputStream stream(statusPath);
    if (!stream.openedOk())
        return;
    stream.setPosition(0);
    stream.truncate();
    stream.writeText(JSON::toString( var(object.get()), true), false, false, "\n");
    stream.flush();
}

struct TimedMidiMessage
{
    double seconds = 0.0;
    MidiMessage message;
};

struct CapturedMidiMessage
{
    int64 samplePosition = 0;
    MidiMessage message;
};

class CaptureComponent final : public AudioAppComponent,
                               private Timer
{
public:
    CaptureComponent(CaptureConfig captureConfig,
                     std::function<void(bool, const String&)> completion)
        : config(std::move(captureConfig)),
          completionCallback(std::move(completion)),
          startButton(config.streamPort > 0 ? "Start MIDI stream" : "Start capture"),
          cancelButton(config.streamPort > 0 ? "Close" : "Cancel")
    {
        if (config.streamPort > 0)
            parseStreamToken();
        loadInputMidi();
        loadPlugin();

        addAndMakeVisible(*pluginEditor);

        const auto editorWidth = jmax(520, pluginEditor->getWidth());
        const auto editorHeight = jmax(300, pluginEditor->getHeight());
        const auto streaming = config.streamPort > 0;
        if (streaming)
        {
            setSize(editorWidth, editorHeight);
        }
        else
        {
            addAndMakeVisible(startButton);
            addAndMakeVisible(cancelButton);
            addAndMakeVisible(statusLabel);
            startButton.onClick = [this] { startCapture(); };
            cancelButton.onClick = [this] { cancel(); };
            statusLabel.setText(
                "Plugin editor is open. Adjust the plugin, then start capture.",
                dontSendNotification);
            statusLabel.setColour(Label::textColourId, Colours::white);
            statusLabel.setJustificationType(Justification::centredLeft);
            setSize(editorWidth, editorHeight + 78);
        }

        setAudioChannels(0, 2);
        startTimerHz(20);
        if (streaming)
            startCapture();
    }

    ~CaptureComponent() override
    {
        stopTimer();
        shutdownAudio();
        if (pluginEditor != nullptr)
            removeChildComponent(pluginEditor);
        plugin.reset();
    }

    void cancel()
    {
        finish(false, "Capture cancelled.");
    }

    void prepareToPlay(int samplesPerBlockExpected,
                       double newSampleRate) override
    {
        currentSampleRate.store(newSampleRate);
        audioSampleCounter.store(0);
        inputEventCursor.store(0);
        if (plugin != nullptr)
            plugin->prepareToPlay(newSampleRate, samplesPerBlockExpected);
    }

    void getNextAudioBlock(const AudioSourceChannelInfo& info) override
    {
        if (info.buffer == nullptr || plugin == nullptr)
            return;

        info.clearActiveBufferRegion();

        const auto sampleRate = currentSampleRate.load();
        const auto blockStart = audioSampleCounter.fetch_add(
            info.numSamples,
            std::memory_order_relaxed
        );
        const auto captureStart = captureStartSample.load();
        const auto isRecording = capturing.load();

        MidiBuffer midiMessages;
        if (isRecording && sampleRate > 0.0)
            addInputEvents(midiMessages, blockStart, captureStart, sampleRate,
                           info.numSamples);

        plugin->processBlock(*info.buffer, midiMessages);

        if (isRecording)
        {
            if (config.streamPort > 0)
                streamMidiEvents(midiMessages);
            else
                collectOutputEvents(midiMessages, blockStart, captureStart);
        }
        if (config.streamPort > 0)
            info.clearActiveBufferRegion();
    }

    void releaseResources() override
    {
        if (plugin != nullptr)
            plugin->releaseResources();
    }

    void resized() override
    {
        auto area = getLocalBounds();
        if (config.streamPort > 0)
        {
            if (pluginEditor != nullptr)
                pluginEditor->setBounds(area);
            return;
        }
        auto controls = area.removeFromTop(78).reduced(10, 8);
        startButton.setBounds(controls.removeFromLeft(130));
        cancelButton.setBounds(controls.removeFromLeft(100).reduced(6, 0));
        statusLabel.setBounds(controls.reduced(8, 0));
        if (pluginEditor != nullptr)
            pluginEditor->setBounds(area);
    }

private:
    void loadInputMidi()
    {
        if (config.inputMidiPath == File())
            return;

        FileInputStream stream(config.inputMidiPath);
        if (!stream.openedOk())
            throw std::runtime_error("Could not open the source MIDI file.");

        MidiFile source;
        if (!source.readFrom(stream))
            throw std::runtime_error("The source MIDI file could not be read.");
        source.convertTimestampTicksToSeconds();

        for (int trackIndex = 0; trackIndex < source.getNumTracks(); ++trackIndex)
        {
            const auto* track = source.getTrack(trackIndex);
            if (track == nullptr)
                continue;

            for (int eventIndex = 0; eventIndex < track->getNumEvents(); ++eventIndex)
            {
                const auto* event = track->getEventPointer(eventIndex);
                if (event == nullptr || event->message.isMetaEvent())
                    continue;
                inputEvents.push_back(
                    { event->message.getTimeStamp(), event->message }
                );
            }
        }

        std::stable_sort(
            inputEvents.begin(),
            inputEvents.end(),
            [](const auto& a, const auto& b) { return a.seconds < b.seconds; }
        );
    }

    void loadPlugin()
    {
        VST3PluginFormat format;
        OwnedArray<PluginDescription> descriptions;
        format.findAllTypesForFile(
            descriptions,
            config.pluginPath.getFullPathName()
        );
        if (descriptions.isEmpty())
            throw std::runtime_error(
                "No VST3 plugin was found in the selected bundle."
            );

        String error;
        formatManager.addDefaultFormats();
        auto* description = descriptions.getFirst();
        if (config.pluginName.isNotEmpty())
        {
            description = nullptr;
            for (int index = 0; index < descriptions.size(); ++index)
            {
                auto* candidate = descriptions[index];
                if (candidate->name == config.pluginName)
                {
                    description = candidate;
                    break;
                }
            }
            if (description == nullptr)
                throw std::runtime_error(
                    "The selected VST3 class was not found in the bundle."
                );
        }

        plugin = formatManager.createPluginInstance(
            *description,
            44100.0,
            512,
            error
        );
        if (plugin == nullptr)
            throw std::runtime_error(
                ("Could not load the VST3 plugin: " + error).toStdString()
            );
        if (!plugin->producesMidi())
            throw std::runtime_error(
                "This VST3 does not declare MIDI output."
            );

        pluginEditor = plugin->createEditorIfNeeded();
        if (pluginEditor == nullptr)
            throw std::runtime_error(
                "This VST3 does not provide an editor window."
            );
    }

    void startCapture()
    {
        {
            const ScopedLock lock(capturedEventsLock);
            capturedEvents.clear();
        }
        inputEventCursor.store(0);
        captureStartSample.store(audioSampleCounter.load());
        captureStartTime = std::chrono::steady_clock::now();
        capturing.store(true);
        startButton.setEnabled(false);
        statusLabel.setText(
            config.streamPort > 0
                ? "Streaming MIDI to Rustel. Close this plugin editor to stop."
                : "Capturing MIDI output. The plugin editor remains available.",
            dontSendNotification
        );
    }

    void parseStreamToken()
    {
        auto nibble = [](juce_wchar character) -> int
        {
            if (character >= '0' && character <= '9')
                return character - '0';
            if (character >= 'a' && character <= 'f')
                return character - 'a' + 10;
            if (character >= 'A' && character <= 'F')
                return character - 'A' + 10;
            return -1;
        };

        for (size_t index = 0; index < streamTokenBytes.size(); ++index)
        {
            const auto high = nibble(streamToken[static_cast<int>(index * 2)]);
            const auto low = nibble(streamToken[static_cast<int>(index * 2 + 1)]);
            if (high < 0 || low < 0)
                throw std::runtime_error("The MIDI stream token is invalid.");
            streamTokenBytes[index] = static_cast<std::uint8_t>((high << 4) | low);
        }
    }

    void streamMidiEvents(const MidiBuffer& midiMessages)
    {
        constexpr size_t headerSize = 26;
        constexpr size_t eventSize = 4;
        constexpr size_t maxEvents = 64;
        std::array<std::uint8_t, headerSize + eventSize * maxEvents> packet {};
        packet[0] = 'R';
        packet[1] = 'S';
        packet[2] = 'M';
        packet[3] = '1';
        std::copy(streamTokenBytes.begin(), streamTokenBytes.end(), packet.begin() + 4);
        auto sequence = streamSequence.fetch_add(1, std::memory_order_relaxed);

        auto sendPacket = [&](size_t eventCount)
        {
            for (int byte = 0; byte < 4; ++byte)
                packet[20 + byte] = static_cast<std::uint8_t>(sequence >> (byte * 8));
            packet[24] = static_cast<std::uint8_t>(eventCount);
            packet[25] = static_cast<std::uint8_t>(eventCount >> 8);
            streamSocket.write(
                config.streamHost,
                config.streamPort,
                packet.data(),
                static_cast<int>(headerSize + eventCount * eventSize));
            sequence = streamSequence.fetch_add(1, std::memory_order_relaxed);
        };

        size_t eventCount = 0;
        for (const auto metadata : midiMessages)
        {
            const auto message = metadata.getMessage();
            if (!message.isNoteOnOrOff())
                continue;
            if (eventCount == maxEvents)
            {
                sendPacket(eventCount);
                eventCount = 0;
            }
            const auto offset = headerSize + eventCount * eventSize;
            const auto velocity = message.isNoteOn()
                ? static_cast<std::uint8_t>(
                    juce::jlimit(0, 127, juce::roundToInt(message.getFloatVelocity() * 127.0f)))
                : 0;
            packet[offset] = message.isNoteOn() ? 0x90 : 0x80;
            packet[offset + 1] = static_cast<std::uint8_t>(message.getChannel() - 1);
            packet[offset + 2] = static_cast<std::uint8_t>(message.getNoteNumber());
            packet[offset + 3] = velocity;
            ++eventCount;
        }
        sendPacket(eventCount);
    }

    void addInputEvents(MidiBuffer& midiMessages,
                        int64 blockStart,
                        int64 captureStart,
                        double sampleRate,
                        int numSamples)
    {
        const auto relativeBlockStart = jmax<int64>(0, blockStart - captureStart);
        const auto relativeBlockEnd = relativeBlockStart + numSamples;
        auto inputCursorIndex = inputEventCursor.load();

        while (inputCursorIndex < static_cast<int>(inputEvents.size()))
        {
            const auto& event = inputEvents[static_cast<size_t>(inputCursorIndex)];
            const auto eventSample = static_cast<int64>(
                std::llround(event.seconds * sampleRate)
            );
            if (eventSample >= relativeBlockEnd)
                break;

            const auto sampleOffset = static_cast<int>(
                jlimit<int64>(
                    0,
                    numSamples - 1,
                    eventSample - relativeBlockStart
                )
            );
            midiMessages.addEvent(event.message, sampleOffset);
            ++inputCursorIndex;
        }

        inputEventCursor.store(inputCursorIndex);
    }

    void collectOutputEvents(const MidiBuffer& midiMessages,
                             int64 blockStart,
                             int64 captureStart)
    {
        const ScopedLock lock(capturedEventsLock);
        for (const auto metadata : midiMessages)
        {
            const auto message = metadata.getMessage();
            if (message.isMetaEvent())
                continue;

            capturedEvents.push_back(
                {
                    blockStart + metadata.samplePosition - captureStart,
                    message
                }
            );
        }
    }

    void timerCallback() override
    {
        if (!capturing.load() || config.durationSeconds == 0)
            return;

        const auto elapsed = std::chrono::duration<double>(
            std::chrono::steady_clock::now() - captureStartTime
        ).count();
        if (elapsed >= config.durationSeconds)
            finish(true, {});
    }

    void finish(bool saveCapture, const String& reason)
    {
        if (finished)
            return;
        finished = true;
        capturing.store(false);
        stopTimer();
        startButton.setEnabled(false);
        cancelButton.setEnabled(false);
        shutdownAudio();

        if (!saveCapture)
        {
            writeStatus(config.statusPath, false, reason);
            if (completionCallback)
                completionCallback(false, reason);
            return;
        }

        try
        {
            const auto noteOnCount = saveMidiFile();
            const auto message =
                "Captured " + String(noteOnCount)
                + " MIDI note-on event(s).";
            statusLabel.setText(message, dontSendNotification);
            writeStatus(config.statusPath, true, message);
            if (completionCallback)
                completionCallback(true, message);
        }
        catch (const std::exception& exception)
        {
            const auto message = String(exception.what());
            statusLabel.setText(message, dontSendNotification);
            writeStatus(config.statusPath, false, message);
            if (completionCallback)
                completionCallback(false, message);
        }
    }

    int saveMidiFile()
    {
        std::vector<CapturedMidiMessage> events;
        {
            const ScopedLock lock(capturedEventsLock);
            events = capturedEvents;
        }

        std::stable_sort(
            events.begin(),
            events.end(),
            [](const auto& a, const auto& b)
            {
                return a.samplePosition < b.samplePosition;
            }
        );

        const auto sampleRate = currentSampleRate.load();
        if (sampleRate <= 0.0)
            throw std::runtime_error("The audio clock did not start.");

        MidiMessageSequence sequence;
        sequence.addEvent(
            MidiMessage::tempoMetaEvent(config.tempoMicroseconds),
            0.0
        );
        int noteOnCount = 0;
        for (auto& event : events)
        {
            const auto seconds = jmax<int64>(0, event.samplePosition) / sampleRate;
            const auto ticks = seconds * 1000000.0
                * config.ticksPerBeat / config.tempoMicroseconds;
            auto message = event.message;
            message.setTimeStamp(ticks);
            sequence.addEvent(message);
            if (message.isNoteOn())
                ++noteOnCount;
        }

        if (noteOnCount < 1)
            throw std::runtime_error(
                "The VST3 capture contained no MIDI note-on events."
            );

        MidiFile output;
        output.setTicksPerQuarterNote(config.ticksPerBeat);
        output.addTrack(sequence);

        config.outputMidiPath.getParentDirectory().createDirectory();
        FileOutputStream stream(config.outputMidiPath);
        if (!stream.openedOk())
            throw std::runtime_error("Could not create the captured MIDI file.");
        stream.setPosition(0);
        stream.truncate();
        if (!output.writeTo(stream, 0))
            throw std::runtime_error("Could not write the captured MIDI file.");
        stream.flush();
        if (stream.getStatus().failed())
            throw std::runtime_error("Could not flush the captured MIDI file.");
        return noteOnCount;
    }

    CaptureConfig config;
    std::function<void(bool, const String&)> completionCallback;
    AudioPluginFormatManager formatManager;
    std::unique_ptr<AudioPluginInstance> plugin;
    AudioProcessorEditor* pluginEditor = nullptr;
    std::vector<TimedMidiMessage> inputEvents;
    std::vector<CapturedMidiMessage> capturedEvents;
    CriticalSection capturedEventsLock;
    DatagramSocket streamSocket;
    std::array<std::uint8_t, 16> streamTokenBytes {};
    std::atomic<std::uint32_t> streamSequence { 0 };
    std::atomic<int64> audioSampleCounter { 0 };
    std::atomic<int64> captureStartSample { 0 };
    std::atomic<int> inputEventCursor { 0 };
    std::atomic<double> currentSampleRate { 0.0 };
    std::atomic<bool> capturing { false };
    std::chrono::steady_clock::time_point captureStartTime;
    TextButton startButton;
    TextButton cancelButton;
    Label statusLabel;
    bool finished = false;
};

class MainWindow final : public DocumentWindow
{
public:
    MainWindow(const String& title,
               CaptureConfig config,
               std::function<void(bool, const String&)> completion)
        : DocumentWindow(
              title,
              Desktop::getInstance().getDefaultLookAndFeel()
                  .findColour(ResizableWindow::backgroundColourId),
              DocumentWindow::allButtons
          )
    {
        setUsingNativeTitleBar(true);
        setResizable(true, true);
        setContentOwned(
            new CaptureComponent(std::move(config), std::move(completion)),
            true
        );
        centreWithSize(getWidth(), getHeight());
        setVisible(true);
    }

    void closeButtonPressed() override
    {
        if (auto* component =
                dynamic_cast<CaptureComponent*>(getContentComponent()))
            component->cancel();
        JUCEApplication::getInstance()->quit();
    }
};

class CaptureHostApplication final : public JUCEApplication
{
public:
    const String getApplicationName() override
    {
        return "Rustel MIDI VST3";
    }

    const String getApplicationVersion() override
    {
        return "0.1.0";
    }

    void initialise(const String& commandLine) override
    {
        const auto configPath = File(commandLine.trim().unquoted());
        try
        {
            config = CaptureConfig::load(configPath);
            mainWindow = std::make_unique<MainWindow>(
                config.pluginName.isNotEmpty()
                    ? config.pluginName
                    : getApplicationName(),
                config,
                [this](bool, const String&)
                {
                    Timer::callAfterDelay(300, [this] { quit(); });
                }
            );
        }
        catch (const std::exception& exception)
        {
            const auto message = String(exception.what());
            if (config.statusPath != File())
                writeStatus(config.statusPath, false, message);
            Logger::writeToLog(message);
            quit();
        }
    }

    void shutdown() override
    {
        mainWindow.reset();
    }

    void systemRequestedQuit() override
    {
        quit();
    }

private:
    CaptureConfig config;
    std::unique_ptr<MainWindow> mainWindow;
};
} // namespace

START_JUCE_APPLICATION(CaptureHostApplication)