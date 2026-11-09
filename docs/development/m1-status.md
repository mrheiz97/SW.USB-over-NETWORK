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

## M2 next steps

1. Design a bounded transfer request and completion model with explicit IDs.
2. Add Linux-only usbfs wrappers for read-only control transfers first.
3. Define interface claim/release ownership without automatic driver detach.
4. Implement bounded queues, monotonic deadlines, cancellation, and hotplug.
5. Add mock-ioctl tests, then isolated hardware tests with harmless devices.
6. Add bulk and interrupt transfers only after control-path review.
