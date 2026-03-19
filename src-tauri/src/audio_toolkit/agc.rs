/// Automatic Gain Control for the audio pipeline.
///
/// Two stages:
/// 1. `StreamingAgc` — per-frame envelope follower applied before VAD and
///    transcription buffer so that quiet/whispered speech is not rejected.
/// 2. `normalize_buffer` — single-pass RMS normalization of the complete
///    utterance before it is sent to the transcription model.
use crate::audio_toolkit::constants;

/// Per-frame streaming AGC with asymmetric attack/release envelope.
pub struct StreamingAgc {
    target_rms: f32,
    max_gain: f32,
    noise_floor: f32,
    attack_alpha: f32,
    release_alpha: f32,
    smoothed_rms: f32,
}

impl StreamingAgc {
    pub fn new(
        target_rms: f32,
        max_gain: f32,
        noise_floor: f32,
        attack_ms: f32,
        release_ms: f32,
        frame_ms: f32,
    ) -> Self {
        Self {
            target_rms,
            max_gain,
            noise_floor,
            attack_alpha: 1.0 - (-frame_ms / attack_ms).exp(),
            release_alpha: 1.0 - (-frame_ms / release_ms).exp(),
            // Start at target so the AGC begins at unity gain rather than
            // spiking to max_gain on the first frame.
            smoothed_rms: target_rms,
        }
    }

    /// Create a `StreamingAgc` from the constants defined in [`constants`].
    pub fn from_defaults() -> Self {
        Self::new(
            constants::AGC_TARGET_RMS,
            constants::AGC_MAX_GAIN,
            constants::AGC_NOISE_FLOOR,
            constants::AGC_ATTACK_MS,
            constants::AGC_RELEASE_MS,
            constants::AGC_FRAME_MS,
        )
    }

    /// Apply gain to `frame` in-place.  Returns the gain that was applied.
    pub fn process_frame(&mut self, frame: &mut [f32]) -> f32 {
        let rms = frame_rms(frame);

        // Update the smoothed envelope (asymmetric EMA).
        let alpha = if rms > self.smoothed_rms {
            self.attack_alpha
        } else {
            self.release_alpha
        };
        self.smoothed_rms += alpha * (rms - self.smoothed_rms);

        // Derive gain.
        let gain = if self.smoothed_rms < self.noise_floor {
            1.0 // don't amplify silence / noise floor
        } else {
            (self.target_rms / self.smoothed_rms).clamp(1.0, self.max_gain)
        };

        if gain != 1.0 {
            for s in frame.iter_mut() {
                *s = (*s * gain).clamp(-1.0, 1.0);
            }
        }

        gain
    }

    pub fn reset(&mut self) {
        self.smoothed_rms = self.target_rms;
    }
}

/// Single-pass RMS normalization of a complete audio buffer.
///
/// Scales the buffer so its RMS matches `target_rms`.  Unlike the streaming
/// AGC (which is boost-only), this function both amplifies quiet audio and
/// attenuates loud audio.  This is intentional: the transcription model
/// benefits from a consistent input level regardless of speaker volume.
///
/// If the buffer's RMS is below `noise_floor` the buffer is returned
/// unchanged (avoids amplifying pure noise).  Samples are clamped to
/// [-1.0, 1.0] after scaling.
pub fn normalize_buffer(samples: &mut [f32], target_rms: f32, noise_floor: f32) {
    if samples.is_empty() {
        return;
    }

    let rms = frame_rms(samples);
    if rms < noise_floor {
        return;
    }

    let gain = target_rms / rms;
    // No point scaling by ~1.0.
    if (gain - 1.0).abs() < 1e-6 {
        return;
    }

    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

/// RMS (root mean square) of a sample buffer.
///
/// Uses f64 accumulation to avoid precision loss on large buffers
/// (e.g. 60 s at 16 kHz = 960 k samples).
#[inline]
fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_frame(
        frequency: f32,
        amplitude: f32,
        sample_rate: u32,
        num_samples: usize,
    ) -> Vec<f32> {
        (0..num_samples)
            .map(|i| amplitude * (2.0 * PI * frequency * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn streaming_agc_converges_to_target() {
        let mut agc = StreamingAgc::from_defaults();
        let target = constants::AGC_TARGET_RMS;

        // Feed 100 frames of quiet sine wave (amplitude 0.01 → RMS ≈ 0.007)
        for _ in 0..100 {
            let mut frame = sine_frame(440.0, 0.01, 16000, 480);
            agc.process_frame(&mut frame);
        }

        // After convergence the output RMS should be close to target.
        let mut frame = sine_frame(440.0, 0.01, 16000, 480);
        agc.process_frame(&mut frame);
        let out_rms = frame_rms(&frame);
        assert!(
            (out_rms - target).abs() < 0.03,
            "Expected RMS near {target}, got {out_rms}"
        );
    }

    #[test]
    fn streaming_agc_silence_passthrough() {
        let mut agc = StreamingAgc::from_defaults();
        let mut frame = vec![0.0001; 480]; // below noise floor
        let gain = agc.process_frame(&mut frame);
        assert!(
            (gain - 1.0).abs() < 1e-6,
            "Gain on silence should be 1.0, got {gain}"
        );
    }

    #[test]
    fn streaming_agc_does_not_attenuate() {
        // Loud signal (RMS already above target) — gain should stay at 1.0.
        let mut agc = StreamingAgc::from_defaults();
        for _ in 0..50 {
            let mut frame = sine_frame(440.0, 0.5, 16000, 480);
            let gain = agc.process_frame(&mut frame);
            assert!(
                gain >= 1.0 - 1e-6,
                "AGC should never attenuate, got gain {gain}"
            );
        }
    }

    #[test]
    fn streaming_agc_ramps_on_level_drop() {
        let mut agc = StreamingAgc::from_defaults();

        // Establish envelope with loud signal.
        for _ in 0..50 {
            let mut frame = sine_frame(440.0, 0.3, 16000, 480);
            agc.process_frame(&mut frame);
        }

        // Drop to whisper level — gain should gradually increase.
        let mut gains = Vec::new();
        for _ in 0..50 {
            let mut frame = sine_frame(440.0, 0.005, 16000, 480);
            let g = agc.process_frame(&mut frame);
            gains.push(g);
        }

        // Gains should be monotonically non-decreasing (release ramp).
        for window in gains.windows(2) {
            assert!(
                window[1] >= window[0] - 1e-6,
                "Gain should ramp up, got {} then {}",
                window[0],
                window[1]
            );
        }
        // Final gain should be substantially above 1.
        assert!(
            *gains.last().unwrap() > 5.0,
            "Expected significant gain after level drop, got {}",
            gains.last().unwrap()
        );
    }

    #[test]
    fn streaming_agc_reset_restores_initial_state() {
        let mut agc = StreamingAgc::from_defaults();
        let initial_rms = agc.smoothed_rms;
        for _ in 0..50 {
            let mut frame = sine_frame(440.0, 0.3, 16000, 480);
            agc.process_frame(&mut frame);
        }
        agc.reset();
        assert!(
            (agc.smoothed_rms - initial_rms).abs() < 1e-9,
            "smoothed_rms should be restored after reset"
        );
    }

    #[test]
    fn streaming_agc_cold_start_no_spike() {
        // First frame on a fresh AGC should not get max gain.
        let mut agc = StreamingAgc::from_defaults();
        let mut frame = sine_frame(440.0, 0.05, 16000, 480);
        let gain = agc.process_frame(&mut frame);
        assert!(gain < 5.0, "Cold-start gain should be moderate, got {gain}");
    }

    #[test]
    fn normalize_buffer_scales_to_target() {
        let target = 0.1;
        let mut buf = sine_frame(440.0, 0.01, 16000, 16000);
        normalize_buffer(&mut buf, target, 0.001);
        let rms = frame_rms(&buf);
        assert!(
            (rms - target).abs() < 0.01,
            "Expected RMS near {target}, got {rms}"
        );
    }

    #[test]
    fn normalize_buffer_attenuates_loud_input() {
        let target = 0.1;
        // Loud signal (amplitude 0.8, RMS ≈ 0.57) should be attenuated.
        let mut buf = sine_frame(440.0, 0.8, 16000, 16000);
        let original_rms = frame_rms(&buf);
        assert!(original_rms > target, "precondition: input should be loud");
        normalize_buffer(&mut buf, target, 0.001);
        let rms = frame_rms(&buf);
        assert!(
            (rms - target).abs() < 0.01,
            "Expected RMS near {target}, got {rms}"
        );
        assert!(rms < original_rms, "should have attenuated");
    }

    #[test]
    fn normalize_buffer_skips_silence() {
        let mut buf = vec![0.0001; 480];
        let original = buf.clone();
        normalize_buffer(&mut buf, 0.1, 0.001);
        assert_eq!(buf, original, "Silent buffer should be unchanged");
    }

    #[test]
    fn normalize_buffer_clamps() {
        // Very quiet signal normalized to high target — samples should clamp.
        let mut buf = sine_frame(440.0, 0.001, 16000, 480);
        normalize_buffer(&mut buf, 0.5, 0.0001);
        for &s in &buf {
            assert!(s >= -1.0 && s <= 1.0, "Sample {s} out of [-1, 1] range");
        }
    }

    #[test]
    fn normalize_buffer_empty() {
        let mut buf: Vec<f32> = vec![];
        normalize_buffer(&mut buf, 0.1, 0.001); // should not panic
    }
}
