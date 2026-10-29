//! Small Win32 host with UI-thread ownership and channel-driven workers.
//! `Host` is kept alive by `run_window`, never freed from a window callback.

mod studio;
mod wizard;

use crate::{CompileCallback, InstallCallback, WizardSelection};
use anyhow::{anyhow, bail, Context, Result};
use ironsetup_core::Manifest;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{Com::*, LibraryLoader::*},
    UI::{
        Controls::{Dialogs::*, *},
        Input::KeyboardAndMouse::*,
        Shell::*,
        WindowsAndMessaging::*,
    },
};

pub(super) const TEXT_LIMIT: usize = 4 * 1024 * 1024;
pub(super) type Controls = BTreeMap<u16, HWND>;

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub(super) fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) struct Fonts {
    normal: HFONT,
    title: HFONT,
    mono: HFONT,
    dpi: i32,
}
impl Fonts {
    unsafe fn new() -> Self {
        let dc = GetDC(null_mut());
        let dpi = if dc.is_null() {
            96
        } else {
            GetDeviceCaps(dc, LOGPIXELSY as i32).max(96)
        };
        if !dc.is_null() {
            ReleaseDC(null_mut(), dc);
        }
        let make = |size: i32, weight: i32, face: &str| {
            CreateFontW(
                -size * dpi / 96,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide(face).as_ptr(),
            )
        };
        Self {
            normal: make(16, FW_NORMAL as i32, "Segoe UI"),
            title: make(25, FW_SEMIBOLD as i32, "Segoe UI"),
            mono: make(15, FW_NORMAL as i32, "Consolas"),
            dpi,
        }
    }
    pub(super) fn px(&self, value: i32) -> i32 {
        value * self.dpi / 96
    }
}
impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for font in [self.normal, self.title, self.mono] {
                if !font.is_null() {
                    DeleteObject(font);
                }
            }
        }
    }
}

pub(super) unsafe fn control(
    parent: HWND,
    controls: &mut Controls,
    fonts: &Fonts,
    id: u16,
    class: &str,
    text: &str,
    style: u32,
) -> Result<HWND> {
    let hwnd = CreateWindowExW(
        if class == "EDIT" || class == "LISTBOX" {
            WS_EX_CLIENTEDGE
        } else {
            0
        },
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        1,
        1,
        parent,
        id as usize as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    if hwnd.is_null() {
        bail!(
            "Cannot create native {class} control ({id}): {}",
            std::io::Error::last_os_error()
        );
    }
    let font = if fonts.normal.is_null() {
        GetStockObject(DEFAULT_GUI_FONT)
    } else {
        fonts.normal
    };
    SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    if class == "EDIT" {
        SendMessageW(hwnd, EM_SETLIMITTEXT, TEXT_LIMIT, 0);
    }
    controls.insert(id, hwnd);
    Ok(hwnd)
}

pub(super) unsafe fn set_text(hwnd: HWND, text: &str) {
    SetWindowTextW(hwnd, wide(text).as_ptr());
}
pub(super) unsafe fn text(hwnd: HWND) -> Result<String> {
    let len = GetWindowTextLengthW(hwnd);
    if len < 0 || len as usize > TEXT_LIMIT {
        bail!("Native text exceeds the supported size limit");
    }
    let mut buf = vec![0u16; len as usize + 1];
    let actual = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16(&buf[..actual.max(0) as usize])
        .context("Text contains an invalid UTF-16 sequence")
}
pub(super) unsafe fn place(hwnd: HWND, fonts: &Fonts, x: i32, y: i32, width: i32, height: i32) {
    MoveWindow(
        hwnd,
        fonts.px(x),
        fonts.px(y),
        fonts.px(width.max(1)),
        fonts.px(height.max(1)),
        1,
    );
}
pub(super) unsafe fn visible(hwnd: HWND, show: bool) {
    ShowWindow(hwnd, if show { SW_SHOW } else { SW_HIDE });
}
pub(super) unsafe fn enabled(hwnd: HWND, enable: bool) {
    EnableWindow(hwnd, i32::from(enable));
}
pub(super) unsafe fn client_size(hwnd: HWND, fonts: &Fonts) -> (i32, i32) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    (
        (rect.right - rect.left) * 96 / fonts.dpi,
        (rect.bottom - rect.top) * 96 / fonts.dpi,
    )
}
pub(super) unsafe fn dialog(owner: HWND, caption: &str, message: &str, flags: u32) -> i32 {
    MessageBoxW(owner, wide(message).as_ptr(), wide(caption).as_ptr(), flags)
}

pub fn show_error(message: &str) {
    unsafe {
        dialog(null_mut(), "IronSetup", message, MB_OK | MB_ICONERROR);
    }
}
pub fn show_information(message: &str) {
    unsafe {
        dialog(null_mut(), "IronSetup", message, MB_OK | MB_ICONINFORMATION);
    }
}
pub fn confirm_uninstall(app_name: &str, target: &Path) -> Result<bool> {
    unsafe {
        let result = dialog(null_mut(), "Remove application", &format!(
            "Remove {app_name} from this computer?\n\n{}\n\nFiles changed after installation and files created by you will be preserved.", target.display()
        ), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2);
        if result == 0 {
            bail!(
                "Cannot show uninstall confirmation: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(result == IDYES)
    }
}

pub(super) unsafe fn file_dialog(
    owner: HWND,
    save: bool,
    initial: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let mut filename = vec![0u16; 32_768];
    if let Some(initial) = initial {
        let value = path_wide(initial);
        if value.len() >= filename.len() {
            bail!("Script path is too long for the native file dialog");
        }
        filename[..value.len()].copy_from_slice(&value);
    }
    let filter = wide("Inno Setup scripts (*.iss)\0*.iss\0All files (*.*)\0*.*\0");
    let extension = wide("iss");
    let title = wide(if save {
        "Save installation script"
    } else {
        "Open installation script"
    });
    let mut ofn: OPENFILENAMEW = std::mem::zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFile = filename.as_mut_ptr();
    ofn.nMaxFile = filename.len() as u32;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrDefExt = extension.as_ptr();
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER
        | OFN_PATHMUSTEXIST
        | OFN_NOCHANGEDIR
        | OFN_HIDEREADONLY
        | if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST
        };
    let accepted = if save {
        GetSaveFileNameW(&mut ofn)
    } else {
        GetOpenFileNameW(&mut ofn)
    };
    if accepted == 0 {
        let error = CommDlgExtendedError();
        if error != 0 {
            bail!("Native file dialog failed (code {error:#x})");
        }
        return Ok(None);
    }
    let len = filename
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(filename.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&filename[..len]))))
}

pub(super) unsafe fn folder_dialog(owner: HWND) -> Result<Option<PathBuf>> {
    let title = wide("Choose the application installation folder");
    let mut display = vec![0u16; 260];
    let mut bi: BROWSEINFOW = std::mem::zeroed();
    bi.hwndOwner = owner;
    bi.pszDisplayName = display.as_mut_ptr();
    bi.lpszTitle = title.as_ptr();
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
    let pidl = SHBrowseForFolderW(&bi);
    if pidl.is_null() {
        return Ok(None);
    }
    let mut path = vec![0u16; 32_768];
    let ok = SHGetPathFromIDListEx(pidl, path.as_mut_ptr(), path.len() as u32, GPFIDL_DEFAULT);
    CoTaskMemFree(pidl.cast());
    if ok == 0 {
        bail!("The selected item is not a filesystem folder");
    }
    let len = path.iter().position(|c| *c == 0).unwrap_or(path.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&path[..len]))))
}

pub(super) enum Action {
    None,
    Close,
}
enum App {
    Wizard(Box<wizard::Wizard>),
    Studio(studio::Studio),
}
impl App {
    unsafe fn init(&mut self, hwnd: HWND, fonts: &Fonts) -> Result<()> {
        match self {
            Self::Wizard(a) => a.init(hwnd, fonts),
            Self::Studio(a) => a.init(hwnd, fonts),
        }
    }
    unsafe fn layout(&mut self, hwnd: HWND, fonts: &Fonts) {
        match self {
            Self::Wizard(a) => a.layout(hwnd, fonts),
            Self::Studio(a) => a.layout(hwnd, fonts),
        }
    }
    unsafe fn command(
        &mut self,
        hwnd: HWND,
        fonts: &Fonts,
        id: u16,
        notification: u16,
    ) -> Result<Action> {
        match self {
            Self::Wizard(a) => a.command(hwnd, fonts, id, notification),
            Self::Studio(a) => a.command(hwnd, fonts, id, notification),
        }
    }
    unsafe fn poll(&mut self, hwnd: HWND, fonts: &Fonts) -> Result<Action> {
        match self {
            Self::Wizard(a) => a.poll(hwnd, fonts),
            Self::Studio(a) => a.poll(hwnd, fonts),
        }
    }
    unsafe fn close(&mut self, hwnd: HWND) -> Result<Action> {
        match self {
            Self::Wizard(a) => a.close(hwnd),
            Self::Studio(a) => a.close(hwnd),
        }
    }
    fn outcome(&self) -> Result<()> {
        match self {
            Self::Wizard(a) => a.outcome(),
            Self::Studio(_) => Ok(()),
        }
    }
}

struct Host {
    app: RefCell<App>,
    fonts: Fonts,
    error: RefCell<Option<String>>,
    studio: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Rust unwinding must never cross a system callback boundary.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(hwnd, msg, wparam, lparam)
    })) {
        Ok(result) => result,
        Err(_) => {
            let host = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Host;
            if let Some(host) = host.as_ref() {
                if let Ok(mut error) = host.error.try_borrow_mut() {
                    *error = Some("The native UI encountered an internal error".into());
                }
            }
            PostQuitMessage(1);
            0
        }
    }
}

unsafe fn dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Host;
    let Some(host) = pointer.as_ref() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    if msg == WM_CTLCOLORSTATIC {
        SetBkColor(wparam as HDC, GetSysColor(COLOR_WINDOW));
        SetTextColor(wparam as HDC, GetSysColor(COLOR_WINDOWTEXT));
        return GetSysColorBrush(COLOR_WINDOW) as isize;
    }
    if msg == WM_GETMINMAXINFO && host.studio {
        let info = &mut *(lparam as *mut MINMAXINFO);
        info.ptMinTrackSize.x = host.fonts.px(760);
        info.ptMinTrackSize.y = host.fonts.px(580);
        return 0;
    }
    if !matches!(msg, WM_CREATE | WM_SIZE | WM_COMMAND | WM_TIMER | WM_CLOSE) {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    // Creating/updating controls synchronously re-enters this procedure. Never
    // alias a mutable state reference, and never close while a callback borrows it.
    let result = {
        let Ok(mut app) = host.app.try_borrow_mut() else {
            return 0;
        };
        match msg {
            WM_CREATE => app.init(hwnd, &host.fonts).map(|()| Action::None),
            WM_SIZE => {
                app.layout(hwnd, &host.fonts);
                Ok(Action::None)
            }
            WM_COMMAND => app.command(
                hwnd,
                &host.fonts,
                (wparam & 0xffff) as u16,
                ((wparam >> 16) & 0xffff) as u16,
            ),
            WM_TIMER => app.poll(hwnd, &host.fonts),
            WM_CLOSE => app.close(hwnd),
            _ => Ok(Action::None),
        }
    };
    match result {
        Ok(Action::Close) => {
            DestroyWindow(hwnd);
        }
        Ok(Action::None) => (),
        Err(error) if msg == WM_CREATE => {
            *host.error.borrow_mut() = Some(format!("{error:#}"));
            return -1;
        }
        Err(error) => {
            dialog(
                hwnd,
                "IronSetup",
                &format!("{error:#}"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    0
}

unsafe fn run_window(app: App, title: &str, studio: bool) -> Result<()> {
    // System-DPI aware controls and fonts. Per-monitor DPI transitions are a
    // documented limitation of this initial native host.
    SetProcessDPIAware();
    let com_result = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    let _com = ComGuard(com_result >= 0);
    let common = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
    };
    if InitCommonControlsEx(&common) == 0 {
        bail!("Cannot initialize native Windows controls");
    }
    let instance = GetModuleHandleW(null());
    let class = wide("IronSetup.Native.Window.v1");
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_WINDOW),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        bail!(
            "Cannot register native window: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut host = Box::new(Host {
        app: RefCell::new(app),
        fonts: Fonts::new(),
        error: RefCell::new(None),
        studio,
    });
    let style = WS_CLIPCHILDREN
        | if studio {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
        };
    let (width, height) = if studio { (1020, 760) } else { (760, 570) };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: host.fonts.px(width),
        bottom: host.fonts.px(height),
    };
    AdjustWindowRect(&mut rect, style, 0);
    let win_width = rect.right - rect.left;
    let win_height = rect.bottom - rect.top;
    let x = ((GetSystemMetrics(SM_CXSCREEN) - win_width) / 2).max(0);
    let y = ((GetSystemMetrics(SM_CYSCREEN) - win_height) / 2).max(0);
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        class.as_ptr(),
        wide(title).as_ptr(),
        style,
        x,
        y,
        win_width,
        win_height,
        null_mut(),
        null_mut(),
        instance,
        (&mut *host as *mut Host).cast(),
    );
    if hwnd.is_null() {
        let detail = host
            .error
            .borrow_mut()
            .take()
            .unwrap_or_else(|| std::io::Error::last_os_error().to_string());
        bail!("Cannot open IronSetup: {detail}");
    }
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    if SetTimer(hwnd, 1, 75, None) == 0 {
        DestroyWindow(hwnd);
        bail!("Cannot create UI progress timer");
    }
    let mut message: MSG = std::mem::zeroed();
    loop {
        let status = GetMessageW(&mut message, null_mut(), 0, 0);
        if status == -1 {
            *host.error.borrow_mut() = Some(std::io::Error::last_os_error().to_string());
            break;
        }
        if status == 0 {
            break;
        }
        if studio && message.message == WM_KEYDOWN {
            let control_down = GetKeyState(VK_CONTROL as i32) < 0;
            let command = match (control_down, message.wParam as u16) {
                (true, 0x4e) => Some(studio::ID_NEW),
                (true, 0x4f) => Some(studio::ID_OPEN),
                (true, 0x53) => Some(studio::ID_SAVE),
                (_, VK_F9) => Some(studio::ID_BUILD),
                _ => None,
            };
            if let Some(command) = command {
                SendMessageW(hwnd, WM_COMMAND, command as usize, 0);
                continue;
            }
            if control_down && message.wParam == 0x41 {
                let focus = GetFocus();
                if GetDlgCtrlID(focus) == studio::ID_EDITOR as i32 {
                    SendMessageW(focus, EM_SETSEL, 0, -1);
                    continue;
                }
            }
        }
        if IsDialogMessageW(hwnd, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    KillTimer(hwnd, 1);
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    // Every child HWND is destroyed before fonts and UI state are dropped.
    if let Some(error) = host.error.borrow_mut().take() {
        return Err(anyhow!(error));
    }
    let result = host.app.borrow().outcome();
    result
}

pub fn installer_wizard(
    manifest: &Manifest,
    selections: WizardSelection,
    perform: InstallCallback,
) -> Result<()> {
    let title = format!("{} {} — Setup", manifest.app.name, manifest.app.version);
    let app = wizard::Wizard::new(manifest.clone(), selections, perform)?;
    unsafe { run_window(App::Wizard(Box::new(app)), &title, false) }
}
pub fn compiler_studio(initial_script: Option<PathBuf>, compile: CompileCallback) -> Result<()> {
    unsafe {
        run_window(
            App::Studio(studio::Studio::new(initial_script, compile)),
            "IronSetup Studio",
            true,
        )
    }
}
