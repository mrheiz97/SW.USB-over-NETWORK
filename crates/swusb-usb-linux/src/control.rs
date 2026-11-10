//! Synchronous USB standard reads on endpoint zero.

use std::io;

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

/// Issues one standard read to a Linux usbfs device at the given bus and address.
/// The timeout applies to the USB transfer itself, in milliseconds.
///
/// # Errors
///
/// Returns an error for invalid addresses or timeouts (outside 1..=5000 ms),
/// unavailable platforms, OS failures, or malformed/short responses.
pub fn read(bus: u16, address: u16, request: ControlRead, timeout_ms: u32) -> io::Result<Vec<u8>> {
    if !(1..=999).contains(&bus) || !(1..=127).contains(&address) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid USB bus or address",
        ));
    }
    let path = format!("/dev/bus/usb/{bus:03}/{address:03}");
    execute(request, timeout_ms, |setup, timeout, data| {
        platform::submit(&path, setup, timeout, data)
    })
}

fn execute(
    request: ControlRead,
    timeout_ms: u32,
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
    let actual = submit(setup, timeout_ms, &mut data)?;
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

    pub(super) fn submit(_: &str, _: Setup, _: u32, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "USB control reads require supported Linux usbfs",
        ))
    }
}

#[cfg(test)]
mod tests;
