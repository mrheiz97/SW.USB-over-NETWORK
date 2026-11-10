# M1 status and M2 entry criteria

M1 provides a buildable native core, safe descriptor parsing, read-only sysfs
inventory, a listing CLI, a simulator, and automated tests. A physical Linux
USB hardware test remains environment-dependent and is not implied by fixture
success.

## Known security boundaries

- sysfs values are untrusted and parsed with checked conversions; however, the
  inventory currently fails the whole scan when one device has malformed
  required attributes. Per-device diagnostics are a planned improvement.
- No usbfs node is opened, no interface is claimed, and no kernel driver is
  detached in M1.
- String output is terminal-visible untrusted device data; terminal escape
  sanitization should be added before operational deployments.

## M2 control reads

The first M2 increment uses synchronous `USBDEVFS_CONTROL` on Linux x86, x86-64,
ARM and AArch64. Only standard device-recipient IN requests for the device
descriptor, current configuration value and device status are exposed. Each
request has a fixed response size and a transfer timeout of 1–5000 ms. The CLI
uses 1000 ms. Bus and device numbers can change after unplugging or reconnecting.
Use fresh inventory before addressing a device.

Linux requires a read/write file descriptor for the ioctl. Prefer narrow
device-node permissions. The transfer path leaves interface and driver bindings
alone. Simulator tests cover setup fields, responses and error propagation.

The Linux FFI boundary uses the C layout and ioctl encoding for the supported
architectures. Compile-time assertions check the layout. The file descriptor
stays open and the mutable response buffer remains valid for the whole
synchronous call. The request length must match the buffer length, and only
the fixed standard IN requests listed above reach the ioctl.

On a DragonBoard 410c Rev B running Debian 13 on ARM64, the CLI read the complete
18-byte device descriptor of an Intenso Speed Line USB stick (`24a9:205a`).
A test client using the same backend read configuration `01`, which matched
sysfs, and device status `00 00`. The stick's `usb-storage` interface binding
matched before and after the two reads.

Next steps include interface ownership, bounded queues, cancellation, hotplug,
and bulk and interrupt transfers.
