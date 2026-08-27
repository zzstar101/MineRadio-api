use std::{
    ffi::{CStr, CString},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

const CX_SET_BODY: usize = 0;
const CX_SET_UIN: usize = 1;
const CX_SET_GUID: usize = 2;
const CX_REAL_X9: usize = 4;
const CX_FREE: usize = 6;
const CX_REAL_X4: usize = 8;
const CX_SODA_SIGN: usize = 11;
const CX_SODA_SET_BIN: usize = 14;
const CX_SODA_SET_DEVICE: usize = 16;
const CX_SODA_INIT: usize = 17;
const CX_SODA_LAST_ERROR: usize = 19;
const CSIGNER_TABLE_LEN: usize = 24;

type CxFn = unsafe extern "C" fn(u64, u64) -> u64;

struct CxLib {
    /// 保活
    _library: libloading::Library,
    table: Box<[CxFn; CSIGNER_TABLE_LEN]>,
    /// C++ 内部状态为单实例且非线程安全，用它串行化所有有状态调用。
    lock: Mutex<()>,
    /// bundle 路径（与动态库同目录）。
    bin_path: PathBuf,
}

// 库句柄本身不可 Send/Sync；函数表只读、有状态操作由 Mutex 保护，可安全跨线程。
unsafe impl Send for CxLib {}
unsafe impl Sync for CxLib {}

static CSIGNER: OnceLock<Result<CxLib, String>> = OnceLock::new();
static SODA_SETUP: OnceLock<Result<(), String>> = OnceLock::new();

pub fn init() -> Result<(), String> {
    let lib = cx_lib()?;
    let _ = SODA_SETUP.get_or_init(|| setup_soda(lib));
    Ok(())
}

fn try_lib() -> Result<&'static CxLib, String> {
    CSIGNER
        .get()
        .and_then(|result| result.as_ref().ok())
        .ok_or_else(|| "csigner 未初始化：请先通过 ApiInner::new / csigner::init 加载".to_owned())
}

fn cx_lib() -> Result<&'static CxLib, String> {
    CSIGNER.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn load() -> Result<CxLib, String> {
    let lib_name = option_env!("CSIGNER_LIB_FILENAME").unwrap_or("csigner.bin");
    let path = resolve_lib_path(lib_name)?;

    let library = unsafe { libloading::Library::new(&path) }
        .map_err(|err| format!("csigner: 加载 {path:?} 失败: {err}"))?;

    let get_table: libloading::Symbol<unsafe extern "C" fn() -> *const CxFn> =
        unsafe { library.get(b"lIlI") }
            .map_err(|err| format!("csigner: 解析符号 lIlI 失败: {err}"))?;
    let table_ptr = unsafe { get_table() };
    if table_ptr.is_null() {
        return Err("csigner: lIlI 返回空指针".to_owned());
    }
    let mut table = Box::new([__dummy_fn as CxFn; CSIGNER_TABLE_LEN]);

    unsafe { std::ptr::copy_nonoverlapping(table_ptr, table.as_mut_ptr(), CSIGNER_TABLE_LEN) };

    let bin_path = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join("sign.bin"))
        .unwrap_or_else(|| PathBuf::from("sign.bin"));
    Ok(CxLib {
        _library: library,
        table,
        lock: Mutex::new(()),
        bin_path,
    })
}

fn setup_soda(lib: &CxLib) -> Result<(), String> {
    const SODA_DEVICE_ID: &str = "3753066532709850";
    let _guard = lib
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    set_string(lib, CX_SODA_SET_BIN, &lib.bin_path.to_string_lossy());
    set_string(lib, CX_SODA_SET_DEVICE, SODA_DEVICE_ID);
    if lib.call(CX_SODA_INIT, 0, 0) == 1 {
        Ok(())
    } else {
        Err(soda_last_error(lib))
    }
}

fn soda_last_error(lib: &CxLib) -> String {
    let ptr = lib.call(CX_SODA_LAST_ERROR, 0, 0);
    if ptr == 0 {
        return String::new();
    }

    let msg = unsafe { CStr::from_ptr(ptr as *const std::ffi::c_char) }
        .to_string_lossy()
        .into_owned();
    lib.call(CX_FREE, ptr, 0);
    msg
}

unsafe extern "C" fn __dummy_fn(_: u64, _: u64) -> u64 {
    0
}

fn resolve_lib_path(name: &str) -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("csigner: '{}' not found", name))
}

impl CxLib {
    fn call(&self, index: usize, a: u64, b: u64) -> u64 {
        debug_assert!(index < CSIGNER_TABLE_LEN);
        unsafe { (self.table[index])(a, b) }
    }
}

fn set_string(lib: &CxLib, index: usize, value: &str) {
    lib.call(index, value.as_ptr() as u64, value.len() as u64);
}

fn take_result(lib: &CxLib, ptr: u64) -> Result<(Vec<u8>, Vec<u8>), String> {
    if ptr == 0 {
        return Err("csigner: 签名函数返回空指针".to_owned());
    }
    let raw = ptr as *const u8;
    let n = unsafe { std::ptr::read_unaligned(raw.cast::<u64>()) } as usize;
    let m = unsafe { std::ptr::read_unaligned(raw.add(8).cast::<u64>()) } as usize;
    if n > isize::MAX as usize || m > isize::MAX as usize {
        return Err("csigner: 签名结果长度异常".to_owned());
    }
    let first = unsafe { std::slice::from_raw_parts(raw.add(16), n) }.to_vec();
    let second = unsafe { std::slice::from_raw_parts(raw.add(16 + n), m) }.to_vec();
    lib.call(CX_FREE, ptr, 0);
    Ok((first, second))
}

pub fn real_x9(body: &str) -> Result<String, String> {
    let lib = try_lib()?;
    let _guard = lib
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let seed = lib.call(3, body.as_ptr() as u64, body.len() as u64);
    let mixed = lib.call(10, seed, body.len() as u64);
    lib.call(7, body.as_ptr() as u64, body.len() as u64);
    set_string(lib, CX_SET_BODY, body);
    let tag = lib.call(15, mixed, seed);
    let ptr = lib.call(CX_REAL_X9, 0, 0);
    let _ = lib.call(20, tag, mixed);
    let (sign, _) = take_result(lib, ptr)?;
    String::from_utf8(sign).map_err(|err| format!("csigner: x9 结果不是合法 UTF-8: {err}"))
}

pub fn set_x4_identity(uin: &str, guid: &str) -> Result<(), String> {
    let lib = try_lib()?;
    let _guard = lib
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let h = lib.call(9, uin.as_ptr() as u64, uin.len() as u64);
    lib.call(18, uin.as_ptr() as u64, uin.len() as u64);
    set_string(lib, CX_SET_UIN, uin);
    let h2 = lib.call(13, h, guid.len() as u64);
    set_string(lib, CX_SET_GUID, guid);
    let _ = lib.call(23, h2, h);
    Ok(())
}

pub fn real_x4(body: &str, ts: u64) -> Result<(String, String), String> {
    let lib = try_lib()?;
    let _guard = lib
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let seed = lib.call(9, ts, body.len() as u64);
    lib.call(7, body.as_ptr() as u64, body.len() as u64);
    set_string(lib, CX_SET_BODY, body);
    let mixed = lib.call(15, seed, ts);
    let ptr = lib.call(CX_REAL_X4, 0, ts);
    let _ = lib.call(21, mixed, seed);
    let (j, m) = take_result(lib, ptr)?;
    let j =
        String::from_utf8(j).map_err(|err| format!("csigner: x4 j 结果不是合法 UTF-8: {err}"))?;
    let m =
        String::from_utf8(m).map_err(|err| format!("csigner: x4 m 结果不是合法 UTF-8: {err}"))?;
    Ok((j, m))
}

pub fn real_soda_sign(url: &str, headers: &str) -> Result<String, String> {
    let lib = try_lib()?;
    let url = CString::new(url).map_err(|_| "csigner: URL 含 NUL".to_owned())?;
    let headers = CString::new(headers).map_err(|_| "csigner: headers 含 NUL".to_owned())?;
    let _guard = lib
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let chk = lib.call(3, url.as_ptr() as u64, url.as_bytes().len() as u64);
    let mixed = lib.call(10, chk, headers.as_bytes().len() as u64);
    lib.call(7, url.as_ptr() as u64, url.as_bytes().len() as u64);
    let ptr = lib.call(CX_SODA_SIGN, url.as_ptr() as u64, headers.as_ptr() as u64);
    let _ = lib.call(20, mixed, chk);
    if ptr == 0 {
        return Err(soda_last_error(lib));
    }
    let (sign, _) = take_result(lib, ptr)?;
    String::from_utf8(sign).map_err(|err| format!("csigner: 签名结果不是合法 UTF-8: {err}"))
}
