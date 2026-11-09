# SW.USB

SW.USB is an experimental, vendor-independent USB-over-network platform. The
current implementation is **Milestone M1: Native USB Core**; it does not yet
forward USB transfers over a network and must not be represented as a complete
USB device server.

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

## Local contributor setup

Private `AGENTS.md` files are ignored and rejected if tracked. Enable the
provided pre-commit hook after cloning:

```sh
git config core.hooksPath .githooks
```

See [development status](docs/development/m1-status.md), the
[architecture](docs/architecture/m1-native-core.md), and
[contributing guide](CONTRIBUTING.md).
