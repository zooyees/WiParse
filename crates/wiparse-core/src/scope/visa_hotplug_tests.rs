use super::*;
use std::cell::RefCell;
use std::collections::VecDeque;

thread_local! {
    static READS: RefCell<VecDeque<(ViStatus, Vec<u8>)>> = RefCell::default();
    static CLOSED: RefCell<Vec<u32>> = RefCell::default();
}

unsafe extern "system" fn read(_: u32, buffer: *mut u8, size: u32, count: *mut u32) -> i32 {
    let (status, bytes) = READS.with(|reads| reads.borrow_mut().pop_front().expect("unexpected read"));
    assert!(bytes.len() <= size as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len());
        *count = bytes.len() as u32;
    }
    status
}
unsafe extern "system" fn close(session: u32) -> i32 {
    CLOSED.with(|closed| closed.borrow_mut().push(session));
    VI_SUCCESS
}
unsafe extern "system" fn set(_: u32, _: u32, _: u64) -> i32 { VI_SUCCESS }
unsafe extern "system" fn write(_: u32, _: *const u8, size: u32, count: *mut u32) -> i32 {
    unsafe { *count = size; }
    VI_SUCCESS
}
unsafe extern "system" fn rm(_: *mut u32) -> i32 { unreachable!() }
unsafe extern "system" fn find(_: u32, _: *const c_char, _: *mut u32, _: *mut u32, _: *mut c_char) -> i32 { unreachable!() }
unsafe extern "system" fn next(_: u32, _: *mut c_char) -> i32 { unreachable!() }
unsafe extern "system" fn open(_: u32, _: *const c_char, _: u32, _: u32, _: *mut u32) -> i32 { unreachable!() }

fn fixture(reads: Vec<(ViStatus, Vec<u8>)>) -> Instrument {
    READS.with(|queue| *queue.borrow_mut() = reads.into());
    CLOSED.with(|closed| closed.borrow_mut().clear());
    Instrument {
        api: Arc::new(VisaApi {
            // Only retain a harmless library owner; all I/O uses the stubs above.
            _lib: unsafe { Library::new("kernel32.dll").unwrap() },
            source: "test".into(), open_default_rm: rm, find_rsrc: find,
            find_next: next, open, close, write, read, set_attribute: set,
            get_attribute: None, clear: None, status_desc: None,
        }),
        session: 42,
        connection_lost: AtomicBool::new(false),
    }
}

#[test]
fn partial_timeout_keeps_bytes_without_invalidating_session() {
    let instrument = fixture(vec![(VI_ERROR_TMO, b"RIGOL,DS1054Z,SN,1\n".to_vec())]);
    assert!(instrument.read_bytes(128).unwrap().starts_with(b"RIGOL"));
    assert!(!instrument.connection_lost());
}

#[test]
fn partial_hotplug_errors_are_not_success_and_close_handle() {
    for status in [VI_ERROR_CONN_LOST, VI_ERROR_INV_OBJECT, VI_ERROR_RSRC_NFOUND, VI_ERROR_IO] {
        let instrument = fixture(vec![(status, b"partial".to_vec())]);
        assert!(instrument.read_bytes(128).is_err());
        assert!(instrument.connection_lost());
        drop(instrument);
        CLOSED.with(|closed| assert_eq!(*closed.borrow(), vec![42]));
    }
}

#[test]
fn disconnect_mid_binary_block_aborts_without_retrying_or_returning_partial_data() {
    let instrument = fixture(vec![
        (VI_SUCCESS, b"#21012345".to_vec()),
        (VI_ERROR_CONN_LOST, Vec::new()),
        (VI_SUCCESS, b"must not read".to_vec()),
    ]);
    assert!(instrument.read_raw().is_err());
    READS.with(|queue| assert_eq!(queue.borrow().len(), 1));
}

#[test]
fn optional_timeout_and_unsupported_attribute_do_not_latch_disconnect() {
    let instrument = fixture(Vec::new());
    for status in [VI_ERROR_TMO, VI_ERROR_NSUP_ATTR, VI_ERROR_NSUP_ATTR_STATE] {
        assert!(instrument.check_io(status).is_err());
        assert!(!instrument.connection_lost());
    }
}
