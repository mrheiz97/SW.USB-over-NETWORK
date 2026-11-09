use std::{env, path::Path};

fn main() {
    let mut args = env::args().skip(1);
    let root = match (args.next().as_deref(), args.next()) {
        (None | Some("list"), None) => Path::new(swusb_usb_linux::DEFAULT_SYSFS_ROOT).to_path_buf(),
        (Some("list"), Some(path)) => path.into(),
        _ => {
            eprintln!("usage: swusb-linux-agent [list [SYSFS_ROOT]]");
            std::process::exit(2);
        }
    };
    match swusb_usb_linux::enumerate(&root) {
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
