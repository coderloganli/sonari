# call control audio

`call/control/bot-speech` owns business-level bot speech queue semantics inside `call control`.

## Responsibilities

- define `BotSpeechItem` as the unified business-owned bot speech input model
- define source type, trigger type, priority, interruptibility, and queue policy
- own user barge-in queue-flush decisions
- own business audio event vocabulary such as:
  - `bot_speech_enqueued`
  - `bot_speech_started`
  - `bot_speech_completed`
  - `bot_speech_interrupted`
  - `queue_flushed`
- define the queue policy consumed by runtime execution components
- keep business audio semantics separate from worker/media implementation details

## Non-responsibilities

- LiveKit track handling
- PCM decoding, resampling, or mixing
- denoise / preprocessing implementation
- playback interruption execution
- bot audio sink management
- low-level frame buffering

Those execution-plane responsibilities live under:

- `call/worker` for runtime media control, preprocessing, interruption, and mixing
- `call/rtc` for LiveKit-specific media adapters
