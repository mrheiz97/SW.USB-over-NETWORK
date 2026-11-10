use super::*;
use std::{
    sync::{Arc, Mutex},
    thread,
};

fn read_device(
    submit: impl FnMut(Setup, u32, &mut [u8]) -> io::Result<usize> + Send + 'static,
) -> ControlDevice {
    ControlDevice::with_backend(submit)
}

impl<F> Backend for F
where
    F: FnMut(Setup, u32, &mut [u8]) -> io::Result<usize> + Send,
{
    fn submit(&mut self, setup: Setup, timeout_ms: u32, data: &mut [u8]) -> io::Result<usize> {
        self(setup, timeout_ms, data)
    }

    fn change_claim(&mut self, _: InterfaceOperation, _: u8) -> io::Result<()> {
        panic!("control reads must not change interface ownership")
    }
}

#[test]
fn reads_standard_requests_from_simulated_hid() {
    let descriptor = [18, 1, 0x10, 2, 0, 0, 0, 64, 9, 0x12, 1, 0, 0, 1, 1, 2, 3, 1];
    let cases: [(ControlRead, u8, u16, &[u8]); 3] = [
        (ControlRead::DeviceDescriptor, 6, 0x0100, &descriptor),
        (ControlRead::ConfigurationValue, 8, 0, &[1]),
        (ControlRead::DeviceStatus, 0, 0, &[0, 0]),
    ];
    for (operation, request, value, response) in cases {
        let data = execute(operation, 500, None, |setup, timeout, buffer| {
            assert_eq!(
                setup,
                Setup {
                    request_type: 0x80,
                    request,
                    value,
                    index: 0,
                    length: u16::try_from(response.len()).unwrap(),
                }
            );
            assert_eq!(timeout, 500);
            buffer.copy_from_slice(response);
            Ok(response.len())
        })
        .unwrap();
        assert_eq!(data, response);
    }
}

#[test]
fn rejects_short_oversize_and_malformed_responses() {
    for length in [0, 17, 18, 19] {
        // At the correct length the all-zero descriptor header is still invalid.
        let error = execute(ControlRead::DeviceDescriptor, 500, None, |_, _, _| {
            Ok(length)
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

// Models one USB address. Each connection has a separate lifetime, including
// when the address is immediately reused by a new connection.
#[derive(Default)]
struct SimulatedPort {
    connection: Option<(Arc<AtomicBool>, u8)>,
}

impl SimulatedPort {
    fn connect(&mut self, configuration: u8) {
        assert!(self.connection.is_none());
        self.connection = Some((Arc::new(AtomicBool::new(true)), configuration));
    }

    fn unplug(&mut self) {
        let (connected, _) = self.connection.take().unwrap();
        connected.store(false, Ordering::Relaxed);
    }

    fn open(&self) -> io::Result<ControlDevice> {
        let (connected, configuration) = self.connection.clone().ok_or(io::ErrorKind::NotFound)?;
        Ok(read_device(move |setup, timeout, data| {
            if !connected.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::NotConnected.into());
            }
            assert_eq!(setup, ControlRead::ConfigurationValue.setup());
            // A deterministic 200 ms device response, with no wall-clock sleep.
            if timeout < 200 {
                return Err(io::ErrorKind::TimedOut.into());
            }
            data.copy_from_slice(&[configuration]);
            Ok(data.len())
        }))
    }
}

#[test]
fn reconnect_at_the_same_address_requires_a_new_handle() {
    let mut port = SimulatedPort::default();
    port.connect(1);
    let mut old = port.open().unwrap();
    let read = ControlRead::ConfigurationValue;
    assert_eq!(old.read(read, 500).unwrap(), [1]);

    port.unplug();
    assert_eq!(
        old.read(read, 500).unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );
    assert!(matches!(port.open(), Err(error) if error.kind() == io::ErrorKind::NotFound));

    port.connect(2);
    assert_eq!(
        old.read(read, 500).unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );
    assert_eq!(port.open().unwrap().read(read, 500).unwrap(), [2]);
}

#[test]
fn timeout_leaves_the_handle_usable_for_a_fresh_read() {
    let mut port = SimulatedPort::default();
    port.connect(1);
    let mut device = port.open().unwrap();
    let read = ControlRead::ConfigurationValue;
    assert_eq!(
        device.read(read, 199).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(device.read(read, 200).unwrap(), [1]);
}

#[test]
fn cancellation_before_submission_skips_the_backend() {
    let signal = Cancellation::default();
    signal.cancel();
    let mut device = read_device(|_, _, _| panic!("cancelled read must not submit"));
    let result = device.read_cancellable(ControlRead::DeviceStatus, 500, &signal);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
}

#[test]
fn cancellation_during_submission_discards_the_response() {
    let signal = Arc::new(Cancellation::default());
    let in_flight = Arc::clone(&signal);
    let mut device = read_device(move |_, _, data| {
        thread::scope(|scope| scope.spawn(|| in_flight.cancel()).join().unwrap());
        data.copy_from_slice(&[1]);
        Ok(data.len())
    });
    let read = ControlRead::ConfigurationValue;
    let result = device.read_cancellable(read, 500, &signal);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(device.read(read, 500).unwrap(), [1]);
}

#[test]
fn rejects_invalid_inputs_before_submitting() {
    for timeout in [0, 5001] {
        let error = execute(ControlRead::DeviceStatus, timeout, None, |_, _, _| {
            panic!("must not submit an invalid request")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    for (bus, address) in [(0, 1), (1000, 1), (1, 0), (1, 128)] {
        assert_eq!(
            read(bus, address, ControlRead::DeviceStatus, 500)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn queue_is_bounded_and_releases_a_slot_after_running() {
    let mut port = SimulatedPort::default();
    port.connect(1);
    let mut queue = ControlQueue::new(port.open().unwrap());
    let request = ControlRead::ConfigurationValue;
    for timeout in [0, 5001] {
        assert_eq!(
            queue.enqueue(request, timeout).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    for _ in 0..ControlQueue::CAPACITY {
        queue.enqueue(request, 500).unwrap();
    }
    assert_eq!(
        queue.enqueue(request, 500).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(queue.run_next().unwrap().result.unwrap(), [1]);
    queue.enqueue(request, 500).unwrap();
    for _ in 0..ControlQueue::CAPACITY {
        assert_eq!(queue.run_next().unwrap().result.unwrap(), [1]);
    }
    assert!(queue.run_next().is_none());
}

#[test]
fn queue_preserves_order_and_skips_a_cancelled_request() {
    use ControlRead::{ConfigurationValue, DeviceDescriptor, DeviceStatus};

    let mut submitted = [DeviceDescriptor, DeviceStatus].into_iter();
    let mut queue = ControlQueue::new(read_device(move |setup, _, data| {
        assert_eq!(setup, submitted.next().unwrap().setup());
        if setup.request == 6 {
            data[..2].copy_from_slice(&[18, 1]);
        }
        Ok(data.len())
    }));
    queue.enqueue(DeviceDescriptor, 500).unwrap();
    let cancelled = queue.enqueue(ConfigurationValue, 500).unwrap();
    queue.enqueue(DeviceStatus, 500).unwrap();
    cancelled.cancel();

    let first = queue.run_next().unwrap();
    assert_eq!(first.request, DeviceDescriptor);
    assert_eq!(first.result.unwrap().len(), 18);
    let second = queue.run_next().unwrap();
    assert_eq!(second.request, ConfigurationValue);
    assert_eq!(
        second.result.unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    let third = queue.run_next().unwrap();
    assert_eq!(third.request, DeviceStatus);
    assert_eq!(third.result.unwrap(), [0, 0]);
    assert!(queue.run_next().is_none());
}

#[test]
fn queued_reads_keep_their_timeout_and_original_connection() {
    let mut port = SimulatedPort::default();
    port.connect(1);
    let mut queue = ControlQueue::new(port.open().unwrap());
    let request = ControlRead::ConfigurationValue;
    queue.enqueue(request, 199).unwrap();
    queue.enqueue(request, 500).unwrap();
    assert_eq!(
        queue.run_next().unwrap().result.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    port.unplug();
    port.connect(2);
    assert_eq!(
        queue.run_next().unwrap().result.unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );
}

#[derive(Default)]
struct SimulatedInterface {
    owner: Option<u8>,
    fail_release: bool,
    fail_seal: bool,
}

struct InterfaceHandle {
    state: Arc<Mutex<SimulatedInterface>>,
    id: u8,
    sealed: bool,
}

impl Backend for InterfaceHandle {
    fn submit(&mut self, _: Setup, _: u32, _: &mut [u8]) -> io::Result<usize> {
        panic!("claim and release must not submit control transfers")
    }

    fn change_claim(&mut self, operation: InterfaceOperation, interface: u8) -> io::Result<()> {
        assert_eq!(interface, 0);
        assert!(
            operation != InterfaceOperation::Claim || !self.sealed,
            "sealed handles must reject new claims before calling the backend"
        );
        let mut state = self.state.lock().unwrap();
        match operation {
            InterfaceOperation::Claim if state.owner.is_some() => {
                return Err(io::ErrorKind::ResourceBusy.into());
            }
            InterfaceOperation::Claim => state.owner = Some(self.id),
            InterfaceOperation::Release => {
                assert_eq!(state.owner, Some(self.id));
                if std::mem::take(&mut state.fail_release) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                state.owner = None;
            }
        }
        Ok(())
    }

    fn seal_claims(&mut self) -> io::Result<()> {
        assert!(!self.sealed, "successful sealing must not repeat the ioctl");
        if std::mem::take(&mut self.state.lock().unwrap().fail_seal) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.sealed = true;
        Ok(())
    }
}

impl Drop for InterfaceHandle {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if state.owner == Some(self.id) {
            state.owner = None;
        }
    }
}

fn open_interface(state: &Arc<Mutex<SimulatedInterface>>, id: u8) -> ControlDevice {
    ControlDevice::with_backend(InterfaceHandle {
        state: Arc::clone(state),
        id,
        sealed: false,
    })
}

#[test]
fn interface_claim_is_exclusive_and_can_be_released_or_closed() {
    let state = Arc::new(Mutex::new(SimulatedInterface::default()));
    let mut first = open_interface(&state, 1);
    let mut second = open_interface(&state, 2);

    first.claim_interface(0).unwrap();
    assert_eq!(
        first.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        second.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::ResourceBusy
    );
    assert_eq!(
        second.release_interface(0).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    first.release_interface(0).unwrap();
    second.claim_interface(0).unwrap();
    drop(second);
    first.claim_interface(0).unwrap();
    first.release_interface(0).unwrap();
}

#[test]
fn interface_busy_preserves_the_existing_driver() {
    let state = Arc::new(Mutex::new(SimulatedInterface {
        owner: Some(0), // owner 0 is the simulated kernel driver
        ..SimulatedInterface::default()
    }));
    let mut device = open_interface(&state, 1);
    assert_eq!(
        device.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::ResourceBusy
    );
    assert_eq!(
        device.release_interface(0).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    drop(device);
    assert_eq!(state.lock().unwrap().owner, Some(0));
}

#[test]
fn failed_interface_release_keeps_the_claim_for_retry() {
    let state = Arc::new(Mutex::new(SimulatedInterface {
        fail_release: true,
        ..SimulatedInterface::default()
    }));
    let mut device = open_interface(&state, 1);
    device.claim_interface(0).unwrap();
    assert_eq!(
        device.release_interface(0).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        device.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(state.lock().unwrap().owner, Some(1));
    device.release_interface(0).unwrap();
    assert_eq!(state.lock().unwrap().owner, None);
}

#[test]
fn sealing_keeps_existing_claims_and_prevents_reacquisition() {
    let state = Arc::new(Mutex::new(SimulatedInterface::default()));
    let mut sealed = open_interface(&state, 1);
    let mut other = open_interface(&state, 2);
    sealed.claim_interface(0).unwrap();
    sealed.seal_claims().unwrap();
    sealed.seal_claims().unwrap();
    assert_eq!(state.lock().unwrap().owner, Some(1));
    assert_eq!(
        other.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::ResourceBusy
    );
    sealed.release_interface(0).unwrap();
    assert_eq!(
        sealed.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    other.claim_interface(0).unwrap();
    other.release_interface(0).unwrap();
}

#[test]
fn a_failed_seal_can_be_retried_and_does_not_change_claims() {
    let state = Arc::new(Mutex::new(SimulatedInterface {
        fail_seal: true,
        ..SimulatedInterface::default()
    }));
    let mut device = open_interface(&state, 1);
    assert_eq!(
        device.seal_claims().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    device.claim_interface(0).unwrap();
    device.seal_claims().unwrap();
    assert_eq!(state.lock().unwrap().owner, Some(1));
    device.release_interface(0).unwrap();
    assert_eq!(
        device.claim_interface(0).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}
