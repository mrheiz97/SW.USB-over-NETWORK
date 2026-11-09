use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
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
    let devices = swusb_usb_linux::enumerate(&root).unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        (devices[0].vendor_id, devices[0].product_id),
        (0x1209, 0x0001)
    );
    assert_eq!(devices[0].interfaces.len(), 1);
    assert_eq!(devices[0].interfaces[0].endpoints[0].address, 0x81);
    fs::remove_dir_all(root).unwrap();
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
