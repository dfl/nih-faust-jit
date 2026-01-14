//! Lock-free ring buffers for transferring audio data from audio thread to GUI thread.
//!
//! Uses a split producer/consumer design where:
//! - Producer is owned by the audio thread (writes samples)
//! - Consumer is owned by the GUI thread (reads samples)

use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapCons, HeapProd, HeapRb,
};

/// Producer side of the ring buffer (audio thread writes here).
/// This is Send but not Sync - it should only be accessed from one thread.
pub struct AudioProducer {
    left: HeapProd<f32>,
    right: HeapProd<f32>,
}

// Safety: HeapProd is Send when the element type is Send
unsafe impl Send for AudioProducer {}

impl AudioProducer {
    /// Push stereo samples from the audio thread.
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        // Just push what we can, dropping oldest if buffer is full
        self.left.push_slice(left);
        self.right.push_slice(right);
    }

    /// Push from a buffer slice (common format in nih-plug).
    pub fn push_from_buffer(&mut self, buffer: &[&mut [f32]]) {
        if !buffer.is_empty() {
            self.left.push_slice(buffer[0]);
        }
        if buffer.len() > 1 {
            self.right.push_slice(buffer[1]);
        } else if !buffer.is_empty() {
            // Mono: copy left to right
            self.right.push_slice(buffer[0]);
        }
    }
}

/// Consumer side of the ring buffer (GUI thread reads from here).
/// This is Send but not Sync - it should only be accessed from one thread.
pub struct AudioConsumer {
    left: HeapCons<f32>,
    right: HeapCons<f32>,
}

// Safety: HeapCons is Send when the element type is Send
unsafe impl Send for AudioConsumer {}

impl AudioConsumer {
    /// Read all available samples into the provided vectors.
    pub fn read_available(&mut self, left_out: &mut Vec<f32>, right_out: &mut Vec<f32>) {
        left_out.clear();
        right_out.clear();

        while let Some(sample) = self.left.try_pop() {
            left_out.push(sample);
        }
        while let Some(sample) = self.right.try_pop() {
            right_out.push(sample);
        }
    }

    /// Get approximate number of samples available for reading.
    pub fn available(&self) -> usize {
        self.left.occupied_len()
    }
}

/// Create a pair of producer/consumer for stereo audio visualization.
/// Returns (producer for audio thread, consumer for GUI thread).
pub fn create_stereo_ring_buffer(capacity: usize) -> (AudioProducer, AudioConsumer) {
    let left_rb = HeapRb::<f32>::new(capacity);
    let right_rb = HeapRb::<f32>::new(capacity);

    let (left_prod, left_cons) = left_rb.split();
    let (right_prod, right_cons) = right_rb.split();

    (
        AudioProducer {
            left: left_prod,
            right: right_prod,
        },
        AudioConsumer {
            left: left_cons,
            right: right_cons,
        },
    )
}

/// Wrapper that holds the consumer side and provides a thread-safe interface.
/// This can be shared via Arc<Mutex<>> to the GUI thread.
pub struct VisualizationConsumer {
    pub consumer: AudioConsumer,
    pub enabled: bool,
}

impl VisualizationConsumer {
    pub fn new(consumer: AudioConsumer) -> Self {
        Self {
            consumer,
            enabled: true,
        }
    }
}
