use std::{env, path::Path};

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    let root = match args.as_slice() {
        [] => Path::new(swusb_usb_linux::DEFAULT_SYSFS_ROOT),
        [command] if command == "list" => Path::new(swusb_usb_linux::DEFAULT_SYSFS_ROOT),
        [command, path] if command == "list" => Path::new(path),
        [command, bus, address] if command == "read-device" => {
            read_device(bus, address);
            return;
        }
        _ => {
            eprintln!("usage: swusb-linux-agent [list [SYSFS_ROOT] | read-device BUS ADDRESS]");
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

fn read_device(bus: &str, address: &str) {
    use swusb_usb_linux::control::{ControlRead, read};

    let (Ok(bus), Ok(address)) = (bus.parse(), address.parse()) else {
        eprintln!("BUS and ADDRESS must be decimal USB numbers");
        std::process::exit(2);
    };
    match read(bus, address, ControlRead::DeviceDescriptor, 1000) {
        Ok(bytes) => println!("Device descriptor: {bytes:02x?}"),
        Err(error) => {
            eprintln!("USB control read failed: {error}");
            std::process::exit(1);
        }
    }
}
