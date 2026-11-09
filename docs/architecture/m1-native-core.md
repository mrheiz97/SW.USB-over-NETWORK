# M1 native core architecture

M1 separates platform-independent inventory types, descriptor parsing, Linux
sysfs discovery, and the command-line application. `swusb-usb-linux` is
intentionally read-only. sysfs provides identity and topology; future transfer
work will use usbfs and narrowly reviewed `USBDEVFS_*` operations.

The descriptor parser consumes length-prefixed records with exact size checks
for standard descriptors. It rejects zero/one-byte records, truncation, wrong
standard sizes, odd string lengths, and invalid UTF-16. Unknown descriptor data
is retained so later class-specific parsers can be added compatibly.

The simulator is a filesystem fixture. Its interface separator is `__` because
Windows cannot check out filenames containing the native sysfs `:` character.
Production Linux sysfs names continue to use `:`.
