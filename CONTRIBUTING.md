# Contributing

Use stable Rust, keep modules small, and add regression tests for malformed
input. Before submitting changes, run formatting, Clippy with warnings denied,
all workspace tests, and `scripts/check-private-files.sh` (or its PowerShell
equivalent). Never commit `AGENTS.md`, credentials, keys, or USB payload data
captured from users. Hardware-dependent changes must document device, kernel,
permissions, and whether a kernel driver was bound.
