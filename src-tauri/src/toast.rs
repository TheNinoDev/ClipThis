use std::thread;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    EnumDisplayMonitors, FillRect, GetMonitorInfoW, InvalidateRect, SelectObject, SetBkMode,
    SetTextColor, SetWindowRgn, TextOutW, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    HMONITOR, MONITORINFO, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, WM_APP, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("ClipThisToast");
const TIMER_DISMISS: usize = 1;
const TIMER_ANIM: usize = 2;
const TOTAL_ENTER_STEPS: u32 = 14; // 14 * 16ms = ~224ms
const TOTAL_EXIT_STEPS: u32 = 10;  // 10 * 16ms = ~160ms
const DISMISS_DELAY_MS: u32 = 3200;
const WM_SHOW: u32 = WM_APP + 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimState {
    Hidden,
    Entering(u32),
    Showing,
    Exiting(u32),
}

struct ToastWindow {
    hwnd: usize,
    base_x: i32,
    base_y: i32,
}

static TOAST_TEXT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static TOAST_WINDOWS: std::sync::Mutex<Vec<ToastWindow>> = std::sync::Mutex::new(Vec::new());
static ANIM_STATE: std::sync::Mutex<AnimState> = std::sync::Mutex::new(AnimState::Hidden);

#[derive(Clone, Copy)]
pub struct ToastService;

impl ToastService {
    pub fn start() -> Self {
        thread::spawn(|| run());
        Self
    }

    pub fn show(&self, msg: &str) {
        *TOAST_TEXT.lock().unwrap() = msg.to_string();
        if let Some(first) = TOAST_WINDOWS.lock().unwrap().first() {
            unsafe {
                let _ = PostMessageW(HWND(first.hwnd as *mut _), WM_SHOW, WPARAM(0), LPARAM(0));
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

            // 1. Background #1D1F29 (Figma card container background)
            let bg_brush = CreateSolidBrush(COLORREF(0x00291f1d));
            FillRect(hdc, &rect, bg_brush);
            let hgdi_bg: windows::Win32::Graphics::Gdi::HGDIOBJ = bg_brush.into();
            let _ = DeleteObject(hgdi_bg);

            // 2. Left vertical accent indicator #5B8CFF (matches circular play button & waveform)
            let accent_brush = CreateSolidBrush(COLORREF(0x00ff8c5b));
            let accent_rect = RECT { left: 16, top: 15, right: 20, bottom: 49 };
            FillRect(hdc, &accent_rect, accent_brush);
            let hgdi_acc: windows::Win32::Graphics::Gdi::HGDIOBJ = accent_brush.into();
            let _ = DeleteObject(hgdi_acc);

            SetBkMode(hdc, TRANSPARENT);

            // 3. Header badge: "CLIPTHIS" in bold #5B8CFF
            let title_font = CreateFontW(13, 0, 0, 0, 700, 0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"));
            let old_font = SelectObject(hdc, title_font);
            SetTextColor(hdc, COLORREF(0x00ff8c5b));
            let title_wide: Vec<u16> = "CLIPTHIS".encode_utf16().collect();
            let _ = TextOutW(hdc, 28, 14, &title_wide);

            // 4. Message: clean white #FFFFFF text with end-ellipsis clipping
            let msg_font = CreateFontW(15, 0, 0, 0, 500, 0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"));
            let _ = SelectObject(hdc, msg_font);
            SetTextColor(hdc, COLORREF(0x00ffffff));
            let text = TOAST_TEXT.lock().unwrap().clone();
            let mut msg_wide: Vec<u16> = text.encode_utf16().collect();
            let mut text_rect = RECT {
                left: 28,
                top: 33,
                right: rect.right - 16,
                bottom: 53,
            };
            let _ = DrawTextW(
                hdc,
                &mut msg_wide,
                &mut text_rect,
                DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );

            let _ = SelectObject(hdc, old_font);
            let hgdi_tfont: windows::Win32::Graphics::Gdi::HGDIOBJ = title_font.into();
            let _ = DeleteObject(hgdi_tfont);
            let hgdi_mfont: windows::Win32::Graphics::Gdi::HGDIOBJ = msg_font.into();
            let _ = DeleteObject(hgdi_mfont);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SHOW => {
            *ANIM_STATE.lock().unwrap() = AnimState::Entering(0);
            let _ = KillTimer(hwnd, TIMER_DISMISS);

            let windows = TOAST_WINDOWS.lock().unwrap();
            for win in windows.iter() {
                let h = HWND(win.hwnd as *mut _);
                let _ = InvalidateRect(h, None, true);
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA);
                let _ = SetWindowPos(
                    h,
                    HWND_TOPMOST,
                    win.base_x,
                    win.base_y - 20,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            let _ = SetTimer(hwnd, TIMER_ANIM, 16, None);
            LRESULT(0)
        }
        WM_TIMER => {
            let timer_id = wparam.0;
            if timer_id == TIMER_ANIM {
                let mut state = ANIM_STATE.lock().unwrap();
                match *state {
                    AnimState::Entering(step) => {
                        let next_step = step + 1;
                        let t = (next_step as f32 / TOTAL_ENTER_STEPS as f32).min(1.0);
                        // Cubic ease-out: 1 - (1 - t)^3
                        let ease = 1.0 - (1.0 - t).powi(3);
                        let alpha = (252.0 * ease).round() as u8;

                        let windows = TOAST_WINDOWS.lock().unwrap();
                        for win in windows.iter() {
                            let h = HWND(win.hwnd as *mut _);
                            let cur_y = win.base_y - (20.0 * (1.0 - ease)).round() as i32;
                            let _ = SetLayeredWindowAttributes(h, COLORREF(0), alpha, LWA_ALPHA);
                            let _ = SetWindowPos(
                                h,
                                HWND_TOPMOST,
                                win.base_x,
                                cur_y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                        }

                        if next_step >= TOTAL_ENTER_STEPS {
                            *state = AnimState::Showing;
                            let _ = KillTimer(hwnd, TIMER_ANIM);
                            let _ = SetTimer(hwnd, TIMER_DISMISS, DISMISS_DELAY_MS, None);
                        } else {
                            *state = AnimState::Entering(next_step);
                        }
                    }
                    AnimState::Exiting(step) => {
                        let next_step = step + 1;
                        let t = (next_step as f32 / TOTAL_EXIT_STEPS as f32).min(1.0);
                        let ease = t * t;
                        let alpha = (252.0 * (1.0 - ease)).max(0.0).round() as u8;

                        let windows = TOAST_WINDOWS.lock().unwrap();
                        for win in windows.iter() {
                            let h = HWND(win.hwnd as *mut _);
                            let cur_y = win.base_y - (14.0 * ease).round() as i32;
                            let _ = SetLayeredWindowAttributes(h, COLORREF(0), alpha, LWA_ALPHA);
                            let _ = SetWindowPos(
                                h,
                                HWND_TOPMOST,
                                win.base_x,
                                cur_y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                        }

                        if next_step >= TOTAL_EXIT_STEPS {
                            *state = AnimState::Hidden;
                            let _ = KillTimer(hwnd, TIMER_ANIM);
                            for win in windows.iter() {
                                let h = HWND(win.hwnd as *mut _);
                                let _ = ShowWindow(h, SW_HIDE);
                            }
                        } else {
                            *state = AnimState::Exiting(next_step);
                        }
                    }
                    _ => {
                        let _ = KillTimer(hwnd, TIMER_ANIM);
                    }
                }
                LRESULT(0)
            } else if timer_id == TIMER_DISMISS {
                let _ = KillTimer(hwnd, TIMER_DISMISS);
                *ANIM_STATE.lock().unwrap() = AnimState::Exiting(0);
                let _ = SetTimer(hwnd, TIMER_ANIM, 16, None);
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
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
        let base_x = info.rcMonitor.left + 24;
        let base_y = info.rcMonitor.top + 24;
        let width = 410;
        let height = 64;

        if let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            CLASS_NAME,
            w!(""),
            WS_POPUP,
            base_x,
            base_y,
            width,
            height,
            None,
            None,
            None,
            None,
        ) {
            let rgn = CreateRoundRectRgn(0, 0, width + 1, height + 1, 16, 16);
            let _ = SetWindowRgn(hwnd, rgn, true);
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
            TOAST_WINDOWS.lock().unwrap().push(ToastWindow { hwnd: hwnd.0 as usize, base_x, base_y });
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
            hbrBackground: CreateSolidBrush(COLORREF(0x00291f1d)),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let _ = EnumDisplayMonitors(None, None, Some(monitor_enum_proc), LPARAM(0));

        let windows = TOAST_WINDOWS.lock().unwrap();
        for win in windows.iter() {
            let h = HWND(win.hwnd as *mut _);
            let _ = ShowWindow(h, SW_HIDE);
        }
        drop(windows);

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
