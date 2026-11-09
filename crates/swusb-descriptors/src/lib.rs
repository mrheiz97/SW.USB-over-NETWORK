//! Bounded, panic-free parsing for standard USB descriptors.

use std::fmt;

pub const DEVICE_DESCRIPTOR: u8 = 0x01;
pub const CONFIGURATION_DESCRIPTOR: u8 = 0x02;
pub const STRING_DESCRIPTOR: u8 = 0x03;
pub const INTERFACE_DESCRIPTOR: u8 = 0x04;
pub const ENDPOINT_DESCRIPTOR: u8 = 0x05;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub usb_version_bcd: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size_0: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version_bcd: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_number_index: u8,
    pub configuration_count: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationDescriptor {
    pub total_length: u16,
    pub interface_count: u8,
    pub configuration_value: u8,
    pub string_index: u8,
    pub attributes: u8,
    pub max_power_ma: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceDescriptor {
    pub number: u8,
    pub alternate_setting: u8,
    pub endpoint_count: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub string_index: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Descriptor {
    Device(DeviceDescriptor),
    Configuration(ConfigurationDescriptor),
    Interface(InterfaceDescriptor),
    Endpoint(EndpointDescriptor),
    String(String),
    Unknown { descriptor_type: u8, data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    TruncatedHeader {
        offset: usize,
        remaining: usize,
    },
    InvalidLength {
        offset: usize,
        length: u8,
    },
    TruncatedDescriptor {
        offset: usize,
        declared: usize,
        remaining: usize,
    },
    WrongLength {
        descriptor_type: u8,
        expected: usize,
        actual: usize,
    },
    InvalidStringLength {
        actual: usize,
    },
    InvalidUtf16,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ParseError {}

/// Parses a complete descriptor stream. No descriptor may extend beyond `input`.
///
/// # Errors
///
/// Returns [`ParseError`] when a record is truncated, has an invalid or
/// unexpected length, or contains an invalid USB string descriptor.
pub fn parse_descriptors(input: &[u8]) -> Result<Vec<Descriptor>, ParseError> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset < input.len() {
        let remaining = input.len() - offset;
        if remaining < 2 {
            return Err(ParseError::TruncatedHeader { offset, remaining });
        }
        let length = usize::from(input[offset]);
        if length < 2 {
            return Err(ParseError::InvalidLength {
                offset,
                length: input[offset],
            });
        }
        if length > remaining {
            return Err(ParseError::TruncatedDescriptor {
                offset,
                declared: length,
                remaining,
            });
        }
        let bytes = &input[offset..offset + length];
        result.push(parse_one(bytes)?);
        offset += length;
    }
    Ok(result)
}

fn exact(bytes: &[u8], descriptor_type: u8, expected: usize) -> Result<(), ParseError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ParseError::WrongLength {
            descriptor_type,
            expected,
            actual: bytes.len(),
        })
    }
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn parse_one(bytes: &[u8]) -> Result<Descriptor, ParseError> {
    let kind = bytes[1];
    match kind {
        DEVICE_DESCRIPTOR => {
            exact(bytes, kind, 18)?;
            Ok(Descriptor::Device(DeviceDescriptor {
                usb_version_bcd: le16(bytes, 2),
                device_class: bytes[4],
                device_subclass: bytes[5],
                device_protocol: bytes[6],
                max_packet_size_0: bytes[7],
                vendor_id: le16(bytes, 8),
                product_id: le16(bytes, 10),
                device_version_bcd: le16(bytes, 12),
                manufacturer_index: bytes[14],
                product_index: bytes[15],
                serial_number_index: bytes[16],
                configuration_count: bytes[17],
            }))
        }
        CONFIGURATION_DESCRIPTOR => {
            exact(bytes, kind, 9)?;
            Ok(Descriptor::Configuration(ConfigurationDescriptor {
                total_length: le16(bytes, 2),
                interface_count: bytes[4],
                configuration_value: bytes[5],
                string_index: bytes[6],
                attributes: bytes[7],
                max_power_ma: u16::from(bytes[8]) * 2,
            }))
        }
        INTERFACE_DESCRIPTOR => {
            exact(bytes, kind, 9)?;
            Ok(Descriptor::Interface(InterfaceDescriptor {
                number: bytes[2],
                alternate_setting: bytes[3],
                endpoint_count: bytes[4],
                class: bytes[5],
                subclass: bytes[6],
                protocol: bytes[7],
                string_index: bytes[8],
            }))
        }
        ENDPOINT_DESCRIPTOR => {
            exact(bytes, kind, 7)?;
            Ok(Descriptor::Endpoint(EndpointDescriptor {
                address: bytes[2],
                attributes: bytes[3],
                max_packet_size: le16(bytes, 4),
                interval: bytes[6],
            }))
        }
        STRING_DESCRIPTOR => {
            if bytes.len() < 2 || !(bytes.len() - 2).is_multiple_of(2) {
                return Err(ParseError::InvalidStringLength {
                    actual: bytes.len(),
                });
            }
            let units = bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]));
            let text = char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| ParseError::InvalidUtf16)?;
            Ok(Descriptor::String(text))
        }
        descriptor_type => Ok(Descriptor::Unknown {
            descriptor_type,
            data: bytes[2..].to_vec(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: [u8; 18] = [
        18, 1, 0x10, 0x02, 0, 0, 0, 64, 0x34, 0x12, 0x78, 0x56, 0, 1, 1, 2, 3, 1,
    ];

    #[test]
    fn parses_device_descriptor() {
        let parsed = parse_descriptors(&DEVICE).unwrap();
        let Descriptor::Device(device) = &parsed[0] else {
            panic!("expected device")
        };
        assert_eq!(device.vendor_id, 0x1234);
        assert_eq!(device.product_id, 0x5678);
    }

    #[test]
    fn parses_mixed_configuration_stream() {
        let bytes = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10,
        ];
        let parsed = parse_descriptors(&bytes).unwrap();
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn rejects_zero_length_without_looping() {
        assert!(matches!(
            parse_descriptors(&[0, 1]),
            Err(ParseError::InvalidLength { .. })
        ));
    }

    #[test]
    fn rejects_truncated_descriptor() {
        assert!(matches!(
            parse_descriptors(&[18, 1, 0]),
            Err(ParseError::TruncatedDescriptor { .. })
        ));
    }

    #[test]
    fn rejects_wrong_standard_length() {
        assert!(matches!(
            parse_descriptors(&[3, 1, 0]),
            Err(ParseError::WrongLength { .. })
        ));
    }

    #[test]
    fn rejects_unpaired_utf16_surrogate() {
        assert_eq!(
            parse_descriptors(&[4, 3, 0x00, 0xd8]),
            Err(ParseError::InvalidUtf16)
        );
    }
}
