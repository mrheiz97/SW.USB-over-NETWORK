use std::{
    ffi::{c_int, c_uint, c_void},
    fs::{File, OpenOptions},
    io::{self, Read, Seek},
    os::{fd::AsRawFd, unix::fs::FileTypeExt},
    ptr,
};

use super::{Backend, InterfaceOperation, Setup};
use crate::endpoint::EndpointReadPlan;

#[path = "usbfs/topology.rs"]
mod topology;

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

#[repr(C)]
struct EndpointTransfer {
    endpoint: c_uint,
    length: c_uint,
    timeout: c_uint,
    data: *mut c_void,
}

#[cfg(target_env = "musl")]
type IoctlRequest = c_int;
#[cfg(not(target_env = "musl"))]
type IoctlRequest = std::ffi::c_ulong;

// Linux UAPI: _IOWR('U', 0, struct usbdevfs_ctrltransfer).
const USBDEVFS_CONTROL: u32 = (3 << 30) | (transfer_size() << 16) | (b'U' as u32) << 8;
// Linux UAPI: _IOWR('U', 2, struct usbdevfs_bulktransfer).
const USBDEVFS_BULK: u32 = USBDEVFS_CONTROL | 2;
// Linux UAPI: _IOR('U', 15 or 16, unsigned int).
const USBDEVFS_INTERFACE_BASE: u32 = (2 << 30) | (4 << 16) | (b'U' as u32) << 8;
// Linux UAPI: _IOW('U', 30, __u32).
const USBDEVFS_DROP_PRIVILEGES: u32 = (1 << 30) | (4 << 16) | (b'U' as u32) << 8 | 30;

const fn transfer_size() -> u32 {
    if size_of::<*mut c_void>() == 8 {
        24
    } else {
        16
    }
}

const _: () = {
    assert!(size_of::<CtrlTransfer>() == transfer_size() as usize);
    assert!(std::mem::offset_of!(CtrlTransfer, timeout) == 8);
    assert!(
        std::mem::offset_of!(CtrlTransfer, data) == if transfer_size() == 24 { 16 } else { 12 }
    );
    assert!(size_of::<EndpointTransfer>() == size_of::<CtrlTransfer>());
    assert!(std::mem::offset_of!(EndpointTransfer, endpoint) == 0);
    assert!(std::mem::offset_of!(EndpointTransfer, length) == 4);
    assert!(std::mem::offset_of!(EndpointTransfer, timeout) == 8);
    assert!(
        std::mem::offset_of!(EndpointTransfer, data) == std::mem::offset_of!(CtrlTransfer, data)
    );
    assert!(size_of::<c_uint>() == 4);
};

#[allow(unsafe_code)]
unsafe extern "C" {
    fn ioctl(fd: c_int, request: IoctlRequest, ...) -> c_int;
}

pub(super) struct Device(File);

enum Request<'a> {
    Control(&'a mut CtrlTransfer),
    Endpoint(&'a mut EndpointTransfer),
    Interface(InterfaceOperation, &'a c_uint),
    SealClaims,
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
        let deny_all_claims = 0_u32;
        let (code, arg): (u32, *const c_void) = match request {
            Request::Control(transfer) => (USBDEVFS_CONTROL, ptr::from_mut(transfer).cast()),
            Request::Endpoint(transfer) => (USBDEVFS_BULK, ptr::from_mut(transfer).cast()),
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
            Request::SealClaims => (
                USBDEVFS_DROP_PRIVILEGES,
                ptr::from_ref(&deny_all_claims).cast(),
            ),
        };
        // ioctl takes signed int on musl, so preserve the high direction bits.
        #[cfg(target_env = "musl")]
        let code = IoctlRequest::from_ne_bytes(code.to_ne_bytes());
        #[cfg(not(target_env = "musl"))]
        let code = IoctlRequest::from(code);
        // SAFETY: this file stays open and Request selects the UAPI code and
        // its matching aligned argument. Both transfer buffers cover their
        // requested writable length. Interface arguments and the zero mask
        // are initialized, read-only 32-bit integers.
        // The synchronous ioctl retains none of these pointers.
        #[allow(unsafe_code)]
        let result = unsafe { ioctl(self.0.as_raw_fd(), code, arg) };
        usize::try_from(result).map_err(|_| io::Error::last_os_error())
    }

    fn control_byte(&mut self, request_type: u8, request: u8, index: u16) -> io::Result<u8> {
        let setup = Setup {
            request_type,
            request,
            value: 0,
            index,
            length: 1,
        };
        let mut data = [0];
        if self.submit(setup, 1000, &mut data)? != data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short USB endpoint preflight response",
            ));
        }
        Ok(data[0])
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

    fn seal_claims(&mut self) -> io::Result<()> {
        self.ioctl(Request::SealClaims)?;
        Ok(())
    }

    fn validate_endpoint(&mut self, plan: EndpointReadPlan) -> io::Result<()> {
        let mut descriptors = vec![0; topology::MAX_DESCRIPTOR_BYTES + 1];
        self.0.rewind()?;
        let actual = self.0.read(&mut descriptors)?;
        descriptors.truncate(actual);
        let configuration = self.control_byte(0x80, 8, 0)?; // GET_CONFIGURATION
        topology::validate(&descriptors, configuration, plan, || {
            self.control_byte(0x81, 10, u16::from(plan.interface)) // GET_INTERFACE
        })
    }

    fn submit_endpoint(
        &mut self,
        plan: EndpointReadPlan,
        timeout: u32,
        data: &mut [u8],
    ) -> io::Result<usize> {
        crate::endpoint::validate_read_parameters(plan.endpoint, data.len())?;
        let length = c_uint::try_from(data.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        let mut transfer = EndpointTransfer {
            endpoint: c_uint::from(plan.endpoint),
            length,
            timeout,
            data: data.as_mut_ptr().cast(),
        };
        self.ioctl(Request::Endpoint(&mut transfer))
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
    let error = device.seal_claims().unwrap_err();
    assert_eq!(error.raw_os_error(), Some(25));
}
