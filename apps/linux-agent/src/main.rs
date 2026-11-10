use std::{env, path::Path};

use swusb_usb_linux::control::{ControlDevice, ControlQueue, ControlRead, read};
use swusb_usb_linux::endpoint::plan_read;

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    let root = match args.as_slice() {
        [] => Path::new(swusb_usb_linux::DEFAULT_SYSFS_ROOT),
        [command] if command == "list" => Path::new(swusb_usb_linux::DEFAULT_SYSFS_ROOT),
        [command, path] if command == "list" => Path::new(path),
        [command, bus, address] if matches!(command.as_str(), "read-device" | "read-controls") => {
            read_controls(bus, address, command == "read-controls");
            return;
        }
        [command, bus, address, endpoint, length, root @ ..]
            if command == "plan-read" && root.len() <= 1 =>
        {
            let root = root
                .first()
                .map_or(swusb_usb_linux::DEFAULT_SYSFS_ROOT, String::as_str);
            if let Err(error) = print_read_plan(Path::new(root), bus, address, endpoint, length) {
                eprintln!("USB endpoint plan failed: {error}");
                std::process::exit(1);
            }
            return;
        }
        _ => {
            eprintln!(
                "usage: swusb-linux-agent [list [SYSFS_ROOT] | read-device BUS ADDRESS | read-controls BUS ADDRESS]"
            );
            eprintln!(
                "       swusb-linux-agent plan-read BUS ADDRESS ENDPOINT_HEX LENGTH [SYSFS_ROOT]"
            );
            std::process::exit(2);
        }
    };
    match swusb_usb_linux::enumerate(root) {
        Ok(devices) => {
            if devices.is_empty() {
                println!("No USB devices found under {}", root.display());
            }
            for device in devices {
                println!(
                    "Bus {:03} Device {:03}: ID {:04x}:{:04x} {}",
                    device.bus_number,
                    device.device_number,
                    device.vendor_id,
                    device.product_id,
                    device.product.as_deref().unwrap_or("<unknown>")
                );
                println!(
                    "  USB {} class {:02x}; manufacturer: {}; serial: {}; configurations: {}",
                    device.usb_version,
                    device.device_class,
                    device.manufacturer.as_deref().unwrap_or("<unknown>"),
                    device.serial_number.as_deref().unwrap_or("<unavailable>"),
                    device.configuration_count
                );
                for interface in device.interfaces {
                    println!(
                        "  Interface {} alt {} class {:02x}/{:02x}/{:02x}",
                        interface.number,
                        interface.alternate_setting,
                        interface.class,
                        interface.subclass,
                        interface.protocol
                    );
                    for endpoint in interface.endpoints {
                        println!(
                            "    Endpoint {:02x} attributes {:02x} max-packet {} interval {}",
                            endpoint.address,
                            endpoint.attributes,
                            endpoint.max_packet_size,
                            endpoint.interval
                        );
                    }
                }
            }
        }
        Err(error) => {
            eprintln!("USB inventory failed: {error}");
            std::process::exit(1);
        }
    }
}

fn print_read_plan(
    root: &Path,
    bus: &str,
    address: &str,
    endpoint: &str,
    length: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (bus, address) = (bus.parse()?, address.parse()?);
    let endpoint = u8::from_str_radix(endpoint.trim_start_matches("0x"), 16)?;
    let length = length.parse()?;
    let devices = swusb_usb_linux::enumerate(root)?;
    let mut matches = devices
        .iter()
        .filter(|device| (device.bus_number, device.device_number) == (bus, address));
    let device = matches.next().ok_or("USB device not found")?;
    if matches.next().is_some() {
        return Err("USB device address is ambiguous".into());
    }
    let plan = plan_read(device, endpoint, length)?;
    println!(
        "Plan: {:?} IN endpoint {:02x}, interface {} alt {}, {} bytes, max packet {}",
        plan.kind,
        plan.endpoint,
        plan.interface,
        plan.alternate_setting,
        plan.length,
        plan.max_packet_size
    );
    Ok(())
}

fn read_controls(bus: &str, address: &str, batch: bool) {
    let (Ok(bus), Ok(address)) = (bus.parse(), address.parse()) else {
        eprintln!("BUS and ADDRESS must be decimal USB numbers");
        std::process::exit(2);
    };
    let result = if batch {
        read_batch(bus, address)
    } else {
        read(bus, address, ControlRead::DeviceDescriptor, 1000)
            .map(|bytes| println!("Device descriptor: {bytes:02x?}"))
    };
    if let Err(error) = result {
        eprintln!("USB control read failed: {error}");
        std::process::exit(1);
    }
}

fn read_batch(bus: u16, address: u16) -> std::io::Result<()> {
    let mut queue = ControlQueue::new(ControlDevice::open(bus, address)?);
    for request in [
        ControlRead::DeviceDescriptor,
        ControlRead::ConfigurationValue,
        ControlRead::DeviceStatus,
    ] {
        queue.enqueue(request, 1000)?;
    }
    while let Some(completion) = queue.run_next() {
        let label = match completion.request {
            ControlRead::DeviceDescriptor => "GET_DESCRIPTOR",
            ControlRead::ConfigurationValue => "GET_CONFIGURATION",
            ControlRead::DeviceStatus => "GET_STATUS",
        };
        let bytes = completion
            .result
            .inspect_err(|_| eprintln!("{label} failed"))?;
        println!("{label}: {bytes:02x?}");
    }
    Ok(())
}
