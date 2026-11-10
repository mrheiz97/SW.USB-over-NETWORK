# SW.USB

SW.USB is an experimental, vendor-independent USB-over-network project. M1
provides USB inventory. M2 begins with local USB control reads on Linux.

## Implemented in M1

- A bounded parser for standard device, configuration, interface, endpoint,
  and UTF-16LE string descriptors, plus preservation of unknown descriptors.
- Read-only Linux USB inventory through sysfs (no libusb, no driver detach).
- A CLI that lists devices, interfaces, and endpoints.
- A portable simulated sysfs tree and malformed-descriptor tests.
- CI for formatting, Clippy, tests, dependency audit, and private-file checks.

## Build and test

```sh
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

On Linux, list real devices:

```sh
cargo run -p swusb-linux-agent -- list
```

On any development host, list the simulator fixture:

```sh
cargo run -p swusb-linux-agent -- list tests/usb-simulator/sysfs
```

The real-device command requires readable `/sys/bus/usb/devices`. It performs
inventory only and does not open usbfs device nodes or claim interfaces.

## Read a USB device descriptor on Linux

Use the bus and device numbers from `list`:

```sh
cargo run -p swusb-linux-agent -- read-device 001 002
cargo run -p swusb-linux-agent -- read-controls 001 002
```

Linux requires read and write access to the usbfs device node for the ioctl.
`read-device` prints the device descriptor. `read-controls` reads the
descriptor, active configuration, and device status over one connection.

To inspect a Bulk or Interrupt IN read plan using the simulator:

```sh
cargo run -p swusb-linux-agent -- plan-read 001 002 81 8 tests/usb-simulator/sysfs
```

This command uses inventory only. It reports the active interface, transfer
type, packet size and requested length.

See [development status](docs/development/m1-status.md), the
[architecture](docs/architecture/m1-native-core.md), and
[contributing guide](CONTRIBUTING.md).
