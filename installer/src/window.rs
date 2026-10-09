//! Small native window on Windows. It shows that the assistant is running, offers to reopen
//! the page and ends the assistant when closed, so the program never runs invisibly.

use crate::server::App;
use anyhow::{Result, bail};
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicIsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CLEARTYPE_QUALITY, COLOR_WINDOW, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
        Ellipse, EndPaint, FW_NORMAL, FW_SEMIBOLD, FillRect, GetStockObject, GetSysColorBrush,
        HBRUSH, HDC, HFONT, InvalidateRect, NULL_PEN, PAINTSTRUCT, SelectObject, SetBkMode,
        SetTextColor, TRANSPARENT,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "SimpleAdminAssistantWindow";
const TITLE: &str = "SimpleAdmin 设备助手";
/// IDOK, so Enter opens the page. Exit deliberately avoids IDCANCEL so Esc does not quit.
const ID_OPEN: usize = 1;
const ID_QUIT: usize = 3;
const WM_SERVER_STOPPED: u32 = WM_APP + 1;
/// Client size and the top of the button band, in 96-DPI pixels.
const WIDTH: i32 = 400;
const HEIGHT: i32 = 184;
const BAND: i32 = 112;

static WINDOW: AtomicIsize = AtomicIsize::new(0);

struct Ui {
    app: Arc<App>,
    url: String,
    dpi: u32,
    font: HFONT,
    title_font: HFONT,
    icon: HICON,
    band: HBRUSH,
    buttons: [HWND; 2],
    status: (Option<&'static str>, bool),
}
thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
fn px(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}
fn rect(left: i32, top: i32, right: i32, bottom: i32, dpi: u32) -> RECT {
    RECT {
        left: px(left, dpi),
        top: px(top, dpi),
        right: px(right, dpi),
        bottom: px(bottom, dpi),
    }
}

unsafe fn font(tenths_of_point: i32, weight: u32, dpi: u32) -> HFONT {
    let face = wide("Microsoft YaHei UI");
    // Zero precision, clipping and pitch values select the system defaults.
    unsafe {
        CreateFontW(
            -(tenths_of_point * dpi as i32 / 720),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    }
}
unsafe fn load_icon(size: i32) -> HICON {
    // MAKEINTRESOURCE(1): the application icon embedded by build.rs.
    unsafe {
        LoadImageW(
            GetModuleHandleW(null()),
            std::ptr::without_provenance::<u16>(1),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        ) as HICON
    }
}

/// Creates fonts and the icon for `dpi` and places the buttons.
unsafe fn apply_dpi(dpi: u32) {
    let (buttons, font_handle) = UI.with(|cell| {
        let mut cell = cell.borrow_mut();
        let ui = cell.as_mut().unwrap();
        unsafe {
            DeleteObject(ui.font);
            DeleteObject(ui.title_font);
            DestroyIcon(ui.icon);
            ui.dpi = dpi;
            ui.font = font(90, FW_NORMAL, dpi);
            ui.title_font = font(120, FW_SEMIBOLD, dpi);
            ui.icon = load_icon(px(48, dpi));
        }
        (ui.buttons, ui.font)
    });
    let places = [(174, 104), (288, 88)];
    for (button, (left, width)) in buttons.into_iter().zip(places) {
        unsafe {
            SetWindowPos(
                button,
                null_mut(),
                px(left, dpi),
                px(136, dpi),
                px(width, dpi),
                px(30, dpi),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(button, WM_SETFONT, font_handle as WPARAM, 1);
        }
    }
}

unsafe fn paint_background(hwnd: HWND, hdc: HDC) {
    let Some((dpi, band)) = UI.with(|cell| cell.borrow().as_ref().map(|ui| (ui.dpi, ui.band)))
    else {
        return;
    };
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetClientRect(hwnd, &mut client);
        let top = RECT {
            bottom: px(BAND, dpi),
            ..client
        };
        FillRect(hdc, &top, GetSysColorBrush(COLOR_WINDOW));
        let bottom = RECT {
            top: px(BAND, dpi),
            ..client
        };
        FillRect(hdc, &bottom, band);
        let line = CreateSolidBrush(rgb(229, 229, 229));
        let separator = RECT {
            top: px(BAND, dpi),
            bottom: px(BAND, dpi) + 1.max(px(1, dpi)),
            ..client
        };
        FillRect(hdc, &separator, line);
        DeleteObject(line);
    }
}

unsafe fn draw_text(hdc: HDC, text: &str, mut area: RECT, color: COLORREF) {
    let text = wide(text);
    unsafe {
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            text.as_ptr(),
            -1,
            &mut area,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    UI.with(|cell| {
        let cell = cell.borrow();
        let Some(ui) = cell.as_ref() else { return };
        let dpi = ui.dpi;
        let (busy, page) = ui.status;
        let (status, dot) = match busy {
            None => ("正在运行 · 空闲".to_owned(), rgb(52, 199, 89)),
            Some(operation) => (format!("正在运行 · {operation}进行中"), rgb(255, 149, 0)),
        };
        let detail = format!(
            "{} · 本机端口 {}",
            if page {
                "网页已打开"
            } else {
                "网页未打开"
            },
            ui.app.port
        );
        let gray = rgb(110, 110, 115);
        unsafe {
            SetBkMode(hdc, TRANSPARENT as i32);
            DrawIconEx(
                hdc,
                px(24, dpi),
                px(24, dpi),
                ui.icon,
                px(48, dpi),
                px(48, dpi),
                0,
                null_mut(),
                DI_NORMAL,
            );
            SelectObject(hdc, ui.title_font);
            draw_text(hdc, TITLE, rect(88, 22, 376, 48, dpi), rgb(29, 29, 31));
            SelectObject(hdc, ui.font);
            let brush = CreateSolidBrush(dot);
            let old_brush = SelectObject(hdc, brush);
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            Ellipse(hdc, px(89, dpi), px(58, dpi), px(98, dpi), px(67, dpi));
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            DeleteObject(brush);
            draw_text(hdc, &status, rect(104, 50, 376, 74, dpi), rgb(29, 29, 31));
            draw_text(hdc, &detail, rect(88, 74, 376, 96, dpi), gray);
            draw_text(hdc, "关闭此窗口即退出", rect(24, 136, 170, 166, dpi), gray);
        }
    });
    unsafe { EndPaint(hwnd, &ps) };
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let app = || UI.with(|cell| cell.borrow().as_ref().map(|ui| ui.app.clone()));
    unsafe {
        match message {
            WM_ERASEBKGND => {
                paint_background(hwnd, wparam as HDC);
                1
            }
            WM_PRINTCLIENT => {
                paint_background(hwnd, wparam as HDC);
                0
            }
            WM_CTLCOLORBTN => {
                UI.with(|cell| cell.borrow().as_ref().map_or(0, |ui| ui.band as LRESULT))
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_TIMER => {
                let changed = UI.with(|cell| {
                    let mut cell = cell.borrow_mut();
                    let ui = cell.as_mut()?;
                    let status = ui.app.summary();
                    (status != ui.status).then(|| ui.status = status)
                });
                if changed.is_some() {
                    InvalidateRect(hwnd, null(), 1);
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xffff {
                    ID_OPEN => {
                        let url = UI.with(|cell| cell.borrow().as_ref().map(|ui| ui.url.clone()));
                        if let Some(url) = url {
                            let _ = crate::open_url(&url);
                        }
                    }
                    ID_QUIT => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                // No RefCell borrow is held here: the message box runs a nested message loop.
                if let Some(operation) = app().and_then(|app| app.summary().0) {
                    let text = wide(&format!(
                        "{operation}正在进行。现在退出会中断操作，模块可能处于未完成状态。\n\n仍要退出吗？"
                    ));
                    let caption = wide(TITLE);
                    let answer = MessageBoxW(
                        hwnd,
                        text.as_ptr(),
                        caption.as_ptr(),
                        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                    );
                    if answer != IDYES {
                        return 0;
                    }
                }
                DestroyWindow(hwnd);
                0
            }
            WM_SERVER_STOPPED => {
                DestroyWindow(hwnd);
                0
            }
            WM_DPICHANGED => {
                apply_dpi(((wparam >> 16) & 0xffff) as u32);
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                InvalidateRect(hwnd, null(), 1);
                0
            }
            WM_DESTROY => {
                WINDOW.store(0, Ordering::SeqCst);
                KillTimer(hwnd, 1);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// Shows the window and runs its message loop until the user closes it or the server stops.
pub fn run(app: Arc<App>, url: String) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide(CLASS);
        let title = wide(TITLE);
        let class_info = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: load_icon(GetSystemMetrics(SM_CXICON)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: load_icon(GetSystemMetrics(SM_CXSMICON)),
        };
        if RegisterClassExW(&class_info) == 0 {
            bail!("无法注册设备助手窗口")
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            bail!("无法创建设备助手窗口")
        }
        let button = |text: &str, id: usize, kind: i32| {
            let text = wide(text);
            let class = wide("BUTTON");
            CreateWindowExW(
                0,
                class.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | kind as u32,
                0,
                0,
                0,
                0,
                hwnd,
                id as HMENU,
                instance,
                null(),
            )
        };
        let buttons = [
            button("打开网页", ID_OPEN, BS_DEFPUSHBUTTON),
            button("退出", ID_QUIT, BS_PUSHBUTTON),
        ];
        let status = app.summary();
        UI.with(|cell| {
            *cell.borrow_mut() = Some(Ui {
                app,
                url,
                dpi: 96,
                font: null_mut(),
                title_font: null_mut(),
                icon: null_mut(),
                band: CreateSolidBrush(rgb(243, 243, 243)),
                buttons,
                status,
            })
        });
        let dpi = GetDpiForWindow(hwnd);
        apply_dpi(dpi);
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: px(WIDTH, dpi),
            bottom: px(HEIGHT, dpi),
        };
        AdjustWindowRectExForDpi(&mut frame, style, 0, 0, dpi);
        let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
        SetWindowPos(
            hwnd,
            null_mut(),
            (GetSystemMetrics(SM_CXSCREEN) - width) / 2,
            (GetSystemMetrics(SM_CYSCREEN) - height) / 3,
            width,
            height,
            SWP_NOZORDER,
        );
        WINDOW.store(hwnd as isize, Ordering::SeqCst);
        SetTimer(hwnd, 1, 1000, None);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetFocus(buttons[0]);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Some(ui) = UI.with(|cell| cell.borrow_mut().take()) {
            DeleteObject(ui.font);
            DeleteObject(ui.title_font);
            DeleteObject(ui.band);
            DestroyIcon(ui.icon);
        }
    }
    Ok(())
}

/// Closes the window after the server has stopped, e.g. through "退出" on the page.
pub fn server_stopped() {
    let hwnd = WINDOW.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe { PostMessageW(hwnd as HWND, WM_SERVER_STOPPED, 0, 0) };
    }
}

/// Brings the window of an already running assistant to the front.
pub fn activate_existing() {
    let class = wide(CLASS);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), null());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}
