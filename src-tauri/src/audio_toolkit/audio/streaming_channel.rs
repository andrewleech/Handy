use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use log::{debug, warn};
use std::sync::atomic::{AtomicU64, Ordering};

const CHANNEL_CAPACITY: usize = 200;

pub struct StreamingAudioChannel {
    sender: Sender<Vec<f32>>,
    receiver: Receiver<Vec<f32>>,
    dropped_frames: AtomicU64,
}

impl StreamingAudioChannel {
    pub fn new() -> Self {
        let (sender, receiver) = bounded(CHANNEL_CAPACITY);
        Self {
            sender,
            receiver,
            dropped_frames: AtomicU64::new(0),
        }
    }

    /// Non-blocking send. Drops the frame if the channel is full.
    pub fn try_send(&self, samples: Vec<f32>) {
        match self.sender.try_send(samples) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let count = self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                if count % 100 == 0 {
                    warn!("Streaming channel full, dropped {} frames total", count + 1);
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                debug!("Streaming channel disconnected, receiver dropped");
            }
        }
    }

    pub fn receiver(&self) -> &Receiver<Vec<f32>> {
        &self.receiver
    }
}

impl Default for StreamingAudioChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_receive() {
        let ch = StreamingAudioChannel::new();
        ch.try_send(vec![1.0, 2.0]);
        let received = ch.receiver().try_recv().unwrap();
        assert_eq!(received, vec![1.0, 2.0]);
    }

    #[test]
    fn drops_when_full() {
        let ch = StreamingAudioChannel::new();
        for _ in 0..CHANNEL_CAPACITY {
            ch.try_send(vec![0.0]);
        }
        // Channel is full; next send should drop without panic
        ch.try_send(vec![1.0]);
        assert_eq!(ch.dropped_frames.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn disconnect_does_not_panic() {
        // We can't selectively drop the receiver from StreamingAudioChannel,
        // so test the disconnect path using a raw crossbeam channel.
        let (sender, receiver) = bounded::<Vec<f32>>(1);
        drop(receiver);
        assert!(matches!(
            sender.try_send(vec![0.0]),
            Err(TrySendError::Disconnected(_))
        ));
    }
}
