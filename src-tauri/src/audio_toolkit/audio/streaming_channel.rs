use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use log::warn;
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
            Err(TrySendError::Disconnected(_)) => {}
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
