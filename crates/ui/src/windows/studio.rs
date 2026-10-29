use super::*;
use crate::flow;
use std::{
    fs,
    io::Write,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const ID_NEW: u16 = 100;
pub(super) const ID_OPEN: u16 = 101;
pub(super) const ID_SAVE: u16 = 102;
const ID_SAVE_AS: u16 = 103;
pub(super) const ID_BUILD: u16 = 104;
const ID_FOLDER: u16 = 105;
const ID_PATH: u16 = 106;
const ID_TITLE: u16 = 107;
pub(super) const ID_EDITOR: u16 = 108;
const ID_LOG_LABEL: u16 = 109;
const ID_LOG: u16 = 110;
const ID_STATUS: u16 = 111;

const NEW_SCRIPT: &str = r#"; IronSetup v0.1 script. Save this file beside your application files.
; F9 builds the installer. Unsupported Inno Setup syntax produces an error.
#define AppName "My Application"
#define AppVersion "1.0.0"

[Setup]
AppId=MyApplication
AppName={#AppName}
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
Compression=zip
OutputDir=Output
OutputBaseFilename=MyApplication-Setup

[Files]
; Uncomment and change Source to your application's actual files:
; Source: "app\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
"#;

pub(super) struct Studio {
    controls: Controls,
    current: Option<PathBuf>,
    initial: Option<PathBuf>,
    saved_disk_text: Option<String>,
    dirty: bool,
    busy: bool,
    compile: Arc<dyn Fn(PathBuf) -> Result<String> + Send + Sync>,
    receiver: Option<Receiver<std::result::Result<String, String>>>,
}
impl Studio {
    pub(super) fn new(initial: Option<PathBuf>, compile: CompileCallback) -> Self {
        Self {
            controls: Controls::new(),
            current: None,
            initial,
            saved_disk_text: None,
            dirty: false,
            busy: false,
            compile: Arc::from(compile),
            receiver: None,
        }
    }
    fn h(&self, id: u16) -> HWND {
        self.controls.get(&id).copied().unwrap_or(null_mut())
    }

    pub(super) unsafe fn init(&mut self, hwnd: HWND, fonts: &Fonts) -> Result<()> {
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ID_TITLE,
            "STATIC",
            "IronSetup Studio",
            0,
        )?;
        SendMessageW(self.h(ID_TITLE), WM_SETFONT, fonts.title as usize, 1);
        for (id, label) in [
            (ID_NEW, "New"),
            (ID_OPEN, "Open..."),
            (ID_SAVE, "Save"),
            (ID_SAVE_AS, "Save as..."),
            (ID_BUILD, "Build  F9"),
            (ID_FOLDER, "Folder"),
        ] {
            control(
                hwnd,
                &mut self.controls,
                fonts,
                id,
                "BUTTON",
                label,
                BS_PUSHBUTTON as u32 | WS_TABSTOP,
            )?;
        }
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ID_PATH,
            "STATIC",
            "Untitled.iss",
            0,
        )?;
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ID_EDITOR,
            "EDIT",
            "",
            (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_WANTRETURN) as u32
                | WS_VSCROLL
                | WS_HSCROLL
                | WS_TABSTOP,
        )?;
        SendMessageW(self.h(ID_EDITOR), WM_SETFONT, fonts.mono as usize, 1);
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ID_LOG_LABEL,
            "STATIC",
            "Build output",
            0,
        )?;
        control(hwnd, &mut self.controls, fonts, ID_LOG, "EDIT", "Open an .iss script or start with New. Save changes, then select Build.\r\nCtrl+N New    Ctrl+O Open    Ctrl+S Save    F9 Build", (ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32 | WS_VSCROLL | WS_TABSTOP)?;
        SendMessageW(self.h(ID_LOG), WM_SETFONT, fonts.mono as usize, 1);
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ID_STATUS,
            "STATIC",
            "Ready  ·  UTF-8 scripts  ·  Rust compiler",
            0,
        )?;
        self.layout(hwnd, fonts);
        if let Some(path) = self.initial.take() {
            // An invalid startup path leaves a usable studio and the actual error
            // in the log instead of displaying an empty successful editor.
            if let Err(error) = self.load_file(hwnd, path) {
                set_text(self.h(ID_LOG), &format!("Cannot open script:\r\n{error:#}"));
                self.set_new(hwnd);
            }
        } else {
            self.set_new(hwnd);
        }
        SetFocus(self.h(ID_EDITOR));
        Ok(())
    }

    pub(super) unsafe fn layout(&mut self, hwnd: HWND, fonts: &Fonts) {
        let (width, height) = client_size(hwnd, fonts);
        let log_height = ((height - 220) / 4).clamp(110, 220);
        let log_y = height - log_height - 46;
        for (id, x, y, w, h) in [
            (ID_TITLE, 24, 20, 400, 36),
            (ID_NEW, 24, 70, 76, 32),
            (ID_OPEN, 108, 70, 90, 32),
            (ID_SAVE, 206, 70, 76, 32),
            (ID_SAVE_AS, 290, 70, 100, 32),
            (ID_BUILD, width - 235, 70, 120, 32),
            (ID_FOLDER, width - 107, 70, 83, 32),
            (ID_PATH, 24, 117, width - 48, 25),
            (ID_EDITOR, 24, 150, width - 48, log_y - 193),
            (ID_LOG_LABEL, 24, log_y - 31, width - 48, 24),
            (ID_LOG, 24, log_y, width - 48, log_height),
            (ID_STATUS, 24, height - 29, width - 48, 23),
        ] {
            if !self.h(id).is_null() {
                place(self.h(id), fonts, x, y, w, h);
            }
        }
    }

    unsafe fn title(&self, hwnd: HWND) {
        let path = self
            .current
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Untitled.iss".into());
        set_text(
            self.h(ID_PATH),
            &format!("{}{}", if self.dirty { "* " } else { "" }, path),
        );
        let name = self
            .current
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.iss".into());
        set_text(
            hwnd,
            &format!(
                "{}{} — IronSetup Studio",
                if self.dirty { "* " } else { "" },
                name
            ),
        );
    }
    unsafe fn set_new(&mut self, hwnd: HWND) {
        set_text(self.h(ID_EDITOR), &flow::crlf(NEW_SCRIPT));
        self.current = None;
        self.saved_disk_text = None;
        self.dirty = false;
        self.title(hwnd);
    }
    unsafe fn load_file(&mut self, hwnd: HWND, path: PathBuf) -> Result<()> {
        let path = fs::canonicalize(path).context("Resolve script path")?;
        let metadata = fs::metadata(&path)?;
        if metadata.len() > TEXT_LIMIT as u64 {
            bail!("Studio supports script files up to 4 MiB");
        }
        let disk_text = fs::read_to_string(&path).context("Read script as UTF-8")?;
        if disk_text.contains('\0') {
            bail!("The script contains NUL characters and cannot be edited safely");
        }
        set_text(
            self.h(ID_EDITOR),
            &flow::crlf(disk_text.trim_start_matches('\u{feff}')),
        );
        self.current = Some(path);
        self.saved_disk_text = Some(disk_text);
        self.dirty = false;
        self.title(hwnd);
        set_text(self.h(ID_STATUS), "Script loaded  ·  UTF-8");
        Ok(())
    }

    unsafe fn save(&mut self, hwnd: HWND, save_as: bool) -> Result<bool> {
        let path = if save_as || self.current.is_none() {
            let Some(path) = file_dialog(hwnd, true, self.current.as_deref())? else {
                return Ok(false);
            };
            path
        } else {
            self.current
                .clone()
                .ok_or_else(|| anyhow!("No script path"))?
        };
        if self.current.as_ref() == Some(&path) {
            if let (Some(saved), Ok(now)) = (&self.saved_disk_text, fs::read_to_string(&path)) {
                if saved != &now && dialog(hwnd, "Script changed on disk", "Another program changed this script after it was opened.\n\nOverwrite that file with the text in this editor?", MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES {
                    return Ok(false);
                }
            }
        }
        let content = text(self.h(ID_EDITOR))?;
        if content.len() > TEXT_LIMIT {
            bail!("Studio supports script files up to 4 MiB");
        }
        save_atomically(&path, content.as_bytes())
            .with_context(|| format!("Save {}", path.display()))?;
        self.current = Some(fs::canonicalize(&path)?);
        self.saved_disk_text = Some(content);
        self.dirty = false;
        self.title(hwnd);
        set_text(self.h(ID_STATUS), "Script saved  ·  UTF-8");
        Ok(true)
    }

    unsafe fn may_discard(&mut self, hwnd: HWND) -> Result<bool> {
        if !self.dirty {
            return Ok(true);
        }
        match dialog(
            hwnd,
            "Unsaved script",
            "Save changes to this script before continuing?",
            MB_YESNOCANCEL | MB_ICONQUESTION,
        ) {
            IDYES => self.save(hwnd, false),
            IDNO => Ok(true),
            _ => Ok(false),
        }
    }

    unsafe fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        for id in [ID_NEW, ID_OPEN, ID_SAVE, ID_SAVE_AS, ID_BUILD] {
            enabled(self.h(id), !busy);
        }
        SendMessageW(self.h(ID_EDITOR), EM_SETREADONLY, usize::from(busy), 0);
        set_text(
            self.h(ID_BUILD),
            if busy { "Building..." } else { "Build  F9" },
        );
    }

    unsafe fn build(&mut self, hwnd: HWND) -> Result<()> {
        if !self.save(hwnd, false)? {
            return Ok(());
        }
        let path = self
            .current
            .clone()
            .ok_or_else(|| anyhow!("Save the script before building"))?;
        let compile = self.compile.clone();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("ironsetup-compile".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile(path)));
                let result = match result {
                    Ok(result) => result.map_err(|error| format!("{error:#}")),
                    Err(_) => Err("The compiler worker encountered an internal error".into()),
                };
                let _ = sender.send(result);
            })
            .context("Start compiler worker")?;
        self.receiver = Some(receiver);
        self.set_busy(true);
        set_text(
            self.h(ID_LOG),
            &format!(
                "Compiling {}\r\n\r\nThe build result and any diagnostics will appear here.",
                self.current
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
        );
        set_text(
            self.h(ID_STATUS),
            "Building  ·  The editor is read-only until this build finishes",
        );
        Ok(())
    }

    pub(super) unsafe fn command(
        &mut self,
        hwnd: HWND,
        _fonts: &Fonts,
        id: u16,
        notification: u16,
    ) -> Result<Action> {
        if id == ID_EDITOR && notification == EN_CHANGE as u16 {
            if !self.busy {
                self.dirty = true;
                self.title(hwnd);
            }
            return Ok(Action::None);
        }
        if self.busy && id != ID_FOLDER {
            return Ok(Action::None);
        }
        match id {
            ID_NEW => {
                if self.may_discard(hwnd)? {
                    self.set_new(hwnd);
                }
            }
            ID_OPEN => {
                if self.may_discard(hwnd)? {
                    if let Some(path) = file_dialog(hwnd, false, self.current.as_deref())? {
                        self.load_file(hwnd, path)?;
                    }
                }
            }
            ID_SAVE => {
                self.save(hwnd, false)?;
            }
            ID_SAVE_AS => {
                self.save(hwnd, true)?;
            }
            ID_BUILD => self.build(hwnd)?,
            ID_FOLDER => {
                if let Some(folder) = self.current.as_ref().and_then(|p| p.parent()) {
                    let outcome = ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        path_wide(folder).as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                    if outcome as isize <= 32 {
                        bail!(
                            "Windows could not open the script folder (code {})",
                            outcome as isize
                        );
                    }
                } else {
                    bail!("Save or open a script to select its folder.");
                }
            }
            2 => return self.close(hwnd),
            _ => (),
        }
        Ok(Action::None)
    }

    pub(super) unsafe fn poll(&mut self, _hwnd: HWND, _fonts: &Fonts) -> Result<Action> {
        let Some(receiver) = &self.receiver else {
            return Ok(Action::None);
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                self.set_busy(false);
                match result {
                    Ok(output) => {
                        set_text(self.h(ID_LOG), &flow::crlf(&output));
                        set_text(
                            self.h(ID_STATUS),
                            "Build completed  ·  Review the artifact paths and warnings above",
                        );
                    }
                    Err(error) => {
                        set_text(
                            self.h(ID_LOG),
                            &flow::crlf(&format!("BUILD FAILED\n\n{error}")),
                        );
                        set_text(
                            self.h(ID_STATUS),
                            "Build failed  ·  Correct the reported error and build again",
                        );
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                self.set_busy(false);
                set_text(
                    self.h(ID_LOG),
                    "BUILD FAILED\r\n\r\nThe compiler worker disconnected without a result.",
                );
                set_text(self.h(ID_STATUS), "Build failed");
            }
            Err(mpsc::TryRecvError::Empty) => (),
        }
        Ok(Action::None)
    }

    pub(super) unsafe fn close(&mut self, hwnd: HWND) -> Result<Action> {
        if self.busy {
            return Ok(Action::None);
        }
        if self.may_discard(hwnd)? {
            Ok(Action::Close)
        } else {
            Ok(Action::None)
        }
    }
}

fn save_atomically(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = parent.join(format!(
        ".ironsetup-save-{}-{stamp}.tmp",
        std::process::id()
    ));
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        output.write_all(content)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}
