use super::*;

#[test]
fn reads_standard_requests_from_simulated_hid() {
    let descriptor = [18, 1, 0x10, 2, 0, 0, 0, 64, 9, 0x12, 1, 0, 0, 1, 1, 2, 3, 1];
    let cases: [(ControlRead, u8, u16, &[u8]); 3] = [
        (ControlRead::DeviceDescriptor, 6, 0x0100, &descriptor),
        (ControlRead::ConfigurationValue, 8, 0, &[1]),
        (ControlRead::DeviceStatus, 0, 0, &[0, 0]),
    ];
    for (operation, request, value, response) in cases {
        let data = execute(operation, 500, |setup, timeout, buffer| {
            assert_eq!(
                setup,
                Setup {
                    request_type: 0x80,
                    request,
                    value,
                    index: 0,
                    length: u16::try_from(response.len()).unwrap(),
                }
            );
            assert_eq!(timeout, 500);
            buffer.copy_from_slice(response);
            Ok(response.len())
        })
        .unwrap();
        assert_eq!(data, response);
    }
}

#[test]
fn rejects_short_oversize_and_malformed_responses() {
    for length in [0, 17, 18, 19] {
        // At the correct length the all-zero descriptor header is still invalid.
        let error = execute(ControlRead::DeviceDescriptor, 500, |_, _, _| Ok(length)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn propagates_simulated_timeout_and_disconnect() {
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::NotConnected] {
        let error =
            execute(ControlRead::DeviceStatus, 500, |_, _, _| Err(kind.into())).unwrap_err();
        assert_eq!(error.kind(), kind);
    }
}

#[test]
fn rejects_invalid_inputs_before_submitting() {
    for timeout in [0, 5001] {
        let error = execute(ControlRead::DeviceStatus, timeout, |_, _, _| {
            panic!("must not submit an invalid request")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    for (bus, address) in [(0, 1), (1000, 1), (1, 0), (1, 128)] {
        assert_eq!(
            read(bus, address, ControlRead::DeviceStatus, 500)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
