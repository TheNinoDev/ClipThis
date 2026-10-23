use std::thread;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint,
    EnumDisplayMonitors, FillRect, GetMonitorInfoW, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, TextOutW, HMONITOR, MONITORINFO, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, ShowWindow, TranslateMessage,
    SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, LWA_ALPHA,
};

const CLASS_NAME: PCWSTR = w!("ClipThisToast");
const TIMER_ID: usize = 1;
const TOAST_MS: u32 = 4000;
const WM_SHOW: u32 = WM_APP + 1;

static TOAST_TEXT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static TOAST_HWNS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

#[derive(Clone, Copy)]
pub struct ToastService;

impl ToastService {
    pub fn start() -> Self {
        thread::spawn(|| run());
        Self
    }

    pub fn show(&self, msg: &str) {
        *TOAST_TEXT.lock().unwrap() = msg.to_string();
        if let Some(raw) = TOAST_HWNS.lock().unwrap().first().copied() {
            let h = HWND(raw as *mut _);
            unsafe {
                let _ = PostMessageW(h, WM_SHOW, WPARAM(0), LPARAM(0));
            }
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
            let brush = CreateSolidBrush(COLORREF(0x002e1e1e));
            FillRect(hdc, &rect, brush);
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(0x00ffffff));
            let font = CreateFontW(16, 0, 0, 0, 600, 0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"));
            let hgdi: windows::Win32::Graphics::Gdi::HGDIOBJ = font.into();
            let old = SelectObject(hdc, hgdi);
            let text = TOAST_TEXT.lock().unwrap().clone();
            let wide: Vec<u16> = text.encode_utf16().collect();
            let _ = TextOutW(hdc, 12, 12, &wide);
            let _ = SelectObject(hdc, old);
            let hgdi_font: windows::Win32::Graphics::Gdi::HGDIOBJ = font.into();
            let _ = DeleteObject(hgdi_font);
            let hgdi_brush: windows::Win32::Graphics::Gdi::HGDIOBJ = brush.into();
            let _ = DeleteObject(hgdi_brush);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            for raw in TOAST_HWNS.lock().unwrap().iter() {
                let h = HWND(*raw as *mut _);
                let _ = ShowWindow(h, SW_HIDE);
            }
            let _ = KillTimer(hwnd, TIMER_ID);
            LRESULT(0)
        }
        WM_SHOW => {
            for raw in TOAST_HWNS.lock().unwrap().iter() {
                let h = HWND(*raw as *mut _);
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            }
            let _ = SetTimer(hwnd, TIMER_ID, TOAST_MS, None);
            LRESULT(0)
        }
        WM_DESTROY => {
            windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn monitor_enum_proc(
    hmon: HMONITOR,
    _hdc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut RECT,
    _data: LPARAM,
) -> windows::Win32::Foundation::BOOL {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut info).as_bool() {
        if let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            CLASS_NAME,
            w!(""),
            WS_POPUP,
            info.rcMonitor.left + 16,
            info.rcMonitor.top + 16,
            400,
            44,
            None,
            None,
            None,
            None,
        ) {
            let rgn = CreateRoundRectRgn(0, 0, 401, 45, 12, 12);
            let _ = SetWindowRgn(hwnd, rgn, true);
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 235, LWA_ALPHA);
            TOAST_HWNS.lock().unwrap().push(hwnd.0 as usize);
        }
    }
    windows::Win32::Foundation::BOOL(1)
}

fn run() {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: CLASS_NAME,
            hbrBackground: CreateSolidBrush(COLORREF(0x002e1e1e)),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let _ = EnumDisplayMonitors(None, None, Some(monitor_enum_proc), LPARAM(0));

        for raw in TOAST_HWNS.lock().unwrap().iter() {
            let h = HWND(*raw as *mut _);
            let _ = ShowWindow(h, SW_HIDE);
        }

        loop {
            let mut m = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            if GetMessageW(&mut m, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            } else {
                break;
            }
        }
    }
}