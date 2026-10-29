use super::*;
use crate::flow::{self, Page};
use ironsetup_core::{evaluate_condition, Progress};
use std::{
    collections::BTreeSet,
    sync::mpsc::{self, Receiver},
    thread,
};

const HEADER: u16 = 10;
const TITLE: u16 = 11;
const SUBTITLE: u16 = 12;
const BODY: u16 = 13;
const ACCEPT: u16 = 14;
const DIRECTORY_LABEL: u16 = 15;
const DIRECTORY: u16 = 16;
const BROWSE: u16 = 17;
const TYPE_LABEL: u16 = 18;
const TYPE: u16 = 19;
const COMPONENTS_LABEL: u16 = 20;
const COMPONENTS: u16 = 21;
const TASKS_LABEL: u16 = 22;
const TASKS: u16 = 23;
const STATUS: u16 = 24;
const PROGRESS: u16 = 25;
const BACK: u16 = 26;
const NEXT: u16 = 27;
const CANCEL: u16 = 28;
const FOOTER: u16 = 29;

enum WorkerEvent {
    Progress(Progress),
    Finished(std::result::Result<String, String>),
}

pub(super) struct Wizard {
    manifest: Manifest,
    selection: WizardSelection,
    route: Vec<Page>,
    step: usize,
    accepted: bool,
    controls: Controls,
    perform: Option<InstallCallback>,
    receiver: Option<Receiver<WorkerEvent>>,
    outcome: Option<std::result::Result<(), String>>,
    finish_text: String,
    combo_types: Vec<Option<String>>,
}

impl Wizard {
    pub(super) fn new(
        manifest: Manifest,
        selection: WizardSelection,
        perform: InstallCallback,
    ) -> Result<Self> {
        let route = flow::pages(&manifest);
        for (label, value) in [
            ("Application name", Some(manifest.app.name.as_str())),
            ("Version", Some(manifest.app.version.as_str())),
            ("Publisher", Some(manifest.app.publisher.as_str())),
            ("License", manifest.license_text.as_deref()),
            (
                "Information before installation",
                manifest.info_before.as_deref(),
            ),
            (
                "Information after installation",
                manifest.info_after.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                if value.contains('\0') || value.len() > TEXT_LIMIT {
                    bail!("{label} cannot contain NUL characters or exceed 4 MiB in the native wizard");
                }
            }
        }
        for value in manifest
            .components
            .iter()
            .map(|c| &c.description)
            .chain(manifest.tasks.iter().map(|t| &t.description))
            .chain(manifest.types.iter().map(|t| &t.description))
        {
            if value.contains('\0') || value.len() > TEXT_LIMIT {
                bail!("A setup choice contains text that cannot be displayed safely");
            }
        }
        for name in &selection.components {
            if !manifest.components.iter().any(|c| &c.name == name) {
                bail!("Unknown component: {name}");
            }
        }
        for name in &selection.tasks {
            if !manifest.tasks.iter().any(|c| &c.name == name) {
                bail!("Unknown task: {name}");
            }
        }
        Ok(Self {
            manifest,
            selection,
            route,
            step: 0,
            accepted: false,
            controls: Controls::new(),
            perform: Some(perform),
            receiver: None,
            outcome: None,
            finish_text: String::new(),
            combo_types: vec![],
        })
    }
    fn h(&self, id: u16) -> HWND {
        self.controls.get(&id).copied().unwrap_or(null_mut())
    }
    fn page(&self) -> Page {
        self.route[self.step]
    }
    fn busy(&self) -> bool {
        self.page() == Page::Progress
    }

    pub(super) unsafe fn init(&mut self, hwnd: HWND, fonts: &Fonts) -> Result<()> {
        for (id, value) in [
            (HEADER, "IRONSETUP"),
            (TITLE, ""),
            (SUBTITLE, ""),
            (DIRECTORY_LABEL, "Installation folder"),
            (TYPE_LABEL, "Setup type"),
            (COMPONENTS_LABEL, "Components"),
            (TASKS_LABEL, "Additional tasks"),
            (STATUS, ""),
            (FOOTER, ""),
        ] {
            control(hwnd, &mut self.controls, fonts, id, "STATIC", value, 0)?;
        }
        SendMessageW(self.h(TITLE), WM_SETFONT, fonts.title as usize, 1);
        control(
            hwnd,
            &mut self.controls,
            fonts,
            BODY,
            "EDIT",
            "",
            (ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32 | WS_VSCROLL | WS_TABSTOP,
        )?;
        control(
            hwnd,
            &mut self.controls,
            fonts,
            ACCEPT,
            "BUTTON",
            "I accept the license agreement",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        )?;
        control(
            hwnd,
            &mut self.controls,
            fonts,
            DIRECTORY,
            "EDIT",
            &self.selection.target_dir.to_string_lossy(),
            ES_AUTOHSCROLL as u32 | WS_TABSTOP,
        )?;
        control(
            hwnd,
            &mut self.controls,
            fonts,
            BROWSE,
            "BUTTON",
            "Browse...",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
        )?;
        control(
            hwnd,
            &mut self.controls,
            fonts,
            TYPE,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
        )?;
        for id in [COMPONENTS, TASKS] {
            control(
                hwnd,
                &mut self.controls,
                fonts,
                id,
                "LISTBOX",
                "",
                (LBS_MULTIPLESEL | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | WS_TABSTOP,
            )?;
            SendMessageW(
                self.h(id),
                LB_SETHORIZONTALEXTENT,
                fonts.px(750) as usize,
                0,
            );
        }
        control(
            hwnd,
            &mut self.controls,
            fonts,
            PROGRESS,
            "msctls_progress32",
            "",
            PBS_SMOOTH,
        )?;
        SendMessageW(self.h(PROGRESS), PBM_SETRANGE32, 0, 1000);
        for (id, value, style) in [
            (BACK, "< Back", BS_PUSHBUTTON),
            (NEXT, "Next >", BS_DEFPUSHBUTTON),
            (CANCEL, "Cancel", BS_PUSHBUTTON),
        ] {
            control(
                hwnd,
                &mut self.controls,
                fonts,
                id,
                "BUTTON",
                value,
                style as u32 | WS_TABSTOP,
            )?;
        }
        for choice in &self.manifest.components {
            let label = format!(
                "{}{}",
                choice.description,
                if choice.flags.iter().any(|f| f == "fixed") {
                    " [required]"
                } else {
                    ""
                }
            );
            SendMessageW(
                self.h(COMPONENTS),
                LB_ADDSTRING,
                0,
                wide(&label).as_ptr() as isize,
            );
        }
        for task in &self.manifest.tasks {
            SendMessageW(
                self.h(TASKS),
                LB_ADDSTRING,
                0,
                wide(&task.description).as_ptr() as isize,
            );
        }
        for ty in &self.manifest.types {
            SendMessageW(
                self.h(TYPE),
                CB_ADDSTRING,
                0,
                wide(&ty.description).as_ptr() as isize,
            );
            self.combo_types.push(if ty.custom {
                None
            } else {
                Some(ty.name.clone())
            });
        }
        if !self.combo_types.iter().any(Option::is_none) {
            SendMessageW(
                self.h(TYPE),
                CB_ADDSTRING,
                0,
                wide("Custom").as_ptr() as isize,
            );
            self.combo_types.push(None);
        }
        let (default_components, default_tasks) = flow::default_selections(&self.manifest, None)?;
        let initial_type = if self.selection.components == default_components
            && self.selection.tasks == default_tasks
        {
            self.combo_types.iter().position(Option::is_some)
        } else {
            None
        }
        .unwrap_or_else(|| {
            self.combo_types
                .iter()
                .position(Option::is_none)
                .unwrap_or(0)
        });
        SendMessageW(self.h(TYPE), CB_SETCURSEL, initial_type, 0);
        self.sync_lists()?;
        self.layout(hwnd, fonts);
        self.render(hwnd)?;
        SetFocus(self.h(NEXT));
        Ok(())
    }

    pub(super) unsafe fn layout(&mut self, hwnd: HWND, fonts: &Fonts) {
        let (width, height) = client_size(hwnd, fonts);
        let right = width - 32;
        for (id, x, y, w, h) in [
            (HEADER, 32, 20, 300, 24),
            (TITLE, 32, 57, width - 64, 38),
            (SUBTITLE, 32, 107, width - 64, 48),
            (BODY, 32, 166, width - 64, height - 290),
            (ACCEPT, 32, height - 104, width - 64, 28),
            (DIRECTORY_LABEL, 32, 197, width - 64, 24),
            (DIRECTORY, 32, 230, width - 190, 30),
            (BROWSE, right - 114, 229, 114, 32),
            (TYPE_LABEL, 32, 162, 130, 25),
            (TYPE, 180, 158, width - 212, 280),
            (COMPONENTS_LABEL, 32, 208, (width - 88) / 2, 24),
            (TASKS_LABEL, width / 2 + 12, 208, (width - 88) / 2, 24),
            (COMPONENTS, 32, 240, (width - 88) / 2, height - 365),
            (TASKS, width / 2 + 12, 240, (width - 88) / 2, height - 365),
            (STATUS, 32, 205, width - 64, 76),
            (PROGRESS, 32, 298, width - 64, 24),
            (FOOTER, 32, height - 103, width - 64, 42),
            (BACK, right - 330, height - 49, 100, 32),
            (NEXT, right - 220, height - 49, 105, 32),
            (CANCEL, right - 105, height - 49, 105, 32),
        ] {
            if !self.h(id).is_null() {
                place(self.h(id), fonts, x, y, w, h);
            }
        }
    }

    unsafe fn render(&mut self, hwnd: HWND) -> Result<()> {
        let page = self.page();
        for id in [
            BODY,
            ACCEPT,
            DIRECTORY_LABEL,
            DIRECTORY,
            BROWSE,
            TYPE_LABEL,
            TYPE,
            COMPONENTS_LABEL,
            COMPONENTS,
            TASKS_LABEL,
            TASKS,
            STATUS,
            PROGRESS,
            FOOTER,
        ] {
            visible(self.h(id), false);
        }
        let (title, subtitle) = match page {
            Page::Welcome => (
                format!("Install {}", self.manifest.app.name),
                format!(
                    "Version {}  •  {}",
                    self.manifest.app.version, self.manifest.app.publisher
                ),
            ),
            Page::Information => (
                "Before you install".into(),
                "Read this information before continuing.".into(),
            ),
            Page::License => (
                "License agreement".into(),
                "Read the license. You must accept it to continue.".into(),
            ),
            Page::Destination => (
                "Choose an installation folder".into(),
                "The installer will place application files in the folder below.".into(),
            ),
            Page::Features => (
                "Choose components and tasks".into(),
                "Click a row or press Space to toggle it. Required components remain selected."
                    .into(),
            ),
            Page::Ready => (
                "Ready to install".into(),
                "Check your choices, then select Install.".into(),
            ),
            Page::Progress => (
                format!("Installing {}", self.manifest.app.name),
                "Please wait while the selected files and settings are installed.".into(),
            ),
            Page::Complete => (
                if self.outcome.as_ref().is_some_and(Result::is_ok) {
                    "Installation complete"
                } else {
                    "Installation did not complete"
                }
                .into(),
                "Read the result below before closing this window.".into(),
            ),
        };
        set_text(self.h(TITLE), &title);
        set_text(self.h(SUBTITLE), &subtitle);
        let body = match page {
            Page::Welcome => Some(format!("Welcome to the {} setup wizard.\r\n\r\nThe wizard will let you review the installation location and available options before it changes your computer.\r\n\r\nClose the application if an older copy is currently running.\r\n\r\nSelect Next to continue.", self.manifest.app.name)),
            Page::Information => self.manifest.info_before.clone(),
            Page::License => self.manifest.license_text.clone(),
            Page::Ready => Some(self.summary()?),
            Page::Complete => Some(self.finish_text.clone()),
            _ => None,
        };
        if let Some(body) = body {
            set_text(self.h(BODY), &flow::crlf(&body));
            visible(self.h(BODY), true);
        }
        if page == Page::License {
            visible(self.h(ACCEPT), true);
            SendMessageW(
                self.h(ACCEPT),
                BM_SETCHECK,
                if self.accepted {
                    BST_CHECKED as usize
                } else {
                    BST_UNCHECKED as usize
                },
                0,
            );
        }
        if page == Page::Destination {
            for id in [DIRECTORY_LABEL, DIRECTORY, BROWSE, FOOTER] {
                visible(self.h(id), true);
            }
            set_text(
                self.h(FOOTER),
                if self.manifest.app.privileges == "admin" {
                    "This package requires administrator privileges."
                } else {
                    "This package installs for the current user."
                },
            );
        }
        if page == Page::Features {
            for id in [
                TYPE_LABEL,
                TYPE,
                COMPONENTS_LABEL,
                COMPONENTS,
                TASKS_LABEL,
                TASKS,
            ] {
                visible(self.h(id), true);
            }
            enabled(self.h(COMPONENTS), !self.manifest.components.is_empty());
            enabled(self.h(TASKS), !self.manifest.tasks.is_empty());
        }
        if page == Page::Progress {
            for id in [STATUS, PROGRESS, FOOTER] {
                visible(self.h(id), true);
            }
            set_text(self.h(STATUS), "Preparing installation...");
            set_text(
                self.h(FOOTER),
                "Closing is disabled while installation is in progress.",
            );
        }
        enabled(
            self.h(BACK),
            self.step > 0 && page != Page::Progress && page != Page::Complete,
        );
        enabled(
            self.h(NEXT),
            page != Page::Progress && (page != Page::License || self.accepted),
        );
        enabled(self.h(CANCEL), page != Page::Progress);
        visible(self.h(CANCEL), page != Page::Complete);
        set_text(
            self.h(NEXT),
            match page {
                Page::Ready => "Install",
                Page::Complete => "Finish",
                _ => "Next >",
            },
        );
        // Pages reuse overlapping controls. Repaint visible children after the
        // previous page's controls are hidden so the license checkbox cannot
        // be covered by a later parent/background erase.
        RedrawWindow(
            hwnd,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
        Ok(())
    }

    fn summary(&self) -> Result<String> {
        let mut files = 0usize;
        let mut bytes = 0u64;
        for file in &self.manifest.files {
            if evaluate_condition(&file.conditions.components, &self.selection.components)?
                && evaluate_condition(&file.conditions.tasks, &self.selection.tasks)?
            {
                files += 1;
                bytes = bytes.saturating_add(file.size);
            }
        }
        let mut run_count = 0usize;
        for action in &self.manifest.run {
            if evaluate_condition(&action.conditions.components, &self.selection.components)?
                && evaluate_condition(&action.conditions.tasks, &self.selection.tasks)?
            {
                run_count += 1;
            }
        }
        Ok(format!("Application: {} {}\r\n\r\nDestination:\r\n{}\r\n\r\nSelected files: {files}\r\nUncompressed size: {:.2} MiB\r\n\r\nComponents: {}\r\nTasks: {}\r\n\r\nPrograms configured to run after installation: {run_count}",
            self.manifest.app.name, self.manifest.app.version, self.selection.target_dir.display(), bytes as f64 / 1_048_576.0,
            list_names(&self.selection.components), list_names(&self.selection.tasks)))
    }

    unsafe fn sync_lists(&mut self) -> Result<()> {
        for (index, component) in self.manifest.components.iter().enumerate() {
            if component.flags.iter().any(|f| f == "fixed") {
                self.selection.components.insert(component.name.clone());
            }
            SendMessageW(
                self.h(COMPONENTS),
                LB_SETSEL,
                usize::from(self.selection.components.contains(&component.name)),
                index as isize,
            );
        }
        for (index, task) in self.manifest.tasks.iter().enumerate() {
            if !evaluate_condition(&task.conditions.components, &self.selection.components)? {
                self.selection.tasks.remove(&task.name);
            }
            SendMessageW(
                self.h(TASKS),
                LB_SETSEL,
                usize::from(self.selection.tasks.contains(&task.name)),
                index as isize,
            );
        }
        Ok(())
    }

    unsafe fn capture_lists(&mut self) -> Result<()> {
        self.selection.components.clear();
        self.selection.tasks.clear();
        for (index, component) in self.manifest.components.iter().enumerate() {
            if component.flags.iter().any(|f| f == "fixed")
                || SendMessageW(self.h(COMPONENTS), LB_GETSEL, index, 0) > 0
            {
                self.selection.components.insert(component.name.clone());
            }
        }
        for (index, task) in self.manifest.tasks.iter().enumerate() {
            if SendMessageW(self.h(TASKS), LB_GETSEL, index, 0) > 0
                && evaluate_condition(&task.conditions.components, &self.selection.components)?
            {
                self.selection.tasks.insert(task.name.clone());
            }
        }
        self.sync_lists()
    }

    unsafe fn capture_directory(&mut self) -> Result<()> {
        let value = text(self.h(DIRECTORY))?;
        let path = PathBuf::from(value.trim());
        if !path.is_absolute() {
            bail!("Choose an absolute installation path, such as C:\\Users\\you\\AppData\\Local\\MyApp.");
        }
        if path.exists() && !path.is_dir() {
            bail!("The installation path points to a file.");
        }
        self.selection.target_dir = path;
        Ok(())
    }

    pub(super) unsafe fn command(
        &mut self,
        hwnd: HWND,
        _fonts: &Fonts,
        id: u16,
        notification: u16,
    ) -> Result<Action> {
        if self.busy() {
            return Ok(Action::None);
        }
        match id {
            ACCEPT if notification == BN_CLICKED as u16 => {
                self.accepted =
                    SendMessageW(self.h(ACCEPT), BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                enabled(self.h(NEXT), self.accepted);
            }
            BROWSE => {
                if let Some(path) = folder_dialog(hwnd)? {
                    set_text(self.h(DIRECTORY), &path.to_string_lossy());
                }
            }
            COMPONENTS | TASKS if notification == LBN_SELCHANGE as u16 => {
                self.capture_lists()?;
                if let Some(index) = self.combo_types.iter().position(Option::is_none) {
                    SendMessageW(self.h(TYPE), CB_SETCURSEL, index, 0);
                }
            }
            TYPE if notification == CBN_SELCHANGE as u16 => {
                let index = SendMessageW(self.h(TYPE), CB_GETCURSEL, 0, 0);
                if index >= 0 {
                    if let Some(Some(name)) = self.combo_types.get(index as usize) {
                        let (components, tasks) =
                            flow::default_selections(&self.manifest, Some(name))?;
                        self.selection.components = components;
                        self.selection.tasks = tasks;
                        self.sync_lists()?;
                    }
                }
            }
            NEXT | 1 => {
                match self.page() {
                    Page::Complete => return Ok(Action::Close),
                    Page::License if !self.accepted => {
                        bail!("Accept the license agreement before continuing.")
                    }
                    Page::Destination => self.capture_directory()?,
                    Page::Features => self.capture_lists()?,
                    Page::Ready => {
                        // This guard is independent from the disabled button and
                        // therefore also applies to keyboard/default-button paths.
                        if self.manifest.license_text.is_some() && !self.accepted {
                            bail!("The license agreement has not been accepted.");
                        }
                        self.begin_install()?;
                        self.render(hwnd)?;
                        return Ok(Action::None);
                    }
                    _ => (),
                }
                self.step += 1;
                self.render(hwnd)?;
                SetFocus(self.h(NEXT));
            }
            BACK => {
                if self.step > 0 && self.page() != Page::Complete {
                    if self.page() == Page::Features {
                        self.capture_lists()?;
                    }
                    self.step -= 1;
                    self.render(hwnd)?;
                }
            }
            CANCEL | 2 => return self.close(hwnd),
            _ => (),
        }
        Ok(Action::None)
    }

    fn begin_install(&mut self) -> Result<()> {
        let perform = self
            .perform
            .take()
            .ok_or_else(|| anyhow!("Installation has already been started"))?;
        let selection = self.selection.clone();
        let (sender, receiver) = mpsc::channel();
        let progress_step = self
            .route
            .iter()
            .position(|page| *page == Page::Progress)
            .ok_or_else(|| anyhow!("Installation route is incomplete"))?;
        thread::Builder::new().name("ironsetup-install".into()).spawn(move || {
            let progress_sender = sender.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                perform(selection, Box::new(move |progress| { let _ = progress_sender.send(WorkerEvent::Progress(progress)); }))
            }));
            let result = match result {
                Ok(result) => result.map_err(|error| format!("{error:#}")),
                Err(_) => Err("The installer worker encountered an internal error. Inspect the installation journal before retrying.".into()),
            };
            let _ = sender.send(WorkerEvent::Finished(result));
        }).context("Cannot start installation worker")?;
        self.receiver = Some(receiver);
        self.step = progress_step;
        Ok(())
    }

    pub(super) unsafe fn poll(&mut self, hwnd: HWND, _fonts: &Fonts) -> Result<Action> {
        let Some(receiver) = &self.receiver else {
            return Ok(Action::None);
        };
        let mut latest = None;
        let mut finished = None;
        for event in receiver.try_iter().take(1024) {
            match event {
                WorkerEvent::Progress(progress) => latest = Some(progress),
                WorkerEvent::Finished(result) => {
                    finished = Some(result);
                    break;
                }
            }
        }
        if let Some(progress) = latest {
            set_text(
                self.h(STATUS),
                &format!("{}\r\n{}", progress.stage, progress.item),
            );
            let amount = if progress.total == 0 {
                0
            } else {
                progress
                    .completed
                    .saturating_mul(1000)
                    .checked_div(progress.total)
                    .unwrap_or(0)
                    .min(1000)
            };
            SendMessageW(self.h(PROGRESS), PBM_SETPOS, amount as usize, 0);
        }
        if let Some(result) = finished {
            self.receiver = None;
            match result {
                Ok(details) => {
                    self.outcome = Some(Ok(()));
                    self.finish_text = details;
                    if let Some(after) = &self.manifest.info_after {
                        self.finish_text.push_str("\r\n\r\n");
                        self.finish_text.push_str(after);
                    }
                    SendMessageW(self.h(PROGRESS), PBM_SETPOS, 1000, 0);
                }
                Err(error) => {
                    self.finish_text = format!("Installation failed.\r\n\r\n{error}\r\n\r\nReview this message and the installation log before trying again.");
                    self.outcome = Some(Err(error));
                }
            }
            self.step = self
                .route
                .iter()
                .position(|page| *page == Page::Complete)
                .unwrap_or(self.route.len() - 1);
            self.render(hwnd)?;
            SetFocus(self.h(NEXT));
        }
        Ok(Action::None)
    }

    pub(super) unsafe fn close(&mut self, hwnd: HWND) -> Result<Action> {
        if self.busy() {
            return Ok(Action::None);
        }
        if self.page() == Page::Complete {
            return Ok(Action::Close);
        }
        if dialog(
            hwnd,
            "Cancel installation",
            "Cancel setup? No installation changes have been made yet.",
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        ) == IDYES
        {
            return Ok(Action::Close);
        }
        Ok(Action::None)
    }

    pub(super) fn outcome(&self) -> Result<()> {
        match &self.outcome {
            Some(Ok(())) => Ok(()),
            Some(Err(message)) => Err(crate::WizardError::OperationFailed(message.clone()).into()),
            None => Err(crate::WizardError::Cancelled.into()),
        }
    }
}

fn list_names(names: &BTreeSet<String>) -> String {
    if names.is_empty() {
        "None".into()
    } else {
        names.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}
