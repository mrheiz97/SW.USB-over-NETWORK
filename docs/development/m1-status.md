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

For repeated reads, `ControlDevice` keeps one usbfs file descriptor open.
After unplugging, that handle stays associated with the disconnected device.
Enumerate again and open a new handle after reconnecting, even if Linux
reuses the bus and device numbers.

`Cancellation` skips a read if cancellation is observed before submission.
If it is observed while the synchronous ioctl runs, the caller receives
`Interrupted` after the ioctl returns and its result is discarded. Cancellation
does not interrupt the kernel transfer itself. The configured USB timeout
still applies.

Linux requires a read/write file descriptor for the ioctl. Prefer narrow
device-node permissions. The transfer path leaves interface and driver bindings
alone. Simulator tests cover setup fields, responses, unplug and reconnect at
the same address, recovery after a timeout, and cancellation before and during
submission.

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

On the same board, unplugging the stick caused its open handle to return
`ENODEV`. The old handle still returned `ENODEV` after reconnecting.
A new handle read configuration `01` and status `00 00` with `usb-storage`
bound. Cancellation before submission returned `Interrupted`, and the
next read succeeded. Twelve reads with a 1 ms timeout all completed
successfully, so the hardware test did not reproduce a timeout.

A separate ARM64 test on the board used `strace` fault injection to make the
first usbfs ioctl return `ETIMEDOUT`. The API reported `TimedOut` and a second
read on the same handle succeeded. In a second run, the USB read succeeded in
the kernel and `strace` delayed delivery of its result. A cancellation issued
during that delay caused the API to discard the result and return
`Interrupted`. A subsequent read succeeded and `usb-storage` remained
bound. These tests cover application behavior at the syscall boundary.

`ControlQueue` holds up to 16 pending reads on one connection. The caller
advances the queue in FIFO order. Each read has its own cancellation signal
and USB timeout. The timeout starts at submission, and cancellation does not
free a slot until the caller dequeues the request. The `read-controls` command
uses this queue for all three standard reads.
On the DragonBoard, `read-controls` returned the Intenso stick's descriptor,
configuration `01`, and status `00 00` with `usb-storage` bound throughout.

## M2 interface ownership

Claim and release are explicit operations on the same open usbfs handle used
for control reads. A successful claim is recorded once. Duplicate claims
and releases of unclaimed interfaces are rejected locally. Failed operations
leave ownership bookkeeping unchanged, so a failed release can be retried.
Closing the handle lets usbfs release any remaining claims.

The Linux backend uses only `USBDEVFS_CLAIMINTERFACE` and
`USBDEVFS_RELEASEINTERFACE` for these operations. Both synchronously read an
initialized, naturally aligned 32-bit `unsigned int` from userspace. The
file descriptor and argument remain valid until the ioctl returns. The FFI wrapper
pairs each request with its typed argument, and preserves the OS error.
An interface already bound to a driver stays with that driver.

Simulator tests cover exclusive ownership, release and reacquisition, cleanup
on close, a busy kernel driver, and retry after a failed release. The DragonBoard
regression test repeated the three standard reads with `usb-storage` bound.
Claim and release were exercised only in the simulator.

Next steps include cancellation of in-flight kernel transfers, and bulk and
interrupt transfers.
