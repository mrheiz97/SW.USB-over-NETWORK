use std::{
    fs,
    io::ErrorKind,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use swusb_usb_linux::{
    control::{Cancellation, SimulatedDevice},
    endpoint::{EndpointReadKind, MAX_READ_BYTES, plan_read},
};

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

fn simulated_endpoint_device() -> SimulatedDevice {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/usb-simulator/sysfs");
    let mut device = swusb_usb_linux::enumerate(&fixture).unwrap().remove(0);
    let mut bulk = device.interfaces[0].clone();
    bulk.number = 1;
    bulk.endpoints[0].address = 0x82;
    bulk.endpoints[0].attributes = 2;
    bulk.endpoints[0].max_packet_size = 512;
    device.interfaces.push(bulk);
    SimulatedDevice::new(device).unwrap()
}

#[test]
fn simulator_reads_bulk_and_interrupt_in_order_per_endpoint() {
    let mut device = simulated_endpoint_device();
    let cancellation = Cancellation::default();
    device.queue_input(0x82, &[1, 2, 3], 5).unwrap();
    device.queue_input(0x81, &[0x10, 0x11], 10).unwrap();
    device.queue_input(0x82, &[4], 0).unwrap();
    device.queue_input(0x81, &[], 0).unwrap();

    device.claim_interface(0).unwrap();
    device.claim_interface(1).unwrap();
    assert_eq!(
        device.read(0x81, 8, 10, &cancellation).unwrap(),
        [0x10, 0x11]
    );
    assert_eq!(device.read(0x82, 512, 5, &cancellation).unwrap(), [1, 2, 3]);
    assert_eq!(device.read(0x82, 512, 5, &cancellation).unwrap(), [4]);
    assert!(device.read(0x81, 8, 10, &cancellation).unwrap().is_empty());
    assert_eq!(
        device.read(0x81, 8, 10, &cancellation).unwrap_err().kind(),
        ErrorKind::TimedOut
    );
}

#[test]
fn simulator_endpoint_errors_leave_the_input_available_for_retry() {
    let mut device = simulated_endpoint_device();
    let cancellation = Cancellation::default();
    device.queue_input(0x82, &[1, 2], 200).unwrap();
    // Owning the Interrupt interface does not authorize a Bulk read.
    device.claim_interface(0).unwrap();
    assert_eq!(
        device.read(0x82, 8, 200, &cancellation).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    device.claim_interface(1).unwrap();
    assert_eq!(
        device.read(0x82, 8, 199, &cancellation).unwrap_err().kind(),
        ErrorKind::TimedOut
    );
    assert_eq!(
        device.read(0x82, 1, 200, &cancellation).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(
        device.read(0x82, 8, 200, &cancelled).unwrap_err().kind(),
        ErrorKind::Interrupted
    );
    device.release_interface(1).unwrap();
    assert_eq!(
        device.read(0x82, 8, 200, &cancellation).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    device.claim_interface(1).unwrap();
    assert_eq!(device.read(0x82, 8, 200, &cancellation).unwrap(), [1, 2]);
}

#[test]
fn simulator_bounds_input_slots_and_payload_size() {
    let mut device = simulated_endpoint_device();
    assert_eq!(
        device
            .queue_input(0x82, &vec![0; MAX_READ_BYTES + 1], 0)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    for _ in 0..SimulatedDevice::INPUT_CAPACITY {
        device.queue_input(0x82, &[1], 0).unwrap();
    }
    assert_eq!(
        device.queue_input(0x81, &[1], 0).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    device.claim_interface(1).unwrap();
    assert_eq!(
        device.read(0x82, 8, 1, &Cancellation::default()).unwrap(),
        [1]
    );
    device.queue_input(0x81, &[1], 0).unwrap();
}

#[test]
fn prepared_endpoint_read_cannot_migrate_to_a_replacement_at_the_same_address() {
    let mut original = simulated_endpoint_device();
    original.claim_interface(1).unwrap();
    original.queue_input(0x82, &[1], 0).unwrap();
    let prepared = original.prepare_read(0x82, 8, 500).unwrap();
    assert_eq!(prepared.plan().kind, EndpointReadKind::Bulk);
    drop(original);

    // The helper uses the same fixture, including its bus and device numbers.
    let mut replacement = simulated_endpoint_device();
    replacement.claim_interface(1).unwrap();
    replacement.queue_input(0x82, &[2], 0).unwrap();
    let cancellation = Cancellation::default();
    assert_eq!(
        replacement
            .run_read(&prepared, &cancellation)
            .unwrap_err()
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(replacement.read(0x82, 8, 500, &cancellation).unwrap(), [2]);
}

#[test]
fn release_and_reclaim_invalidates_only_that_interfaces_prepared_reads() {
    let mut device = simulated_endpoint_device();
    device.claim_interface(0).unwrap();
    device.claim_interface(1).unwrap();
    device.queue_input(0x81, &[1], 0).unwrap();
    device.queue_input(0x82, &[2], 0).unwrap();
    let interrupt = device.prepare_read(0x81, 8, 500).unwrap();
    let bulk = device.prepare_read(0x82, 8, 500).unwrap();
    let cancellation = Cancellation::default();

    device.release_interface(1).unwrap();
    device.claim_interface(1).unwrap();
    assert_eq!(
        device.run_read(&bulk, &cancellation).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(device.run_read(&interrupt, &cancellation).unwrap(), [1]);
    assert_eq!(device.read(0x82, 8, 500, &cancellation).unwrap(), [2]);
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
