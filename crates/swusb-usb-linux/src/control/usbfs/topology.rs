use std::io;

use swusb_descriptors::{Descriptor, parse_descriptors};

use crate::endpoint::{EndpointReadKind, EndpointReadPlan};

pub(super) const MAX_DESCRIPTOR_BYTES: usize = 128 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn active_configuration(descriptors: &[u8], configuration: u8) -> io::Result<&[u8]> {
    if configuration == 0 || descriptors.len() > MAX_DESCRIPTOR_BYTES {
        return Err(invalid(
            "unconfigured device or oversized USB descriptor cache",
        ));
    }
    let Some((device, mut remaining)) = descriptors.split_at_checked(18) else {
        return Err(invalid("truncated cached USB device descriptor"));
    };
    if !device.starts_with(&[18, 1]) {
        return Err(invalid("invalid cached USB device descriptor"));
    }
    let mut active = None;
    for _ in 0..device[17] {
        let Some([9, 2, low, high, _, value, ..]) = remaining.get(..9) else {
            return Err(invalid("invalid cached USB configuration header"));
        };
        let length = usize::from(u16::from_le_bytes([*low, *high]));
        if length < 9 {
            return Err(invalid("invalid cached USB configuration length"));
        }
        let Some((current, rest)) = remaining.split_at_checked(length) else {
            return Err(invalid("truncated cached USB configuration"));
        };
        if *value == configuration && active.replace(current).is_some() {
            return Err(invalid("ambiguous active USB configuration"));
        }
        remaining = rest;
    }
    if !remaining.is_empty() {
        return Err(invalid("unexpected bytes after cached USB configurations"));
    }
    active.ok_or_else(|| invalid("active USB configuration is absent from the cache"))
}

pub(super) fn validate(
    descriptors: &[u8],
    configuration: u8,
    plan: EndpointReadPlan,
    read_alternate: impl FnOnce() -> io::Result<u8>,
) -> io::Result<()> {
    let configuration = active_configuration(descriptors, configuration)?;
    let parsed = parse_descriptors(configuration)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut alternates = parsed.iter().filter_map(|descriptor| match descriptor {
        Descriptor::Interface(interface) if interface.number == plan.interface => {
            Some(interface.alternate_setting)
        }
        _ => None,
    });
    let first = alternates
        .next()
        .ok_or_else(|| invalid("USB interface is absent from the active configuration"))?;
    let alternate = if alternates.next().is_some() {
        read_alternate()?
    } else {
        first
    };
    if alternate != plan.alternate_setting {
        return Err(invalid("USB alternate setting changed"));
    }
    let mut current = None;
    let mut active_seen = false;
    let mut found = false;
    for descriptor in parsed {
        match descriptor {
            Descriptor::Interface(interface) => {
                let active = interface.alternate_setting == alternate;
                if interface.number == plan.interface
                    && active
                    && std::mem::replace(&mut active_seen, true)
                {
                    return Err(invalid("ambiguous active USB alternate setting"));
                }
                current = Some((interface.number, active));
            }
            Descriptor::Endpoint(endpoint) if endpoint.address == plan.endpoint => {
                let Some((owner, active)) = current else {
                    return Err(invalid("USB endpoint has no interface"));
                };
                if owner != plan.interface {
                    return Err(invalid(
                        "USB endpoint address also belongs to another interface",
                    ));
                }
                if active {
                    let kind_matches = matches!(
                        (plan.kind, endpoint.attributes & 3),
                        (EndpointReadKind::Bulk, 2) | (EndpointReadKind::Interrupt, 3)
                    );
                    let packet_size = endpoint.max_packet_size & 0x07ff;
                    if found
                        || !kind_matches
                        || packet_size == 0
                        || packet_size != plan.max_packet_size
                    {
                        return Err(invalid("USB endpoint metadata no longer matches the plan"));
                    }
                    found = true;
                }
            }
            _ => {}
        }
    }
    if found {
        Ok(())
    } else {
        Err(invalid(
            "USB endpoint is absent from the active alternate setting",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(records: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 18];
        bytes[..2].copy_from_slice(&[18, 1]);
        bytes[17] = 1;
        let [low, high] = u16::try_from(9 + records.len()).unwrap().to_le_bytes();
        bytes.extend_from_slice(&[9, 2, low, high, 1, 3, 0, 0x80, 0]);
        bytes.extend_from_slice(records);
        bytes
    }

    const PLAN: EndpointReadPlan = EndpointReadPlan {
        kind: EndpointReadKind::Bulk,
        interface: 4,
        alternate_setting: 1,
        endpoint: 0x82,
        max_packet_size: 512,
        length: 512,
    };
    const RECORDS: [u8; 25] = [
        9, 4, 4, 0, 0, 0xff, 0, 0, 0, 9, 4, 4, 1, 1, 0xff, 0, 0, 0, 7, 5, 0x82, 2, 0, 2, 0,
    ];

    #[test]
    fn validates_bulk_and_interrupt_metadata_on_the_active_setting() {
        let mut bytes = cache(&RECORDS);
        validate(&bytes, 3, PLAN, || Ok(1)).unwrap();
        bytes[18 + 9 + 18 + 3] = 3;
        assert!(validate(&bytes, 3, PLAN, || Ok(1)).is_err());
        let plan = EndpointReadPlan {
            kind: EndpointReadKind::Interrupt,
            ..PLAN
        };
        validate(&bytes, 3, plan, || Ok(1)).unwrap();
        assert!(validate(&bytes, 3, plan, || Ok(0)).is_err());
        assert!(validate(&bytes, 1, plan, || Ok(1)).is_err());
        assert!(validate(&bytes[..bytes.len() - 1], 3, plan, || Ok(1)).is_err());
        assert_eq!(
            validate(&bytes, 3, plan, || Err(io::ErrorKind::TimedOut.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn rejects_aliases_even_on_an_inactive_setting_of_another_interface() {
        let mut records = RECORDS.to_vec();
        records.extend_from_slice(&[9, 4, 5, 2, 1, 0xff, 0, 0, 0, 7, 5, 0x82, 2, 0, 2, 0]);
        assert!(validate(&cache(&records), 3, PLAN, || Ok(1)).is_err());
    }

    #[test]
    fn sole_alternate_setting_does_not_require_a_device_query() {
        let bytes = cache(&[9, 4, 4, 0, 1, 7, 0, 0, 0, 7, 5, 0x82, 2, 0, 2, 0]);
        let plan = EndpointReadPlan {
            alternate_setting: 0,
            ..PLAN
        };
        validate(&bytes, 3, plan, || {
            panic!("sole alternate setting is known")
        })
        .unwrap();
    }
}
