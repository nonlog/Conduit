#![windows_subsystem = "windows"]

use std::future::{Future, IntoFuture};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use windows::core::{Interface, HSTRING};
use windows::ApplicationModel::Activation::{ActivationKind, ShareTargetActivatedEventArgs};
use windows::ApplicationModel::AppInstance;
use windows::ApplicationModel::DataTransfer::ShareTarget::ShareOperation;
use windows::ApplicationModel::DataTransfer::StandardDataFormats;
use windows::Storage::StorageItemTypes;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Unparker(std::thread::Thread);

impl Wake for Unparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: IntoFuture>(operation: F) -> F::Output {
    let mut future = std::pin::pin!(operation.into_future());
    let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return result;
        }
        std::thread::park();
    }
}

fn main() {
    let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
    let result = if initialized {
        run()
    } else {
        Err("Could not initialize the Windows Runtime.".to_string())
    };
    if initialized {
        unsafe { RoUninitialize() };
    }
    if let Err(message) = result {
        show_error(&message);
    }
}

fn run() -> Result<(), String> {
    let activated = AppInstance::GetActivatedEventArgs()
        .map_err(|error| format!("Could not read Share Target activation: {error}"))?;
    if activated
        .Kind()
        .map_err(|error| format!("Could not inspect Share Target activation: {error}"))?
        != ActivationKind::ShareTarget
    {
        return Err("Conduit was started without a Windows Share Target payload.".into());
    }

    let args: ShareTargetActivatedEventArgs = activated
        .cast()
        .map_err(|error| format!("Could not read Share Target arguments: {error}"))?;
    let operation = args
        .ShareOperation()
        .map_err(|error| format!("Could not read the shared operation: {error}"))?;
    operation
        .ReportStarted()
        .map_err(|error| format!("Could not start the shared operation: {error}"))?;

    match receive_files(&operation) {
        Ok(files) if !files.is_empty() => {
            operation
                .ReportDataRetrieved()
                .map_err(|error| format!("Could not acknowledge the shared files: {error}"))?;
            let mut failures = Vec::new();
            for file in &files {
                if let Err(error) = send_file(file) {
                    failures.push(format!("{}: {error}", file.display()));
                }
            }
            if failures.is_empty() {
                operation
                    .ReportCompleted()
                    .map_err(|error| format!("Could not complete the shared operation: {error}"))?;
                Ok(())
            } else {
                let detail = failures.into_iter().take(3).collect::<Vec<_>>().join("; ");
                let _ = operation.ReportError(&HSTRING::from(&detail));
                Err(detail)
            }
        }
        Ok(_) => report_error(&operation, "The shared item did not contain a file."),
        Err(error) => report_error(&operation, &error),
    }
}

fn receive_files(operation: &ShareOperation) -> Result<Vec<PathBuf>, String> {
    let data = operation
        .Data()
        .map_err(|error| format!("Could not read shared data: {error}"))?;
    let format = StandardDataFormats::StorageItems()
        .map_err(|error| format!("Could not identify shared files: {error}"))?;
    if !data
        .Contains(&format)
        .map_err(|error| format!("Could not inspect shared formats: {error}"))?
    {
        return Err("Conduit can receive files from the Windows share sheet.".into());
    }
    let items = block_on(
        data.GetStorageItemsAsync()
            .map_err(|error| format!("Could not retrieve shared files: {error}"))?,
    )
    .map_err(|error| format!("Could not retrieve shared files: {error}"))?;
    let mut files = Vec::new();
    for index in 0..items.Size().map_err(|error| error.to_string())? {
        let item = items.GetAt(index).map_err(|error| error.to_string())?;
        if item
            .IsOfType(StorageItemTypes::File)
            .map_err(|error| error.to_string())?
        {
            let path = item.Path().map_err(|error| error.to_string())?.to_string();
            if !path.is_empty() {
                files.push(PathBuf::from(path));
            }
        }
    }
    Ok(files)
}

fn send_file(path: &Path) -> Result<(), String> {
    let helper = std::env::current_exe()
        .map_err(|error| format!("Could not locate Conduit: {error}"))?
        .with_file_name("conduit-send.exe");
    let status = Command::new(&helper)
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not start {}: {error}", helper.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Conduit send helper exited with {status}."))
    }
}

fn report_error(operation: &ShareOperation, message: &str) -> Result<(), String> {
    let safe = message.chars().take(1024).collect::<String>();
    let _ = operation.ReportError(&HSTRING::from(&safe));
    Err(safe)
}

fn show_error(message: &str) {
    let Ok(ui) = std::env::current_exe().map(|path| path.with_file_name("conduit-rune.exe")) else {
        return;
    };
    if !ui.is_file() {
        return;
    }
    let _ = Command::new(ui)
        .arg("--send-error")
        .arg(message.chars().take(1024).collect::<String>())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_through_the_installed_sibling_helper() {
        let helper = Path::new(r"C:\Scoop\apps\conduit\current\conduit-share-target.exe");
        assert_eq!(
            helper.with_file_name("conduit-send.exe"),
            PathBuf::from(r"C:\Scoop\apps\conduit\current\conduit-send.exe")
        );
    }

    #[test]
    fn errors_are_bounded_before_being_sent_to_windows() {
        let message = "x".repeat(2048);
        assert_eq!(message.chars().take(1024).count(), 1024);
    }
}
