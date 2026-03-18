# AGC: Two-Stage Automatic Gain Control for Audio Pipeline

## Context

Quiet/whispered speech falls below the Silero VAD's hardcoded 0.3 threshold and gets silently dropped, even though the transcription model (Qwen) can handle the audio. There is no normalization anywhere in the pipeline. Two AGC stages address this: a streaming AGC before VAD (so quiet speech isn't rejected), and a whole-buffer normalization before transcription (so the model sees consistent levels).

---

## Stage 1: Streaming AGC (per-frame, before VAD)

### New file: `src-tauri/src/audio_toolkit/agc.rs`

**`StreamingAgc` struct:**
- Asymmetric EMA envelope follower on frame RMS
- `process_frame(&mut self, frame: &mut [f32])` — computes frame RMS, updates smoothed RMS, derives gain, applies in-place with clipping protection (clamp to [-1, 1])
- Parameters (hardcoded constants, not user settings):
  - `target_rms: 0.1`
  - `max_gain: 30.0` (cap to prevent noise explosion)
  - `noise_floor: 0.001` (below this RMS, gain = 1.0 — don't amplify silence)
  - `attack_ms: 75` (fast response to louder input)
  - `release_ms: 400` (slow gain increase prevents pumping)
  - `frame_ms: 30` (extracted to `AGC_FRAME_MS` constant)
- EMA alpha from time constants: `alpha = 1.0 - exp(-frame_ms / time_constant_ms)`
- `smoothed_rms` initialized to `target_rms` (not 0.0) to avoid gain spike on first frame
- `reset()` restores `smoothed_rms` to `target_rms`

**`normalize_buffer` free function** (for stage 2):
- Computes whole-buffer RMS using f64 accumulator for precision on large buffers
- Scales to target, clamps samples to [-1.0, 1.0]
- Skips normalization if RMS < noise floor or gain ≈ 1.0

**`frame_rms` helper:**
- Uses f64 accumulation to avoid precision loss on buffers up to 960k samples (60s at 16kHz)

Unit tests: constant sine convergence, silence passthrough, no-attenuate invariant, level-drop ramp-up, cold-start no-spike, reset state, normalize_buffer scaling + silence skip + clamp + empty.

### Wire into `src-tauri/src/audio_toolkit/audio/recorder.rs`

**`AudioRecorder`:**
- Add `agc: Option<Arc<Mutex<StreamingAgc>>>` field
- Add `.with_agc(agc: StreamingAgc) -> Self` builder method
- Pass `agc` into `run_consumer()`

**`run_consumer()`:**
- Accept `agc` parameter
- Add reusable `agc_buf: Vec<f32>` local buffer (avoids per-frame allocation)
- Reset AGC on `Cmd::Start`

**`handle_frame()` (nested fn):**
- New parameters: `agc: &Option<Arc<Mutex<StreamingAgc>>>` + `agc_tmp: &mut Vec<f32>`
- Before VAD: copy frame into `agc_tmp`, apply AGC in-place, pass AGC'd slice to both VAD and `out_buf`
- SmoothedVad's internal `frame_buffer` stores `frame.to_vec()`, so prefill frames contain AGC'd audio

---

## Stage 2: Whole-Buffer Normalization

### In `src-tauri/src/managers/audio.rs` — `stop_recording()`

After `rec.stop()` returns samples, before the short-padding logic:

```rust
if !samples.is_empty() {
    normalize_buffer(&mut samples, NORMALIZE_TARGET_RMS, NORMALIZE_NOISE_FLOOR);
}
```

Changed `let samples` to `let mut samples`.

---

## VAD Threshold Setting

### `src-tauri/src/settings.rs`
- Add `vad_threshold: f32` with `#[serde(default = "default_vad_threshold")]`, default 0.3
- Add to `get_default_settings()`

### `src-tauri/src/managers/audio.rs` — `create_audio_recorder()`
- Read `settings.vad_threshold` instead of hardcoded 0.3
- Create `StreamingAgc::from_defaults()` and attach via `.with_agc()`

### `src-tauri/src/shortcut/mod.rs`
- Add `change_vad_threshold_setting` command
- Validate 0.05..=0.8 (matches frontend slider range)
- Guard against active recording — skip recorder recreation if recording in progress
- Log errors from `update_selected_device()` instead of silently discarding

### `src-tauri/src/lib.rs`
- Register command in `collect_commands!`

### Frontend
- `src/stores/settingsStore.ts` — add `vad_threshold` to `settingUpdaters`
- `src/components/settings/debug/VadThreshold.tsx` — new slider component (0.05–0.8, step 0.05)
- `src/components/settings/debug/DebugSettings.tsx` — include VadThreshold in debug settings panel
- `src/i18n/locales/en/translation.json` — i18n keys for "Voice Detection Sensitivity"

---

## Constants

### `src-tauri/src/audio_toolkit/constants.rs`
```rust
pub const AGC_FRAME_MS: f32 = 30.0;
pub const AGC_TARGET_RMS: f32 = 0.1;
pub const AGC_MAX_GAIN: f32 = 30.0;
pub const AGC_NOISE_FLOOR: f32 = 0.001;
pub const AGC_ATTACK_MS: f32 = 75.0;
pub const AGC_RELEASE_MS: f32 = 400.0;
pub const NORMALIZE_TARGET_RMS: f32 = 0.1;
pub const NORMALIZE_NOISE_FLOOR: f32 = 0.001;
```

---

## Module registration

### `src-tauri/src/audio_toolkit/mod.rs`
- Add `pub mod agc;`
- Add `pub use agc::{StreamingAgc, normalize_buffer};`

---

## Files modified (summary)

| File | Change |
|------|--------|
| `src-tauri/src/audio_toolkit/agc.rs` | **New** — StreamingAgc + normalize_buffer + tests |
| `src-tauri/src/audio_toolkit/mod.rs` | Add module + re-exports |
| `src-tauri/src/audio_toolkit/constants.rs` | AGC constants |
| `src-tauri/src/audio_toolkit/audio/recorder.rs` | Wire AGC into AudioRecorder, run_consumer, handle_frame |
| `src-tauri/src/managers/audio.rs` | Create AGC in create_audio_recorder, normalize in stop_recording, read threshold from settings |
| `src-tauri/src/settings.rs` | Add vad_threshold field |
| `src-tauri/src/shortcut/mod.rs` | Add change_vad_threshold_setting command |
| `src-tauri/src/lib.rs` | Register command |
| `src/stores/settingsStore.ts` | Add vad_threshold updater |
| `src/components/settings/debug/VadThreshold.tsx` | **New** — slider component |
| `src/components/settings/debug/DebugSettings.tsx` | Include VadThreshold component |
| `src/i18n/locales/en/translation.json` | i18n keys for VAD threshold setting |

## Not changed

- SileroVad / SmoothedVad internals — AGC sits outside them
- VoiceActivityDetector trait
- FrameResampler — AGC is post-resample
- AudioVisualiser — operates on raw pre-resample audio (shows real mic levels, not AGC'd)
- cpal stream callback / sample format handling

---

## Work completed

### Commit 1: `feat: add two-stage AGC and configurable VAD threshold`
All planned implementation work — AGC module, pipeline wiring, settings, Tauri command, frontend UI.

### Commit 2: `fix: address review findings for AGC implementation`
Fixes from principal-engineer agent review:
- **Clipping protection** — `process_frame` now clamps samples to [-1, 1] after gain (matching `normalize_buffer`)
- **Cold-start gain spike** — `smoothed_rms` initialized to `target_rms` instead of 0.0, so AGC starts at unity gain
- **f64 accumulator** — `frame_rms` uses f64 for sum-of-squares to avoid precision loss on large buffers
- **Frame duration constant** — extracted hardcoded `30.0` to `AGC_FRAME_MS`
- **Recording guard** — `change_vad_threshold_setting` skips recorder recreation during active recording
- **Error logging** — `update_selected_device()` errors are logged instead of silently discarded
- **Validation alignment** — backend validates 0.05..=0.8 matching the frontend slider range
- **New test** — `streaming_agc_cold_start_no_spike` verifies first frame doesn't get max gain

### Review findings not addressed (intentional)
- **Max gain 30x / noise floor tuning** — these are empirical tuning parameters; reducing gain pre-emptively defeats the purpose (whisper detection). Tune after real-world testing.
- **Double normalization concern** — stage 2 is intentionally a "final polish" for edge cases where stage 1 hasn't converged. Same target RMS is correct.
- **Mutex overhead** — matches existing codebase pattern (VAD uses same Arc<Mutex<>> per-frame). Not worth changing in isolation.
- **`&Option<Arc<...>>` idiom** — matches existing `handle_frame` pattern for VAD.

### Build status
- `cargo fmt` — clean
- `cargo check` — no Rust compilation errors (build-script failures from missing system deps `libclang-dev` and `libgtk-layer-shell-dev` are pre-existing)
- `cargo test` — blocked by same missing system deps
- Frontend lint — bun not available in current environment

---

## Verification (pending)

1. `cargo test` — run AGC unit tests once build deps are installed
2. `cargo clippy` — check for warnings
3. `bun run lint` + `bun run format:check` — frontend checks
4. Manual test: record whispered speech at arm's length — verify VAD accepts it and transcription succeeds
5. Manual test: normal volume speech — verify no audible artifacts or quality regression
6. Manual test: adjust VAD threshold slider in settings UI — verify it persists and takes effect

---

## Future work

- **Silero VAD v5 upgrade** — planned as next step after AGC. v5 has better accuracy on edge cases including low-energy speech.
- **AGC parameter tuning** — `max_gain`, `noise_floor`, `attack_ms`, `release_ms` may need adjustment based on real-world testing across different microphones and environments.
