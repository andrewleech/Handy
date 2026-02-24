use handy_keys::KeyboardListener;
use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// Grace period after listener starts, during which key events are ignored.
/// This allows the hotkey release (e.g. Ctrl+Space up) to complete
/// without immediately triggering stop.
const GRACE_PERIOD: Duration = Duration::from_millis(500);

/// Returns true if this event should stop recording.
///
/// Only physical key-down events with a non-modifier key (i.e. `key: Some(...)`)
/// that arrive after the grace period trigger a stop.
fn should_stop_recording(event: &handy_keys::KeyEvent, elapsed: Duration) -> bool {
    event.is_key_down && event.key.is_some() && elapsed >= GRACE_PERIOD
}

/// Managed Tauri state wrapping the optional listener.
pub struct LiveTypingListenerState(pub Mutex<Option<LiveTypingStopListener>>);

pub struct LiveTypingStopListener {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LiveTypingStopListener {
    /// Spawn a listener that detects any physical key-down and triggers stop.
    ///
    /// `enigo.text()` sends VK_PACKET (0xE7) events on Windows.
    /// `handy_keys::vk_to_key()` returns `None` for VK_PACKET, so these
    /// events are silently dropped by the keyboard hook — the listener
    /// never sees them. Only physical key presses (with real VK codes)
    /// produce `KeyEvent`s with `key: Some(...)`.
    pub fn spawn(app: AppHandle, binding_id: String) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();

        let handle = std::thread::spawn(move || {
            let listener = match KeyboardListener::new() {
                Ok(l) => l,
                Err(e) => {
                    error!("Failed to create stop-detection listener: {}", e);
                    return;
                }
            };

            let start = Instant::now();

            while thread_running.load(Ordering::Relaxed) {
                match listener.recv_timeout(Duration::from_millis(50)) {
                    Ok(event) => {
                        if !should_stop_recording(&event, start.elapsed()) {
                            continue;
                        }
                        info!("Live typing stop: detected key {:?}", event.key);
                        if let Some(coordinator) =
                            app.try_state::<crate::TranscriptionCoordinator>()
                        {
                            // Simulate a toggle press to stop recording
                            coordinator.send_input(&binding_id, "", true, false);
                        }
                        break;
                    }
                    Err(_) => continue,
                }
            }
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for LiveTypingStopListener {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handy_keys::{Key, Modifiers};

    fn make_event(is_key_down: bool, key: Option<Key>) -> handy_keys::KeyEvent {
        handy_keys::KeyEvent {
            modifiers: Modifiers::empty(),
            key,
            is_key_down,
            changed_modifier: None,
        }
    }

    #[test]
    fn ignores_key_up() {
        let event = make_event(false, Some(Key::A));
        assert!(!should_stop_recording(&event, Duration::from_secs(1)));
    }

    #[test]
    fn ignores_modifier_only() {
        let event = make_event(true, None);
        assert!(!should_stop_recording(&event, Duration::from_secs(1)));
    }

    #[test]
    fn ignores_during_grace_period() {
        let event = make_event(true, Some(Key::A));
        assert!(!should_stop_recording(&event, Duration::from_millis(200)));
    }

    #[test]
    fn triggers_on_physical_key_down_after_grace() {
        let event = make_event(true, Some(Key::A));
        assert!(should_stop_recording(&event, Duration::from_secs(1)));
    }

    #[test]
    fn triggers_exactly_at_grace_boundary() {
        let event = make_event(true, Some(Key::Escape));
        assert!(should_stop_recording(&event, GRACE_PERIOD));
    }

    #[test]
    fn ignores_modifier_change_event() {
        // Modifier-change events have key: None and changed_modifier populated.
        // They should not trigger stop since no physical key was pressed.
        let event = handy_keys::KeyEvent {
            modifiers: Modifiers::SHIFT,
            key: None,
            is_key_down: true,
            changed_modifier: Some(Modifiers::SHIFT),
        };
        assert!(!should_stop_recording(&event, Duration::from_secs(1)));
    }
}
