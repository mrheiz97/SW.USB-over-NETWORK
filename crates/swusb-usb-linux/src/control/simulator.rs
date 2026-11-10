use std::{collections::VecDeque, io};
use swusb_core::UsbDevice;

use super::{
    Backend, Cancellation, ControlDevice, InterfaceOperation, PreparedEndpointRead, Setup,
};
use crate::endpoint::plan_read;

struct Input {
    endpoint: u8,
    bytes: Vec<u8>,
    response_after_ms: u32,
}

/// One offline USB connection with scripted Bulk and Interrupt IN responses.
///
/// Responses are consumed in order per endpoint. The simulator uses a
/// deterministic response latency per read attempt and never waits in real time.
pub struct SimulatedDevice {
    inventory: UsbDevice,
    connection: ControlDevice,
    inputs: VecDeque<Input>,
}

impl SimulatedDevice {
    /// Maximum number of scripted responses buffered on this connection.
    pub const INPUT_CAPACITY: usize = 16;

    /// Creates a connection from an active interface snapshot.
    ///
    /// # Errors
    ///
    /// Returns `InvalidData` if an interface appears more than once.
    pub fn new(inventory: UsbDevice) -> io::Result<Self> {
        let mut present = [false; 256];
        for interface in &inventory.interfaces {
            if std::mem::replace(&mut present[usize::from(interface.number)], true) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate active interface in simulator inventory",
                ));
            }
        }
        Ok(Self {
            inventory,
            connection: ControlDevice::with_backend(SimulatedBackend { present }),
            inputs: VecDeque::new(),
        })
    }

    /// Claims one interface in the simulator.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for an absent interface or `AlreadyExists` for a
    /// duplicate claim.
    pub fn claim_interface(&mut self, interface: u8) -> io::Result<()> {
        self.connection.claim_interface(interface)
    }

    /// Releases one interface in the simulator.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` if this connection has no claim on the interface.
    pub fn release_interface(&mut self, interface: u8) -> io::Result<()> {
        self.connection.release_interface(interface)
    }

    /// Queues one scripted response, including a possible zero-length packet.
    ///
    /// # Errors
    ///
    /// Returns an endpoint planning error for invalid input or `WouldBlock`
    /// when all response slots are occupied.
    pub fn queue_input(
        &mut self,
        endpoint: u8,
        bytes: &[u8],
        response_after_ms: u32,
    ) -> io::Result<()> {
        plan_read(&self.inventory, endpoint, bytes.len().max(1))?;
        if self.inputs.len() == Self::INPUT_CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "simulator input queue is full",
            ));
        }
        self.inputs.push_back(Input {
            endpoint,
            bytes: bytes.to_vec(),
            response_after_ms,
        });
        Ok(())
    }

    /// Prepares a read for the currently claimed interface.
    ///
    /// Releasing the interface invalidates the request, even if the interface
    /// is later reclaimed. A request cannot be used on another connection.
    ///
    /// # Errors
    ///
    /// Returns an endpoint planning error, `PermissionDenied` without a claim,
    /// or `InvalidInput` for an invalid timeout.
    pub fn prepare_read(
        &self,
        endpoint: u8,
        length: usize,
        timeout_ms: u32,
    ) -> io::Result<PreparedEndpointRead> {
        let plan = plan_read(&self.inventory, endpoint, length)?;
        self.connection.prepare_read_on_claim(plan, timeout_ms)
    }

    /// Runs a prepared read on its original claim.
    ///
    /// An empty or shorter response is valid. A cancelled, timed-out, or
    /// undersized read leaves the response queued.
    ///
    /// # Errors
    ///
    /// Returns `PermissionDenied` if the claim changed, `InvalidInput` for an
    /// undersized read, `Interrupted` on cancellation, or `TimedOut` when no
    /// response can complete in time.
    pub fn run_read(
        &mut self,
        request: &PreparedEndpointRead,
        cancellation: &Cancellation,
    ) -> io::Result<Vec<u8>> {
        let plan = request.plan;
        self.connection.validate_read_claim(request)?;
        cancellation.check()?;
        let (index, input) = self
            .inputs
            .iter()
            .enumerate()
            .find(|(_, input)| input.endpoint == plan.endpoint)
            .filter(|(_, input)| input.response_after_ms <= request.timeout_ms)
            .ok_or(io::ErrorKind::TimedOut)?;
        if input.bytes.len() > plan.length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read length is smaller than the scripted response",
            ));
        }
        cancellation.check()?;
        self.inputs
            .remove(index)
            .map(|input| input.bytes)
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    /// Prepares and runs one simulated endpoint read.
    ///
    /// # Errors
    ///
    /// Returns the errors from [`Self::prepare_read`] and [`Self::run_read`].
    pub fn read(
        &mut self,
        endpoint: u8,
        length: usize,
        timeout_ms: u32,
        cancellation: &Cancellation,
    ) -> io::Result<Vec<u8>> {
        let request = self.prepare_read(endpoint, length, timeout_ms)?;
        self.run_read(&request, cancellation)
    }
}

struct SimulatedBackend {
    present: [bool; 256],
}

impl Backend for SimulatedBackend {
    fn submit(&mut self, _: Setup, _: u32, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn change_claim(&mut self, _: InterfaceOperation, interface: u8) -> io::Result<()> {
        if self.present[usize::from(interface)] {
            Ok(())
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }
}
