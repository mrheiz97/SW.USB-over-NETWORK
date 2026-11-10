//! Linux sysfs inventory and read-only USB control transfers.

pub mod control;

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};
use swusb_core::{UsbDevice, UsbEndpoint, UsbInterface};

pub const DEFAULT_SYSFS_ROOT: &str = "/sys/bus/usb/devices";

#[derive(Debug)]
pub enum InventoryError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    InvalidField {
        path: PathBuf,
        value: String,
        expected: &'static str,
    },
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidField {
                path,
                value,
                expected,
            } => write!(f, "{}: invalid {expected}: {value:?}", path.display()),
        }
    }
}
impl std::error::Error for InventoryError {}

/// Enumerates devices under the default Linux USB sysfs root.
///
/// # Errors
///
/// Returns [`InventoryError`] when the root cannot be read or a required sysfs
/// attribute is unreadable or malformed.
pub fn enumerate_default() -> Result<Vec<UsbDevice>, InventoryError> {
    enumerate(Path::new(DEFAULT_SYSFS_ROOT))
}

/// Enumerates physical USB device directories. Individual malformed devices are errors;
/// callers never receive silently corrupted identity data.
///
/// # Errors
///
/// Returns [`InventoryError`] when `root` cannot be read or a required device,
/// interface, or endpoint attribute is unreadable or malformed.
pub fn enumerate(root: &Path) -> Result<Vec<UsbDevice>, InventoryError> {
    let entries = fs::read_dir(root).map_err(|source| InventoryError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut devices = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| InventoryError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_dir() || !path.join("idVendor").is_file() {
            continue;
        }
        devices.push(read_device(&path)?);
    }
    devices.sort_by_key(|d| (d.bus_number, d.device_number));
    Ok(devices)
}

fn read_device(path: &Path) -> Result<UsbDevice, InventoryError> {
    let mut interfaces = read_interfaces(path)?;
    interfaces.sort_by_key(|i| (i.number, i.alternate_setting));
    Ok(UsbDevice {
        sysfs_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        bus_number: decimal(path, "busnum")?,
        device_number: decimal(path, "devnum")?,
        vendor_id: hex(path, "idVendor")?,
        product_id: hex(path, "idProduct")?,
        usb_version: required(path, "version")?,
        device_class: hex(path, "bDeviceClass")?,
        manufacturer: optional(path, "manufacturer")?,
        product: optional(path, "product")?,
        serial_number: optional(path, "serial")?,
        configuration_count: decimal(path, "bNumConfigurations")?,
        interfaces,
    })
}

fn read_interfaces(device: &Path) -> Result<Vec<UsbInterface>, InventoryError> {
    let Some(parent) = device.parent() else {
        return Ok(Vec::new());
    };
    let device_name = device.file_name().unwrap_or_default().to_string_lossy();
    let native_prefix = format!("{device_name}:");
    // `:` is not portable to Windows checkouts, so simulator fixtures may use `__`.
    let fixture_prefix = format!("{device_name}__");
    let mut result = Vec::new();
    for entry in fs::read_dir(parent).map_err(|source| InventoryError::Io {
        path: parent.to_path_buf(),
        source,
    })? {
        let path = entry
            .map_err(|source| InventoryError::Io {
                path: parent.to_path_buf(),
                source,
            })?
            .path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if (!name.starts_with(&native_prefix) && !name.starts_with(&fixture_prefix))
            || !path.join("bInterfaceNumber").is_file()
        {
            continue;
        }
        result.push(UsbInterface {
            number: hex(&path, "bInterfaceNumber")?,
            alternate_setting: decimal(&path, "bAlternateSetting")?,
            class: hex(&path, "bInterfaceClass")?,
            subclass: hex(&path, "bInterfaceSubClass")?,
            protocol: hex(&path, "bInterfaceProtocol")?,
            endpoints: read_endpoints(&path)?,
        });
    }
    Ok(result)
}

fn read_endpoints(interface: &Path) -> Result<Vec<UsbEndpoint>, InventoryError> {
    let mut result = Vec::new();
    for entry in fs::read_dir(interface).map_err(|source| InventoryError::Io {
        path: interface.to_path_buf(),
        source,
    })? {
        let path = entry
            .map_err(|source| InventoryError::Io {
                path: interface.to_path_buf(),
                source,
            })?
            .path();
        if !path.join("bEndpointAddress").is_file() {
            continue;
        }
        result.push(UsbEndpoint {
            address: hex(&path, "bEndpointAddress")?,
            attributes: hex(&path, "bmAttributes")?,
            max_packet_size: decimal(&path, "wMaxPacketSize")?,
            interval: decimal(&path, "bInterval")?,
        });
    }
    result.sort_by_key(|e| e.address);
    Ok(result)
}

fn required(path: &Path, field: &str) -> Result<String, InventoryError> {
    let file = path.join(field);
    fs::read_to_string(&file)
        .map(|s| s.trim().to_owned())
        .map_err(|source| InventoryError::Io { path: file, source })
}
fn optional(path: &Path, field: &str) -> Result<Option<String>, InventoryError> {
    let file = path.join(field);
    match fs::read_to_string(&file) {
        Ok(s) => Ok(Some(s.trim().to_owned()).filter(|s| !s.is_empty())),
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                || e.kind() == io::ErrorKind::PermissionDenied =>
        {
            Ok(None)
        }
        Err(source) => Err(InventoryError::Io { path: file, source }),
    }
}
fn hex<T>(path: &Path, field: &str) -> Result<T, InventoryError>
where
    T: TryFrom<u16>,
{
    parse_num(path, field, 16)
}
fn decimal<T>(path: &Path, field: &str) -> Result<T, InventoryError>
where
    T: TryFrom<u16>,
{
    parse_num(path, field, 10)
}
fn parse_num<T>(path: &Path, field: &str, radix: u32) -> Result<T, InventoryError>
where
    T: TryFrom<u16>,
{
    let file = path.join(field);
    let value = required(path, field)?;
    let parsed = u16::from_str_radix(&value, radix)
        .ok()
        .and_then(|n| T::try_from(n).ok());
    parsed.ok_or(InventoryError::InvalidField {
        path: file,
        value,
        expected: if radix == 16 {
            "hexadecimal integer"
        } else {
            "decimal integer"
        },
    })
}
