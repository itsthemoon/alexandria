use super::{Shared, REGISTRY};
use rusqlite::ffi;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::Once;

pub(crate) const NAME: &str = "alexverified";
const NAME_C: &CStr = c"alexverified";

#[repr(C)]
struct VFile {
    base: ffi::sqlite3_file,
    reader: *mut Shared,
}

static INIT: Once = Once::new();
static mut REGISTERED: c_int = ffi::SQLITE_ERROR;

pub(crate) fn register() -> Result<(), String> {
    INIT.call_once(|| unsafe {
        let default = ffi::sqlite3_vfs_find(std::ptr::null());
        if default.is_null() {
            return;
        }
        let vfs = Box::new(ffi::sqlite3_vfs {
            iVersion: 1,
            szOsFile: std::mem::size_of::<VFile>() as c_int,
            mxPathname: 512,
            pNext: std::ptr::null_mut(),
            zName: NAME_C.as_ptr(),
            pAppData: default.cast(),
            xOpen: Some(x_open),
            xDelete: Some(x_delete),
            xAccess: Some(x_access),
            xFullPathname: Some(x_full_pathname),
            xDlOpen: None,
            xDlError: None,
            xDlSym: None,
            xDlClose: None,
            xRandomness: Some(x_randomness),
            xSleep: Some(x_sleep),
            xCurrentTime: Some(x_current_time),
            xGetLastError: Some(x_get_last_error),
            xCurrentTimeInt64: None,
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        });
        REGISTERED = ffi::sqlite3_vfs_register(Box::into_raw(vfs), 0);
    });
    if unsafe { REGISTERED } == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err("could not register the verified search reader".into())
    }
}

static METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(f_close),
    xRead: Some(f_read),
    xWrite: Some(f_write),
    xTruncate: Some(f_truncate),
    xSync: Some(f_sync),
    xFileSize: Some(f_file_size),
    xLock: Some(f_lock),
    xUnlock: Some(f_lock),
    xCheckReservedLock: Some(f_check_reserved),
    xFileControl: Some(f_file_control),
    xSectorSize: Some(f_sector_size),
    xDeviceCharacteristics: Some(f_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

fn default_vfs(vfs: *mut ffi::sqlite3_vfs) -> *mut ffi::sqlite3_vfs {
    unsafe { (*vfs).pAppData.cast() }
}

unsafe extern "C" fn x_open(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    (*file).pMethods = std::ptr::null();
    if name.is_null()
        || flags & ffi::SQLITE_OPEN_MAIN_DB == 0
        || flags & (ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE) != 0
    {
        return ffi::SQLITE_CANTOPEN;
    }
    let token = match CStr::from_ptr(name).to_str() {
        Ok(t) => t,
        Err(_) => return ffi::SQLITE_CANTOPEN,
    };
    let reader = match REGISTRY
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(token).cloned()))
    {
        Some(r) => r,
        None => return ffi::SQLITE_CANTOPEN,
    };
    let vf = file.cast::<VFile>();
    std::ptr::write(
        vf,
        VFile {
            base: ffi::sqlite3_file { pMethods: &METHODS },
            reader: Box::into_raw(Box::new(reader)),
        },
    );
    if !out_flags.is_null() {
        *out_flags = ffi::SQLITE_OPEN_READONLY | ffi::SQLITE_OPEN_MAIN_DB;
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_delete(_: *mut ffi::sqlite3_vfs, _: *const c_char, _: c_int) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn x_access(
    _: *mut ffi::sqlite3_vfs,
    _: *const c_char,
    _: c_int,
    out: *mut c_int,
) -> c_int {
    *out = 0;
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_full_pathname(
    _: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let src = CStr::from_ptr(name).to_bytes_with_nul();
    if src.len() > n_out as usize {
        return ffi::SQLITE_CANTOPEN;
    }
    std::ptr::copy_nonoverlapping(src.as_ptr().cast(), out, src.len());
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_randomness(vfs: *mut ffi::sqlite3_vfs, n: c_int, out: *mut c_char) -> c_int {
    let d = default_vfs(vfs);
    match (*d).xRandomness {
        Some(f) => f(d, n, out),
        None => 0,
    }
}

unsafe extern "C" fn x_sleep(vfs: *mut ffi::sqlite3_vfs, us: c_int) -> c_int {
    let d = default_vfs(vfs);
    match (*d).xSleep {
        Some(f) => f(d, us),
        None => 0,
    }
}

unsafe extern "C" fn x_current_time(vfs: *mut ffi::sqlite3_vfs, out: *mut f64) -> c_int {
    let d = default_vfs(vfs);
    match (*d).xCurrentTime {
        Some(f) => f(d, out),
        None => ffi::SQLITE_ERROR,
    }
}

unsafe extern "C" fn x_get_last_error(_: *mut ffi::sqlite3_vfs, _: c_int, _: *mut c_char) -> c_int {
    0
}

unsafe extern "C" fn f_close(file: *mut ffi::sqlite3_file) -> c_int {
    let vf = file.cast::<VFile>();
    if !(*vf).reader.is_null() {
        drop(Box::from_raw((*vf).reader));
        (*vf).reader = std::ptr::null_mut();
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn f_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    off: ffi::sqlite3_int64,
) -> c_int {
    if amt < 0 || off < 0 {
        return ffi::SQLITE_IOERR_READ;
    }
    let vf = file.cast::<VFile>();
    let out = std::slice::from_raw_parts_mut(buf.cast::<u8>(), amt as usize);
    let got = match (*(*vf).reader).lock() {
        Ok(mut r) => r.read(off as u64, out),
        Err(_) => None,
    };
    match got {
        Some(n) if n == out.len() => ffi::SQLITE_OK,
        Some(n) => {
            out[n..].fill(0);
            ffi::SQLITE_IOERR_SHORT_READ
        }
        None => ffi::SQLITE_IOERR_READ,
    }
}

unsafe extern "C" fn f_write(
    _: *mut ffi::sqlite3_file,
    _: *const c_void,
    _: c_int,
    _: ffi::sqlite3_int64,
) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn f_truncate(_: *mut ffi::sqlite3_file, _: ffi::sqlite3_int64) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn f_sync(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn f_file_size(file: *mut ffi::sqlite3_file, out: *mut ffi::sqlite3_int64) -> c_int {
    let vf = file.cast::<VFile>();
    match (*(*vf).reader).lock() {
        Ok(r) => {
            *out = r.size as ffi::sqlite3_int64;
            ffi::SQLITE_OK
        }
        Err(_) => ffi::SQLITE_IOERR_FSTAT,
    }
}

unsafe extern "C" fn f_lock(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn f_check_reserved(_: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    *out = 0;
    ffi::SQLITE_OK
}

unsafe extern "C" fn f_file_control(_: *mut ffi::sqlite3_file, _: c_int, _: *mut c_void) -> c_int {
    ffi::SQLITE_NOTFOUND
}

unsafe extern "C" fn f_sector_size(_: *mut ffi::sqlite3_file) -> c_int {
    4096
}

unsafe extern "C" fn f_device_characteristics(_: *mut ffi::sqlite3_file) -> c_int {
    ffi::SQLITE_IOCAP_IMMUTABLE
}
