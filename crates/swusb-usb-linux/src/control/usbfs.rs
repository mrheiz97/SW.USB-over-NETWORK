use std::{
    ffi::{c_int, c_uint, c_void},
    fs::{File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::fs::FileTypeExt},
    ptr,
};

use super::{Backend, InterfaceOperation, Setup};

#[repr(C)]
struct CtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut c_void,
}

#[cfg(target_env = "musl")]
type IoctlRequest = c_int;
#[cfg(not(target_env = "musl"))]
type IoctlRequest = std::ffi::c_ulong;

// Linux UAPI: _IOWR('U', 0, struct usbdevfs_ctrltransfer).
const USBDEVFS_CONTROL: u32 = (3 << 30) | (ctrl_size() << 16) | (b'U' as u32) << 8;
// Linux UAPI: _IOR('U', 15 or 16, unsigned int).
const USBDEVFS_INTERFACE_BASE: u32 = (2 << 30) | (4 << 16) | (b'U' as u32) << 8;

const fn ctrl_size() -> u32 {
    if size_of::<*mut c_void>() == 8 {
        24
    } else {
        16
    }
}

const _: () = {
    assert!(size_of::<CtrlTransfer>() == ctrl_size() as usize);
    assert!(std::mem::offset_of!(CtrlTransfer, timeout) == 8);
    assert!(std::mem::offset_of!(CtrlTransfer, data) == if ctrl_size() == 24 { 16 } else { 12 });
    assert!(size_of::<c_uint>() == 4);
};

#[allow(unsafe_code)]
unsafe extern "C" {
    fn ioctl(fd: c_int, request: IoctlRequest, ...) -> c_int;
}

pub(super) struct Device(File);

enum Request<'a> {
    Control(&'a mut CtrlTransfer),
    Interface(InterfaceOperation, &'a c_uint),
}

impl Device {
    pub(super) fn open(path: &str) -> io::Result<Self> {
        // usbdev_do_ioctl requires FMODE_WRITE even for IN requests.
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        if !file.metadata()?.file_type().is_char_device() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usbfs path is not a character device",
            ));
        }
        Ok(Self(file))
    }

    fn ioctl(&self, request: Request<'_>) -> io::Result<usize> {
        let (code, arg): (u32, *const c_void) = match request {
            Request::Control(transfer) => (USBDEVFS_CONTROL, ptr::from_mut(transfer).cast()),
            Request::Interface(operation, interface) => {
                let number = match operation {
                    InterfaceOperation::Claim => 15,
                    InterfaceOperation::Release => 16,
                };
                (
                    USBDEVFS_INTERFACE_BASE | number,
                    ptr::from_ref(interface).cast(),
                )
            }
        };
        // ioctl takes signed int on musl, so preserve the high direction bits.
        #[cfg(target_env = "musl")]
        let code = IoctlRequest::from_ne_bytes(code.to_ne_bytes());
        #[cfg(not(target_env = "musl"))]
        let code = IoctlRequest::from(code);
        // SAFETY: this file stays open and Request selects the UAPI code and
        // its matching aligned argument. Control's writable buffer covers
        // wLength bytes. Interface arguments are read-only unsigned ints.
        // The synchronous ioctl retains none of these pointers.
        #[allow(unsafe_code)]
        let result = unsafe { ioctl(self.0.as_raw_fd(), code, arg) };
        usize::try_from(result).map_err(|_| io::Error::last_os_error())
    }
}

impl Backend for Device {
    fn submit(&mut self, setup: Setup, timeout: u32, data: &mut [u8]) -> io::Result<usize> {
        let length = u16::try_from(data.len())
            .ok()
            .filter(|&length| length == setup.length)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "USB buffer length mismatch")
            })?;
        let mut transfer = CtrlTransfer {
            request_type: setup.request_type,
            request: setup.request,
            value: setup.value,
            index: setup.index,
            length,
            timeout,
            data: data.as_mut_ptr().cast(),
        };
        self.ioctl(Request::Control(&mut transfer))
    }

    fn change_claim(&mut self, operation: InterfaceOperation, interface: u8) -> io::Result<()> {
        self.ioctl(Request::Interface(operation, &c_uint::from(interface)))?;
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn non_usb_character_device_rejects_ioctl() {
    use super::ControlRead;

    let mut data = [0; 18];
    let mut device = Device::open("/dev/null").unwrap();
    let error = device
        .submit(ControlRead::DeviceDescriptor.setup(), 500, &mut data)
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(25)); // Linux ENOTTY
}
