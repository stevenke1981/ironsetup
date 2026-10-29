#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let text: Vec<u16> = "IronSetup Demo\n\nThis application was packaged and installed by Rust.\n\nCheck the installation directory and Windows Installed Apps.\nUse unins000.exe to remove this demo.".encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "IronSetup Demo".encode_utf16().chain(Some(0)).collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    #[cfg(not(windows))]
    println!("IronSetup Demo: installed successfully.");
}
