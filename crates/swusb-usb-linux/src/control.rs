//! Synchronous USB standard reads on endpoint zero.

use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod endpoint_read;
mod queue;
mod simulator;
pub use endpoint_read::PreparedEndpointRead;
pub use queue::{ControlCompletion, ControlQueue};
pub use simulator::SimulatedDevice;

use crate::endpoint::EndpointReadPlan;

/// The device-recipient standard reads supported by the first transfer backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRead {
    DeviceDescriptor,
    ConfigurationValue,
    DeviceStatus,
}

impl ControlRead {
    const fn setup(self) -> Setup {
        let (request, value, length) = match self {
            Self::DeviceDescriptor => (6, 0x0100, 18),
            Self::ConfigurationValue => (8, 0, 1),
            Self::DeviceStatus => (0, 0, 2),
        };
        Setup {
            request_type: 0x80, // IN, standard, device recipient
            request,
            value,
            index: 0,
            length,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Setup {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
}

/// A one-way cancellation signal for a control read.
#[derive(Debug, Default)]
pub struct Cancellation(AtomicBool);

impl Cancellation {
    /// Requests cancellation. An ioctl already in progress can still finish.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn check(&self) -> io::Result<()> {
        if self.0.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "USB control read cancelled",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterfaceOperation {
    Claim,
    Release,
}

trait Backend: Send {
    fn submit(&mut self, setup: Setup, timeout_ms: u32, data: &mut [u8]) -> io::Result<usize>;
    fn change_claim(&mut self, operation: InterfaceOperation, interface: u8) -> io::Result<()>;
    fn seal_claims(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn validate_endpoint(&mut self, _: EndpointReadPlan) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn submit_endpoint(&mut self, _: EndpointReadPlan, _: u32, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// A handle to one USB device connection, kept open across standard reads.
///
/// After a disconnect, enumerate again and open a new handle. This handle
/// continues to refer to the original connection even if its address is reused.
pub struct ControlDevice {
    backend: Box<dyn Backend>,
    claimed: BTreeMap<u8, Arc<()>>,
    claims_sealed: bool,
}

impl ControlDevice {
    /// Opens the usbfs node for a bus and address obtained from fresh inventory.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid address, an unavailable platform, or if
    /// the node cannot be opened.
    pub fn open(bus: u16, address: u16) -> io::Result<Self> {
        let device = platform::Device::open(&device_path(bus, address)?)?;
        Ok(Self::with_backend(device))
    }

    fn with_backend(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
            claimed: BTreeMap::new(),
            claims_sealed: false,
        }
    }

    /// Claims an interface for this connection. The kernel rejects a claim
    /// when the interface already belongs to a driver or another handle.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyExists` for a duplicate local claim, `PermissionDenied`
    /// if claims are sealed, or an OS error if the interface cannot be claimed.
    pub fn claim_interface(&mut self, interface: u8) -> io::Result<()> {
        if self.claimed.contains_key(&interface) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "USB interface is already claimed by this handle",
            ));
        }
        if self.claims_sealed {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "new USB interface claims are sealed on this handle",
            ));
        }
        self.backend
            .change_claim(InterfaceOperation::Claim, interface)?;
        self.claimed.insert(interface, Arc::new(()));
        Ok(())
    }

    /// Releases a claim held by this connection. Closing the connection also
    /// releases its claims in usbfs.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for an interface this handle did not claim, or an
    /// OS error if release fails. A failed release leaves the claim recorded.
    pub fn release_interface(&mut self, interface: u8) -> io::Result<()> {
        if !self.claimed.contains_key(&interface) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "USB interface is not claimed by this handle",
            ));
        }
        self.backend
            .change_claim(InterfaceOperation::Release, interface)?;
        self.claimed.remove(&interface);
        Ok(())
    }

    /// Permanently blocks new claims on this handle, including implicit
    /// claims by usbfs. Existing claims can still be released.
    ///
    /// Calling this again on a sealed handle succeeds without another ioctl.
    ///
    /// # Errors
    ///
    /// Returns an OS or backend error if sealing fails. The handle remains
    /// unsealed in that case.
    pub fn seal_claims(&mut self) -> io::Result<()> {
        if !self.claims_sealed {
            self.backend.seal_claims()?;
            self.claims_sealed = true;
        }
        Ok(())
    }

    /// Performs one standard read with a timeout in milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid timeouts, OS failures, or invalid responses.
    pub fn read(&mut self, request: ControlRead, timeout_ms: u32) -> io::Result<Vec<u8>> {
        execute(request, timeout_ms, None, |setup, timeout, data| {
            self.backend.submit(setup, timeout, data)
        })
    }

    /// Performs a read with cooperative cancellation.
    ///
    /// A cancellation observed before submission skips the ioctl. One observed
    /// after submission discards its result when the synchronous ioctl returns.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::read`], or [`io::ErrorKind::Interrupted`]
    /// when cancellation is observed.
    pub fn read_cancellable(
        &mut self,
        request: ControlRead,
        timeout_ms: u32,
        cancellation: &Cancellation,
    ) -> io::Result<Vec<u8>> {
        execute(
            request,
            timeout_ms,
            Some(cancellation),
            |setup, timeout, data| self.backend.submit(setup, timeout, data),
        )
    }
}

/// Issues one standard read to a Linux usbfs device at the given bus and address.
/// The timeout applies to the USB transfer itself, in milliseconds.
///
/// # Errors
///
/// Returns an error for invalid addresses or timeouts (outside 1..=5000 ms),
/// unavailable platforms, OS failures, or malformed/short responses.
pub fn read(bus: u16, address: u16, request: ControlRead, timeout_ms: u32) -> io::Result<Vec<u8>> {
    let path = device_path(bus, address)?;
    execute(request, timeout_ms, None, |setup, timeout, data| {
        platform::Device::open(&path)?.submit(setup, timeout, data)
    })
}

fn device_path(bus: u16, address: u16) -> io::Result<String> {
    if !(1..=999).contains(&bus) || !(1..=127).contains(&address) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid USB bus or address",
        ));
    }
    Ok(format!("/dev/bus/usb/{bus:03}/{address:03}"))
}

fn execute(
    request: ControlRead,
    timeout_ms: u32,
    cancellation: Option<&Cancellation>,
    submit: impl FnOnce(Setup, u32, &mut [u8]) -> io::Result<usize>,
) -> io::Result<Vec<u8>> {
    validate_timeout(timeout_ms)?;
    let setup = request.setup();
    let mut data = vec![0; usize::from(setup.length)];
    if let Some(signal) = cancellation {
        signal.check()?;
    }
    let result = submit(setup, timeout_ms, &mut data);
    if let Some(signal) = cancellation {
        signal.check()?;
    }
    let actual = result?;
    if actual != data.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("USB read returned {actual} bytes, expected {}", data.len()),
        ));
    }
    if request == ControlRead::DeviceDescriptor && !data.starts_with(&[18, 1]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid USB device descriptor header",
        ));
    }
    Ok(data)
}

fn validate_timeout(timeout_ms: u32) -> io::Result<()> {
    if (1..=5000).contains(&timeout_ms) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "USB timeout must be 1..=5000 ms",
        ))
    }
}

// These Linux architectures use the asm-generic _IOC bit layout.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm"
    )
))]
#[path = "control/usbfs.rs"]
mod platform;

#[cfg(not(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm"
    )
)))]
mod platform {
    use super::{Backend, InterfaceOperation, Setup, io};

    pub(super) enum Device {}

    impl Device {
        pub(super) fn open(_: &str) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "USB control reads require supported Linux usbfs",
            ))
        }
    }

    impl Backend for Device {
        fn submit(&mut self, _: Setup, _: u32, _: &mut [u8]) -> io::Result<usize> {
            match *self {}
        }

        fn change_claim(&mut self, _: InterfaceOperation, _: u8) -> io::Result<()> {
            match *self {}
        }
    }
}

#[cfg(test)]
mod tests;
