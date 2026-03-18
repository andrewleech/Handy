pub const WHISPER_SAMPLE_RATE: u32 = 16000;

// VAD / AGC frame duration (must match the resampler frame size)
pub const AGC_FRAME_MS: f32 = 30.0;

// Streaming AGC defaults
pub const AGC_TARGET_RMS: f32 = 0.1;
pub const AGC_MAX_GAIN: f32 = 30.0;
pub const AGC_NOISE_FLOOR: f32 = 0.001;
pub const AGC_ATTACK_MS: f32 = 75.0;
pub const AGC_RELEASE_MS: f32 = 400.0;

// Whole-buffer normalization defaults
pub const NORMALIZE_TARGET_RMS: f32 = 0.1;
pub const NORMALIZE_NOISE_FLOOR: f32 = 0.001;
