//! Read-only planning for Bulk and Interrupt IN endpoints from sysfs inventory.

use std::io;
use swusb_core::UsbDevice;

/// Maximum payload size for one planned endpoint read.
pub const MAX_READ_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointReadKind {
    Bulk,
    Interrupt,
}

/// An endpoint and its interface in one inventory snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointReadPlan {
    pub kind: EndpointReadKind,
    pub interface: u8,
    pub alternate_setting: u8,
    pub endpoint: u8,
    pub max_packet_size: u16,
    pub length: usize,
}

/// Resolves an IN endpoint in the device's active interface inventory.
///
/// The plan describes a snapshot. A caller must revalidate the connection
/// before using it for I/O.
///
/// # Errors
///
/// Returns an error for invalid addresses or lengths, a missing or ambiguous
/// endpoint, malformed inventory, or an unsupported transfer type.
pub fn plan_read(device: &UsbDevice, endpoint: u8, length: usize) -> io::Result<EndpointReadPlan> {
    if !(0x81..=0x8f).contains(&endpoint) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a nonzero USB IN endpoint address (81–8f hex)",
        ));
    }
    if !(1..=MAX_READ_BYTES).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "endpoint read length must be 1..=16384 bytes",
        ));
    }
    let mut matched = None;
    let mut interfaces = [false; 256];
    for interface in &device.interfaces {
        if std::mem::replace(&mut interfaces[usize::from(interface.number)], true)
            || interface.endpoints.len() > 30
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid active USB interface inventory",
            ));
        }
        for entry in interface.endpoints.iter().filter(|e| e.address == endpoint) {
            if matched.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "USB endpoint appears more than once in active interfaces",
                ));
            }
            matched = Some((interface, entry));
        }
    }
    let (interface, entry) =
        matched.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "USB endpoint not found"))?;
    let kind = match entry.attributes & 3 {
        2 => EndpointReadKind::Bulk,
        3 => EndpointReadKind::Interrupt,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "endpoint must be Bulk or Interrupt",
            ));
        }
    };
    if entry.max_packet_size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "USB endpoint reports a zero packet size",
        ));
    }
    Ok(EndpointReadPlan {
        kind,
        interface: interface.number,
        alternate_setting: interface.alternate_setting,
        endpoint,
        max_packet_size: entry.max_packet_size,
        length,
    })
}
