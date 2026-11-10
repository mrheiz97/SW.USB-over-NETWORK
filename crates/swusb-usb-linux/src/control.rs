//! Synchronous USB standard reads on endpoint zero.

use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

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

type Submit = dyn FnMut(Setup, u32, &mut [u8]) -> io::Result<usize> + Send;

/// A handle to one USB device connection, kept open across standard reads.
///
/// After a disconnect, enumerate again and open a new handle. This handle
/// continues to refer to the original connection even if its address is reused.
pub struct ControlDevice {
    submit: Box<Submit>,
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
        Ok(Self {
            submit: Box::new(move |setup, timeout, data| device.submit(setup, timeout, data)),
        })
    }

    /// Performs one standard read with a timeout in milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid timeouts, OS failures, or invalid responses.
    pub fn read(&mut self, request: ControlRead, timeout_ms: u32) -> io::Result<Vec<u8>> {
        execute(request, timeout_ms, None, &mut self.submit)
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
        execute(request, timeout_ms, Some(cancellation), &mut self.submit)
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
    if !(1..=5000).contains(&timeout_ms) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "USB timeout must be 1..=5000 ms",
        ));
    }
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
    use super::{Setup, io};

    pub(super) enum Device {}

    impl Device {
        pub(super) fn open(_: &str) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "USB control reads require supported Linux usbfs",
            ))
        }

        pub(super) fn submit(&self, _: Setup, _: u32, _: &mut [u8]) -> io::Result<usize> {
            match *self {}
        }
    }
}

#[cfg(test)]
mod tests;
