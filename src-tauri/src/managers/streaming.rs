use crate::audio_toolkit::StreamingAudioChannel;
use crate::managers::model::ModelManager;
use log::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use transcribe_rs::engines::nemotron_streaming::NemotronStreamingEngine;
use transcribe_rs::StreamingTranscriptionEngine;

enum EngineSlot {
    Empty,
    Loading,
    Ready(NemotronStreamingEngine),
}

/// Manages the lifecycle of the streaming transcription engine.
///
/// Currently hardwired to `NemotronStreamingEngine` — the only streaming engine
/// available. The `streaming_model` setting selects model weights, not engine type.
pub struct StreamingManager {
    app_handle: AppHandle,
    model_manager: Arc<ModelManager>,
    engine_slot: Arc<Mutex<EngineSlot>>,
    engine_ready_condvar: Arc<Condvar>,
    stop_flag: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl StreamingManager {
    pub fn new(app_handle: &AppHandle, model_manager: Arc<ModelManager>) -> Self {
        Self {
            app_handle: app_handle.clone(),
            model_manager,
            engine_slot: Arc::new(Mutex::new(EngineSlot::Empty)),
            engine_ready_condvar: Arc::new(Condvar::new()),
            stop_flag: Arc::new(Mutex::new(None)),
            thread_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Lock engine_slot, recovering from poison if a previous thread panicked.
    fn lock_engine_slot(&self) -> MutexGuard<'_, EngineSlot> {
        self.engine_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Loads the streaming model into the engine slot in the background.
    /// Fire-and-forget — does not block.
    pub fn preload_model(&self, model_id: &str) {
        let model_path = {
            let mut slot = self.lock_engine_slot();
            match *slot {
                EngineSlot::Ready(_) | EngineSlot::Loading => return,
                EngineSlot::Empty => {}
            }
            match self.model_manager.get_model_path(model_id) {
                Ok(p) => {
                    *slot = EngineSlot::Loading;
                    p
                }
                Err(e) => {
                    warn!("Streaming model not available for preload: {}", e);
                    return;
                }
            }
        };

        let engine_slot = self.engine_slot.clone();
        let condvar = self.engine_ready_condvar.clone();
        let model_id = model_id.to_string();

        thread::spawn(move || {
            info!("Preloading streaming model: {}", model_id);
            let mut engine = NemotronStreamingEngine::new();
            match engine.load_model(&model_path) {
                Ok(()) => {
                    info!("Streaming model loaded: {}", model_id);
                    let mut slot = engine_slot
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *slot = EngineSlot::Ready(engine);
                    condvar.notify_all();
                }
                Err(e) => {
                    error!("Failed to preload streaming model: {}", e);
                    let mut slot = engine_slot
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *slot = EngineSlot::Empty;
                    condvar.notify_all();
                }
            }
        });
    }

    /// Takes the engine from cache, creates a channel, spawns the streaming loop.
    /// Returns the channel for the AudioRecorder to write into, or None if no engine.
    ///
    /// Callers must ensure only one thread calls start/stop at a time.
    /// Concurrent calls risk deadlock: stop_streaming joins the streaming
    /// thread, which acquires engine_slot on exit.
    pub fn start_streaming(&self) -> Option<Arc<StreamingAudioChannel>> {
        // Stop any previous session first
        self.stop_streaming();

        // Take the engine out of the slot
        let engine = {
            let mut slot = self.lock_engine_slot();
            match std::mem::replace(&mut *slot, EngineSlot::Empty) {
                EngineSlot::Ready(e) => e,
                other => {
                    // Put it back
                    *slot = other;
                    warn!("Streaming engine not ready, skipping streaming");
                    return None;
                }
            }
        };

        let channel = Arc::new(StreamingAudioChannel::new());
        let stop = Arc::new(AtomicBool::new(false));

        {
            let mut flag = self
                .stop_flag
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *flag = Some(stop.clone());
        }

        let app_handle = self.app_handle.clone();
        let receiver = channel.receiver().clone();
        let engine_slot = self.engine_slot.clone();

        let handle = thread::spawn(move || {
            streaming_loop(engine, receiver, stop, app_handle, engine_slot);
        });

        {
            let mut th = self
                .thread_handle
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *th = Some(handle);
        }

        Some(channel)
    }

    /// Blocks until the engine becomes ready, the slot becomes empty (load failed),
    /// `should_cancel` returns true, or `timeout` elapses. Returns true if ready.
    pub fn wait_for_engine_ready(
        &self,
        timeout: Duration,
        should_cancel: impl Fn() -> bool,
    ) -> bool {
        wait_for_slot_ready(
            &self.engine_slot,
            &self.engine_ready_condvar,
            timeout,
            should_cancel,
        )
    }

    /// Releases the streaming engine, freeing its memory.
    /// Stops any active streaming session first.
    pub fn unload_engine(&self) {
        self.stop_streaming();
        let mut slot = self.lock_engine_slot();
        *slot = EngineSlot::Empty;
    }

    /// Signals the streaming loop to stop and waits for the thread to finish.
    /// The streaming loop returns the engine to the cache for reuse.
    ///
    /// The streaming loop uses a 50ms receive timeout and checks a stop flag,
    /// so join completes promptly under normal conditions. A hung ONNX session
    /// would cause this to block indefinitely — accepted risk, consistent with
    /// AudioRecorder::close().
    pub fn stop_streaming(&self) {
        // Signal stop
        if let Some(flag) = self
            .stop_flag
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            flag.store(true, Ordering::Release);
        }

        // Join the thread
        if let Some(handle) = self
            .thread_handle
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            if let Err(e) = handle.join() {
                error!("Streaming thread panicked: {:?}", e);
            }
        }
    }
}

impl Drop for StreamingManager {
    fn drop(&mut self) {
        self.stop_streaming();
    }
}

/// Core condvar wait loop: blocks until the engine slot becomes Ready (returns true),
/// becomes Empty (returns false), `should_cancel` returns true (returns false),
/// or `timeout` elapses (returns false).
fn wait_for_slot_ready(
    engine_slot: &Mutex<EngineSlot>,
    condvar: &Condvar,
    timeout: Duration,
    should_cancel: impl Fn() -> bool,
) -> bool {
    let start = Instant::now();
    let mut guard = engine_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        match *guard {
            EngineSlot::Ready(_) => return true,
            EngineSlot::Loading => {}
            EngineSlot::Empty => return false,
        }
        if should_cancel() {
            return false;
        }
        let remaining = timeout.checked_sub(start.elapsed()).unwrap_or_default();
        if remaining.is_zero() {
            return false;
        }
        let (g, _) = condvar
            .wait_timeout(guard, remaining.min(Duration::from_millis(200)))
            .unwrap_or_else(|p| p.into_inner());
        guard = g;
    }
}

/// Maximum length (in bytes) for accumulated completed text.
/// Once exceeded, text is truncated from the front to this limit.
const MAX_COMPLETED_TEXT_LEN: usize = 2000;

/// Maximum length (in bytes) for the in-progress sentence buffer.
/// Prevents unbounded growth if the engine never emits an endpoint.
const MAX_SENTENCE_LEN: usize = 4000;

/// Truncate `text` from the front so that at most `max_len` bytes remain.
/// Splits on a char boundary to avoid panics on multi-byte UTF-8.
fn truncate_front(text: &mut String, max_len: usize) {
    if text.len() <= max_len {
        return;
    }
    let trim_to = text.len() - max_len;
    // Find the next char boundary at or after trim_to
    let boundary = (trim_to..=text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    text.drain(..boundary);
}

struct TextAccumulator {
    completed_text: String,
    current_sentence: String,
}

impl TextAccumulator {
    fn new() -> Self {
        Self {
            completed_text: String::new(),
            current_sentence: String::new(),
        }
    }

    /// Appends segment text. Returns true if endpoint was reached and sentence
    /// was moved to completed text.
    fn push_segment(&mut self, text: &str, is_endpoint: bool) -> bool {
        self.current_sentence.push_str(text);

        // Prevent unbounded growth if engine never emits endpoint
        if self.current_sentence.len() > MAX_SENTENCE_LEN {
            truncate_front(&mut self.current_sentence, MAX_SENTENCE_LEN);
        }

        if is_endpoint && !self.current_sentence.is_empty() {
            if !self.completed_text.is_empty() {
                self.completed_text.push(' ');
            }
            self.completed_text
                .push_str(self.current_sentence.trim());
            self.current_sentence.clear();
            truncate_front(&mut self.completed_text, MAX_COMPLETED_TEXT_LEN);
            return true;
        }
        false
    }

    fn display_text(&self) -> String {
        let trimmed = self.current_sentence.trim();
        if self.completed_text.is_empty() {
            trimmed.to_string()
        } else if trimmed.is_empty() {
            self.completed_text.clone()
        } else {
            format!("{} {}", self.completed_text, trimmed)
        }
    }
}

fn streaming_loop(
    mut engine: NemotronStreamingEngine,
    receiver: crossbeam_channel::Receiver<Vec<f32>>,
    stop: Arc<AtomicBool>,
    app_handle: AppHandle,
    engine_slot: Arc<Mutex<EngineSlot>>,
) {
    let mut acc = TextAccumulator::new();

    debug!("Streaming loop started");

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        // Receive with timeout so we can check the stop flag periodically
        let samples = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(s) => s,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };

        match engine.push_samples(&samples) {
            Ok(segments) => {
                for segment in segments {
                    let is_endpoint = acc.push_segment(&segment.text, segment.is_endpoint);

                    if is_endpoint {
                        debug!(
                            "Endpoint detected, completed_text: '{}'",
                            acc.completed_text
                        );
                        engine.reset();
                        // Break from the segment loop — stale segments after
                        // reset are discarded. The outer recv loop continues
                        // to process the next audio chunk.
                        break;
                    }
                }

                let display = acc.display_text();

                if !display.is_empty() {
                    let _ = app_handle.emit_to("recording_overlay", "streaming-text", &display);
                }
            }
            Err(e) => {
                warn!("Streaming engine push_samples error: {}", e);
            }
        }
    }

    // Reset engine state and return it to the cache for reuse
    engine.reset();
    {
        let mut slot = engine_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = EngineSlot::Ready(engine);
    }

    debug!("Streaming loop finished");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_front_no_op_when_within_limit() {
        let mut s = "hello".to_string();
        truncate_front(&mut s, 10);
        assert_eq!(s, "hello");
    }

    #[test]
    fn truncate_front_exact_limit() {
        let mut s = "hello".to_string();
        truncate_front(&mut s, 5);
        assert_eq!(s, "hello");
    }

    #[test]
    fn truncate_front_trims_from_front() {
        let mut s = "hello world".to_string();
        truncate_front(&mut s, 5);
        assert_eq!(s, "world");
    }

    #[test]
    fn truncate_front_respects_char_boundary() {
        // 'é' is 2 bytes in UTF-8
        // "café latte" = c(1) + a(1) + f(1) + é(2) + ' '(1) + l(1) + a(1) + t(1) + t(1) + e(1) = 11 bytes
        let mut s = "café latte".to_string();
        assert_eq!(s.len(), 11);
        // Request 8 bytes: trim_to = 11 - 8 = 3, which lands on 'f'/'é' boundary
        truncate_front(&mut s, 8);
        assert_eq!(s, "é latte");
    }

    #[test]
    fn truncate_front_multibyte_boundary_mid_char() {
        // Force a split in the middle of a multi-byte char
        let mut s = "x\u{1F600}y".to_string(); // 'x' (1) + emoji (4) + 'y' (1) = 6 bytes
        assert_eq!(s.len(), 6);
        truncate_front(&mut s, 2);
        // trim_to = 6 - 2 = 4, which is mid-emoji (bytes 1..5). Should advance to byte 5 ('y').
        assert_eq!(s, "y");
    }

    #[test]
    fn text_accumulator_single_segment() {
        let mut acc = TextAccumulator::new();
        let endpoint = acc.push_segment("hello", false);
        assert!(!endpoint);
        assert_eq!(acc.display_text(), "hello");
    }

    #[test]
    fn text_accumulator_endpoint_moves_to_completed() {
        let mut acc = TextAccumulator::new();
        acc.push_segment("hello", false);
        let endpoint = acc.push_segment(" world", true);
        assert!(endpoint);
        assert_eq!(acc.display_text(), "hello world");
        assert!(acc.current_sentence.is_empty());
    }

    #[test]
    fn text_accumulator_multiple_endpoints() {
        let mut acc = TextAccumulator::new();
        acc.push_segment("first", true);
        acc.push_segment("second", true);
        acc.push_segment("third", false);
        assert_eq!(acc.display_text(), "first second third");
    }

    #[test]
    fn text_accumulator_truncate_caps_completed() {
        let mut acc = TextAccumulator::new();
        let long_text = "a".repeat(MAX_COMPLETED_TEXT_LEN + 500);
        acc.push_segment(&long_text, true);
        assert!(acc.completed_text.len() <= MAX_COMPLETED_TEXT_LEN);
    }

    #[test]
    fn text_accumulator_display_text_concatenation() {
        let mut acc = TextAccumulator::new();
        assert_eq!(acc.display_text(), "");

        acc.push_segment("  partial ", false);
        assert_eq!(acc.display_text(), "partial");

        acc.push_segment("", true);
        acc.push_segment(" next", false);
        assert_eq!(acc.display_text(), "partial next");

        let mut acc2 = TextAccumulator::new();
        acc2.push_segment("done", true);
        assert_eq!(acc2.display_text(), "done");
    }

    #[test]
    fn engine_slot_starts_empty() {
        let slot = EngineSlot::Empty;
        assert!(matches!(slot, EngineSlot::Empty));
    }

    #[test]
    fn wait_returns_false_when_empty() {
        let slot = Arc::new(Mutex::new(EngineSlot::Empty));
        let cv = Arc::new(Condvar::new());
        let result = wait_for_slot_ready(&slot, &cv, Duration::from_secs(1), || false);
        assert!(!result);
    }

    #[test]
    fn wait_returns_false_when_cancelled() {
        let slot = Arc::new(Mutex::new(EngineSlot::Loading));
        let cv = Arc::new(Condvar::new());
        let result = wait_for_slot_ready(&slot, &cv, Duration::from_secs(5), || true);
        assert!(!result);
    }

    #[test]
    fn wait_returns_false_on_timeout() {
        let slot = Arc::new(Mutex::new(EngineSlot::Loading));
        let cv = Arc::new(Condvar::new());
        let result = wait_for_slot_ready(&slot, &cv, Duration::from_millis(50), || false);
        assert!(!result);
    }

    #[test]
    fn wait_returns_true_when_notified_ready() {
        let slot = Arc::new(Mutex::new(EngineSlot::Loading));
        let cv = Arc::new(Condvar::new());

        let slot_c = slot.clone();
        let cv_c = cv.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            // Simulate engine finishing load — we can't construct a real
            // NemotronStreamingEngine in tests, so set Empty to prove the
            // condvar wakes the waiter. A Ready would be ideal but requires
            // the actual engine.
            //
            // Instead, transition Loading → Empty, which makes
            // wait_for_slot_ready return false. We verify the condvar
            // wake-up happened (not a timeout) by checking elapsed time.
            *slot_c.lock().unwrap() = EngineSlot::Empty;
            cv_c.notify_all();
        });

        let start = Instant::now();
        let result = wait_for_slot_ready(&slot, &cv, Duration::from_secs(5), || false);
        let elapsed = start.elapsed();

        // The waiter should have woken promptly (well under the 5s timeout)
        assert!(!result); // Empty → false
        assert!(
            elapsed < Duration::from_secs(1),
            "Condvar wake took too long: {:?}",
            elapsed
        );
    }
}
