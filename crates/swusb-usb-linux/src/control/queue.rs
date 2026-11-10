use std::{collections::VecDeque, io, sync::Arc};

use super::{Cancellation, ControlDevice, ControlRead, validate_timeout};

struct PendingRead {
    request: ControlRead,
    timeout_ms: u32,
    cancellation: Arc<Cancellation>,
}

/// The result of one dequeued control read, including cancelled and failed reads.
#[derive(Debug)]
pub struct ControlCompletion {
    pub request: ControlRead,
    pub result: io::Result<Vec<u8>>,
}

/// A bounded FIFO queue attached to one USB connection.
///
/// The caller advances the queue one request at a time. Reconnecting requires
/// a new device handle and queue. Queued reads cannot migrate to a new device.
pub struct ControlQueue {
    device: ControlDevice,
    pending: VecDeque<PendingRead>,
}

impl ControlQueue {
    /// Maximum number of pending reads per connection.
    pub const CAPACITY: usize = 16;

    /// Creates an empty queue for an open device connection.
    #[must_use]
    pub fn new(device: ControlDevice) -> Self {
        Self {
            device,
            pending: VecDeque::with_capacity(Self::CAPACITY),
        }
    }

    /// Enqueues a standard read and returns its independent cancellation signal.
    ///
    /// The timeout applies to the USB transfer, excluding time spent in the
    /// queue. Cancelled reads occupy a slot until dequeued.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` for a timeout outside 1–5000 ms or `WouldBlock`
    /// if the queue is full.
    pub fn enqueue(
        &mut self,
        request: ControlRead,
        timeout_ms: u32,
    ) -> io::Result<Arc<Cancellation>> {
        validate_timeout(timeout_ms)?;
        if self.pending.len() >= Self::CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "USB control read queue is full",
            ));
        }
        let cancellation = Arc::new(Cancellation::default());
        self.pending.push_back(PendingRead {
            request,
            timeout_ms,
            cancellation: Arc::clone(&cancellation),
        });
        Ok(cancellation)
    }

    /// Executes the oldest request and frees its slot, or returns `None` if empty.
    pub fn run_next(&mut self) -> Option<ControlCompletion> {
        let pending = self.pending.pop_front()?;
        Some(ControlCompletion {
            request: pending.request,
            result: self.device.read_cancellable(
                pending.request,
                pending.timeout_ms,
                &pending.cancellation,
            ),
        })
    }
}
