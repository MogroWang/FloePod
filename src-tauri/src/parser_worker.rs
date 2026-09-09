//! Resource-limited parser process. No database, WebView, network, or plaintext temporary input file.
use crate::privacy::metadata::{ScanError, MAX_INPUT_BYTES};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::os::windows::{
    io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::CommandExt,
};
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::JobObjects::*;

const FLAG: &str = "--floepod-parser-worker";
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;
const RESPONSE_MARKER: &[u8] = b"FLOEPOD_PARSER_RESPONSE:";
static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
pub enum Operation {
    Inspect {
        display_path: PathBuf,
    },
    Clean {
        source: PathBuf,
        output: PathBuf,
    },
    PdfText {
        source: PathBuf,
    },
    OfficeText {
        source: PathBuf,
    },
    #[cfg(test)]
    Sleep,
    #[cfg(test)]
    CheckMemoryLimit,
}
#[derive(Serialize, Deserialize)]
struct Request {
    operation: Operation,
    input_len: usize,
}

fn failed(error: impl std::fmt::Display) -> ScanError {
    ScanError::Failed(error.to_string())
}

pub fn invoke<T: DeserializeOwned>(operation: Operation, input: Vec<u8>) -> Result<T, ScanError> {
    invoke_with_timeout(operation, input, Duration::from_secs(15))
}

pub(crate) fn invoke_with_timeout<T: DeserializeOwned>(
    operation: Operation,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<T, ScanError> {
    let started = Instant::now();
    let _serial = loop {
        if started.elapsed() >= timeout {
            return Err(ScanError::Skipped("等待解析超过时间限制".into()));
        }
        match SERIAL.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => return Err(failed(error)),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(20))
            }
        }
    };
    if input.len() as u64 > MAX_INPUT_BYTES {
        return Err(ScanError::Skipped("解析输入超过 32 MiB".into()));
    }
    let request = serde_json::to_vec(&Request {
        operation,
        input_len: input.len(),
    })
    .map_err(failed)?;
    if request.len() > 8192 {
        return Err(failed("解析请求路径过长"));
    }
    let mut command = Command::new(std::env::current_exe().map_err(failed)?);
    #[cfg(not(test))]
    command.arg(FLAG);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "parser_worker::tests::worker_entry",
            "--nocapture",
        ])
        .env("FLOEPOD_TEST_PARSER_WORKER", "1");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(failed)?;
    // The child waits for stdin. Limits must be installed before it receives any parser input.
    let job = match limit_process(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| failed("缺少解析输入管道"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("缺少解析输出管道"))?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(&request)?;
        stdin.write_all(b"\n")?;
        stdin.write_all(&input)
    });
    let reader = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stdout.take(MAX_RESPONSE + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(failed("解析进程异常终止或超过内存/CPU 限制，未完成检查"))
                }
            }
            Err(error) => break Err(failed(error)),
            Ok(None) if started.elapsed() >= timeout => {
                break Err(ScanError::Skipped("解析超过时间限制，未完成检查".into()))
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // KILL_ON_JOB_CLOSE also closes inherited pipes on every error/timeout path.
    drop(job);
    let _ = child.wait();
    let written = writer.join().map_err(|_| failed("解析输入任务异常终止"))?;
    let response = reader
        .join()
        .map_err(|_| failed("解析输出任务异常终止"))?
        .map_err(failed)?;
    outcome?;
    written.map_err(failed)?;
    if response.len() as u64 > MAX_RESPONSE {
        return Err(ScanError::Skipped("解析输出超过大小限制".into()));
    }
    let offset = response
        .windows(RESPONSE_MARKER.len())
        .position(|bytes| bytes == RESPONSE_MARKER)
        .ok_or_else(|| failed("解析进程没有返回完整结果"))?
        + RESPONSE_MARKER.len();
    let response: Result<Value, ScanError> =
        serde_json::from_slice(&response[offset..]).map_err(failed)?;
    serde_json::from_value(response?).map_err(failed)
}

fn limit_process(child: &std::process::Child) -> Result<OwnedHandle, ScanError> {
    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            return Err(failed(std::io::Error::last_os_error()));
        }
        let job = OwnedHandle::from_raw_handle(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_PROCESS_TIME
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.BasicLimitInformation.PerProcessUserTimeLimit = 10 * 10_000_000;
        limits.ProcessMemoryLimit = 512 * 1024 * 1024;
        if SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
            || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
        {
            return Err(failed(format!(
                "无法安装解析资源限制: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(job)
    }
}

pub fn run_if_requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) || args.next().is_some() {
        return false;
    }
    worker_main();
    true
}

fn worker_main() {
    let response = (|| -> Result<Value, ScanError> {
        let mut stdin = std::io::BufReader::new(std::io::stdin().lock());
        let mut line = Vec::new();
        stdin
            .by_ref()
            .take(8194)
            .read_until(b'\n', &mut line)
            .map_err(failed)?;
        if line.len() > 8193 || line.last() != Some(&b'\n') {
            return Err(failed("解析请求无效"));
        }
        let request: Request = serde_json::from_slice(&line).map_err(failed)?;
        if request.input_len as u64 > MAX_INPUT_BYTES {
            return Err(failed("解析输入超限"));
        }
        let mut input = Vec::new();
        stdin
            .take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut input)
            .map_err(failed)?;
        if input.len() != request.input_len {
            return Err(failed("解析输入长度不一致"));
        }
        match request.operation {
            Operation::Inspect { display_path } => {
                let mut issues = Vec::new();
                crate::privacy::metadata::inspect(&input, &display_path, &mut issues)?;
                serde_json::to_value(issues).map_err(failed)
            }
            Operation::Clean { source, output } => {
                let ext = source
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let removed = match ext.as_str() {
                    "pdf" => crate::privacy::clean::clean_pdf(&source, &output),
                    "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => {
                        crate::privacy::clean::clean_office(&source, &output)
                    }
                    "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff" => {
                        crate::privacy::clean::clean_image(&source, &output)
                    }
                    _ => Err("当前格式没有自动清理器".into()),
                }
                .map_err(failed)?;
                serde_json::to_value(removed).map_err(failed)
            }
            Operation::PdfText { source } => {
                serde_json::to_value(crate::search::extract_pdf(&source).map_err(failed)?)
                    .map_err(failed)
            }
            Operation::OfficeText { source } => {
                serde_json::to_value(crate::search::extract_office(&source).map_err(failed)?)
                    .map_err(failed)
            }
            #[cfg(test)]
            Operation::Sleep => {
                std::thread::sleep(Duration::from_secs(30));
                Ok(Value::Null)
            }
            #[cfg(test)]
            Operation::CheckMemoryLimit => {
                let mut bytes = Vec::<u8>::new();
                Ok(Value::Bool(
                    bytes.try_reserve_exact(768 * 1024 * 1024).is_err(),
                ))
            }
        }
    })();
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(RESPONSE_MARKER).is_err()
        || serde_json::to_writer(&mut stdout, &response).is_err()
        || stdout.flush().is_err()
    {
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn job_rejects_allocations_above_the_process_memory_limit() {
        let rejected: bool = invoke(Operation::CheckMemoryLimit, Vec::new()).unwrap();
        assert!(rejected);
    }
    #[test]
    fn worker_entry() {
        if std::env::var_os("FLOEPOD_TEST_PARSER_WORKER").is_some() {
            worker_main();
            std::process::exit(0); // Do not write the test harness footer into the protocol.
        }
    }
    #[test]
    fn parser_timeout_is_reported_and_child_is_reaped() {
        let started = Instant::now();
        let result: Result<Value, _> =
            invoke_with_timeout(Operation::Sleep, Vec::new(), Duration::from_millis(150));
        assert!(matches!(result, Err(ScanError::Skipped(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
