//! Shared, platform-independent USB inventory types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbEndpoint {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbInterface {
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<UsbEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub sysfs_name: String,
    pub bus_number: u16,
    pub device_number: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usb_version: String,
    pub device_class: u8,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub configuration_count: u8,
    pub interfaces: Vec<UsbInterface>,
}
