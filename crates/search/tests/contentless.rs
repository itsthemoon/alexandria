use alexandria_search::query;
use rusqlite::ffi;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, Once};

static READS: AtomicUsize = AtomicUsize::new(0);
static DEFAULT: AtomicUsize = AtomicUsize::new(0);
static METHODS: LazyLock<Mutex<HashMap<usize, usize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static INIT: Once = Once::new();

const NAME: &CStr = c"countreads";

static CMETHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(c_close),
    xRead: Some(c_read),
    xWrite: None,
    xTruncate: None,
    xSync: None,
    xFileSize: Some(c_file_size),
    xLock: Some(c_lock),
    xUnlock: Some(c_lock),
    xCheckReservedLock: Some(c_check_reserved),
    xFileControl: Some(c_file_control),
    xSectorSize: Some(c_sector_size),
    xDeviceCharacteristics: Some(c_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

unsafe fn default_vfs() -> *mut ffi::sqlite3_vfs {
    DEFAULT.load(Ordering::SeqCst) as *mut ffi::sqlite3_vfs
}

fn inner(file: *mut ffi::sqlite3_file) -> *mut ffi::sqlite3_io_methods {
    *METHODS.lock().unwrap().get(&(file as usize)).unwrap() as *mut ffi::sqlite3_io_methods
}

fn register() {
    INIT.call_once(|| unsafe {
        let d = ffi::sqlite3_vfs_find(ptr::null());
        DEFAULT.store(d as usize, Ordering::SeqCst);
        let vfs = Box::new(ffi::sqlite3_vfs {
            iVersion: 1,
            szOsFile: 4096,
            mxPathname: 512,
            pNext: ptr::null_mut(),
            zName: NAME.as_ptr(),
            pAppData: d.cast(),
            xOpen: Some(c_open),
            xDelete: Some(c_delete),
            xAccess: Some(c_access),
            xFullPathname: Some(c_full_pathname),
            xDlOpen: None,
            xDlError: None,
            xDlSym: None,
            xDlClose: None,
            xRandomness: Some(c_randomness),
            xSleep: Some(c_sleep),
            xCurrentTime: Some(c_current_time),
            xGetLastError: Some(c_get_last_error),
            xCurrentTimeInt64: None,
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        });
        ffi::sqlite3_vfs_register(Box::into_raw(vfs), 0);
    });
}

unsafe extern "C" fn c_open(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let d = default_vfs();
    let rc = match (*d).xOpen {
        Some(f) => f(d, name, file, flags, out_flags),
        None => ffi::SQLITE_CANTOPEN,
    };
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    let prev = (*file).pMethods.cast_mut();
    METHODS.lock().unwrap().insert(file as usize, prev as usize);
    (*file).pMethods = &CMETHODS;
    ffi::SQLITE_OK
}

unsafe extern "C" fn c_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    off: ffi::sqlite3_int64,
) -> c_int {
    READS.fetch_add(1, Ordering::Relaxed);
    match (*inner(file)).xRead {
        Some(f) => f(file, buf, amt, off),
        None => ffi::SQLITE_IOERR_READ,
    }
}

unsafe extern "C" fn c_close(file: *mut ffi::sqlite3_file) -> c_int {
    let rc = match (*inner(file)).xClose {
        Some(f) => f(file),
        None => ffi::SQLITE_OK,
    };
    METHODS.lock().unwrap().remove(&(file as usize));
    rc
}

unsafe extern "C" fn c_file_size(file: *mut ffi::sqlite3_file, out: *mut ffi::sqlite3_int64) -> c_int {
    match (*inner(file)).xFileSize {
        Some(f) => f(file, out),
        None => ffi::SQLITE_IOERR_FSTAT,
    }
}

unsafe extern "C" fn c_lock(file: *mut ffi::sqlite3_file, l: c_int) -> c_int {
    match (*inner(file)).xLock {
        Some(f) => f(file, l),
        None => ffi::SQLITE_IOERR_LOCK,
    }
}

unsafe extern "C" fn c_check_reserved(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    match (*inner(file)).xCheckReservedLock {
        Some(f) => f(file, out),
        None => ffi::SQLITE_IOERR,
    }
}

unsafe extern "C" fn c_file_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    match (*inner(file)).xFileControl {
        Some(f) => f(file, op, arg),
        None => ffi::SQLITE_NOTFOUND,
    }
}

unsafe extern "C" fn c_sector_size(file: *mut ffi::sqlite3_file) -> c_int {
    match (*inner(file)).xSectorSize {
        Some(f) => f(file),
        None => 4096,
    }
}

unsafe extern "C" fn c_device_characteristics(file: *mut ffi::sqlite3_file) -> c_int {
    match (*inner(file)).xDeviceCharacteristics {
        Some(f) => f(file),
        None => 0,
    }
}

unsafe extern "C" fn c_delete(_: *mut ffi::sqlite3_vfs, name: *const c_char, dir: c_int) -> c_int {
    let d = default_vfs();
    match (*d).xDelete {
        Some(f) => f(d, name, dir),
        None => ffi::SQLITE_IOERR,
    }
}

unsafe extern "C" fn c_access(_: *mut ffi::sqlite3_vfs, name: *const c_char, flags: c_int, out: *mut c_int) -> c_int {
    let d = default_vfs();
    match (*d).xAccess {
        Some(f) => f(d, name, flags, out),
        None => ffi::SQLITE_IOERR,
    }
}

unsafe extern "C" fn c_full_pathname(
    _: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    n: c_int,
    out: *mut c_char,
) -> c_int {
    let d = default_vfs();
    match (*d).xFullPathname {
        Some(f) => f(d, name, n, out),
        None => ffi::SQLITE_CANTOPEN,
    }
}

unsafe extern "C" fn c_randomness(_: *mut ffi::sqlite3_vfs, n: c_int, out: *mut c_char) -> c_int {
    let d = default_vfs();
    match (*d).xRandomness {
        Some(f) => f(d, n, out),
        None => 0,
    }
}

unsafe extern "C" fn c_sleep(_: *mut ffi::sqlite3_vfs, us: c_int) -> c_int {
    let d = default_vfs();
    match (*d).xSleep {
        Some(f) => f(d, us),
        None => 0,
    }
}

unsafe extern "C" fn c_current_time(_: *mut ffi::sqlite3_vfs, out: *mut f64) -> c_int {
    let d = default_vfs();
    match (*d).xCurrentTime {
        Some(f) => f(d, out),
        None => ffi::SQLITE_ERROR,
    }
}

unsafe extern "C" fn c_get_last_error(_: *mut ffi::sqlite3_vfs, _: c_int, _: *mut c_char) -> c_int {
    0
}

fn build_contentless(path: &std::path::Path) {
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=OFF;
         CREATE VIRTUAL TABLE articles USING fts5(title, body, content='',
         tokenize='unicode61 remove_diacritics 2');
         CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);",
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    for i in 0..20_000i64 {
        tx.execute(
            "INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![i + 1, format!("Filler {i}"), "water water water".to_string()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO docs(id, title, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![i + 1, format!("Filler {i}"), format!("Filler_{i}-{}", "x".repeat(2000))],
        )
        .unwrap();
    }
    tx.execute(
        "INSERT INTO articles(rowid, title, body) VALUES (2000000, 'Water', 'Water is a chemical compound.')",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO docs(id, title, url) VALUES (2000000, 'Water', 'Water')",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    c.execute_batch("INSERT INTO articles(articles) VALUES('optimize');").unwrap();
}

#[test]
fn query_bounds_docs_lookups_on_a_contentless_index() {
    let dir = std::env::temp_dir().join(format!(
        "alex-contentless-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("idx.db");
    build_contentless(&db);
    register();
    let path = db.to_string_lossy().into_owned();
    let conn = rusqlite::Connection::open_with_flags_and_vfs(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        "countreads",
    )
    .unwrap();
    READS.store(0, Ordering::Relaxed);
    let hits = query(&conn, "water").unwrap();
    let reads = READS.load(Ordering::Relaxed);
    assert_eq!(hits[0].title, "Water");
    assert!(hits.len() <= 40, "{}", hits.len());
    assert!(
        reads < 7000,
        "query performed {reads} file reads; it should not scan every matching docs row"
    );
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}
