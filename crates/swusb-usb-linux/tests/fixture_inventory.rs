use std::{
    fs,
    io::ErrorKind,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use swusb_usb_linux::endpoint::{EndpointReadKind, MAX_READ_BYTES, plan_read};

#[test]
fn enumerates_simulated_device_interfaces_and_endpoints() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("swusb-fixture-{}-{unique}", std::process::id()));
    copy_tree(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/usb-simulator/sysfs"),
        &root,
    );
    let bulk = root.join("1-1__1.0/ep_82");
    fs::create_dir(&bulk).unwrap();
    for (name, value) in [
        ("bEndpointAddress", "82"),
        ("bmAttributes", "02"),
        ("wMaxPacketSize", "0200"),
        ("bInterval", "00"),
    ] {
        fs::write(bulk.join(name), value).unwrap();
    }
    let devices = swusb_usb_linux::enumerate(&root).unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        (devices[0].vendor_id, devices[0].product_id),
        (0x1209, 0x0001)
    );
    assert_eq!(devices[0].interfaces.len(), 1);
    assert_eq!(devices[0].interfaces[0].endpoints[0].address, 0x81);
    assert_eq!(devices[0].interfaces[0].endpoints[0].interval, 10);
    let interrupt = plan_read(&devices[0], 0x81, 8).unwrap();
    assert_eq!(interrupt.kind, EndpointReadKind::Interrupt);
    assert_eq!((interrupt.interface, interrupt.alternate_setting), (0, 0));
    let bulk = plan_read(&devices[0], 0x82, MAX_READ_BYTES).unwrap();
    assert_eq!(bulk.kind, EndpointReadKind::Bulk);
    assert_eq!(bulk.max_packet_size, 512);
    assert_eq!(bulk.length, MAX_READ_BYTES);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_or_ambiguous_endpoint_read_plans() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/usb-simulator/sysfs");
    let mut device = swusb_usb_linux::enumerate(&fixture).unwrap().remove(0);
    for (endpoint, length, kind) in [
        (0x01, 8, ErrorKind::InvalidInput),
        (0x80, 8, ErrorKind::InvalidInput),
        (0x91, 8, ErrorKind::InvalidInput),
        (0x81, 0, ErrorKind::InvalidInput),
        (0x81, MAX_READ_BYTES + 1, ErrorKind::InvalidInput),
        (0x8f, 8, ErrorKind::NotFound),
    ] {
        assert_eq!(
            plan_read(&device, endpoint, length).unwrap_err().kind(),
            kind
        );
    }
    device.interfaces[0].endpoints[0].attributes = 1;
    assert_eq!(
        plan_read(&device, 0x81, 8).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    let mut duplicate = device.interfaces[0].clone();
    duplicate.number = 1;
    device.interfaces.push(duplicate);
    assert_eq!(
        plan_read(&device, 0x81, 8).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

fn copy_tree(source: PathBuf, target: &PathBuf) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}
