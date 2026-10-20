//! WebView2ウィンドウ。
//!
//! AviUtl2の編集APIはホストメインスレッドで呼び出す必要があるため、親HWNDと
//! WebView2を汎用プラグインの初期化スレッドで作成する。Rust→JS通知だけは
//! `PostMessageW`で同じスレッドへ戻す。

use std::collections::VecDeque;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use aviutl2::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_SPACE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, EnumWindows, GetClassNameW,
    GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, RegisterClassExW,
    WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SIZE, WNDCLASSEXW, WS_POPUP,
};
use windows_sys::core::BOOL;
use wry::{Rect, WebContext, WebView, WebViewBuilder};

const CLASS_NAME: &[u16] = &[
    b'H' as u16,
    b'L' as u16,
    b'_' as u16,
    b'E' as u16,
    b'z' as u16,
    b'i' as u16,
    b'n' as u16,
    b'g' as u16,
    b'H' as u16,
    b'o' as u16,
    b's' as u16,
    b't' as u16,
    0,
];
const WM_HL_EVALUATE: u32 = 0x8000;

static HOST_HWND: OnceLock<NonZeroIsize> = OnceLock::new();
static WEBVIEW_PTR: AtomicPtr<WebView> = AtomicPtr::new(std::ptr::null_mut());
static EVAL_QUEUE: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn eval_queue() -> &'static Mutex<VecDeque<String>> {
    EVAL_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// AviUtl2のウィンドウクライアントとして登録するHWNDを保持する。
pub struct HlEzingWindow {
    hwnd: NonZeroIsize,
}

impl HlEzingWindow {
    /// ホストメインスレッド上でWebView2を作成する。
    pub fn create() -> Result<Self> {
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: CLASS_NAME.as_ptr() as _,
            hIconSm: std::ptr::null_mut(),
        };
        unsafe {
            RegisterClassExW(&class);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                0,
                CLASS_NAME.as_ptr() as _,
                CLASS_NAME.as_ptr() as _,
                WS_POPUP,
                0,
                0,
                300,
                200,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(anyhow!("CreateWindowExW failed: {}", unsafe {
                GetLastError()
            }));
        }
        let hwnd = NonZeroIsize::new(hwnd as isize).context("Window HWND is null")?;
        HOST_HWND.set(hwnd).ok();
        let parent = ParentHandle(hwnd);

        let mut context = WebContext::new(Some(
            aviutl2::config::app_data_path()
                .join("Plugin")
                .join("HL_Ezing")
                .join("webview2"),
        ));
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(crate::ipc::FRONTEND_URL)
            .with_custom_protocol("hl-ezing".into(), crate::ipc::serve_asset)
            .with_navigation_handler(|url| crate::ipc::is_trusted_ui_uri(&url))
            .with_drag_drop_handler(|_| false)
            .with_ipc_handler(crate::ipc::handle_ipc)
            .with_devtools(cfg!(debug_assertions))
            .with_bounds(Rect {
                position: wry::dpi::Position::Physical(wry::dpi::PhysicalPosition::new(0, 0)),
                size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::new(300, 200)),
            })
            .build_as_child(&parent)
            .context("Failed to build WebView")?;

        // WebView2は作成スレッドに束縛されるため、DLLの終了まで解放しない。
        // Rust→JS処理はwindow_procからのみこのポインタを参照する。
        let webview = Box::into_raw(Box::new(webview));
        WEBVIEW_PTR.store(webview, Ordering::Release);

        Ok(Self { hwnd })
    }
}

impl HasWindowHandle for HlEzingWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = Win32WindowHandle::new(self.hwnd);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

/// プラグインのホストHWNDを返す。トラックバー探索の座標系確認用。
pub fn host_hwnd() -> Option<HWND> {
    HOST_HWND.get().map(|hwnd| hwnd.get() as HWND)
}

struct MainWindowSearch {
    best: HWND,
    best_area: i64,
}

unsafe extern "system" fn find_main_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = unsafe { &mut *(lparam as *mut MainWindowSearch) };
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }

    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if process_id != unsafe { GetCurrentProcessId() } {
        return 1;
    }

    let mut class_name = [0u16; 128];
    let class_len =
        unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
    if class_len <= 0 {
        return 1;
    }
    let class_name = String::from_utf16_lossy(&class_name[..class_len as usize]);
    // HL_Ezing 自身のホストウィンドウやドラッグ用オーバーレイは除外する。
    if class_name.starts_with("HL_Ezing") {
        return 1;
    }

    // 通常は AviUtl2 のメインウィンドウ。クラス名が変わった場合に備え、
    // 見つからなければ可視トップレベルの中で最大のものをフォールバックにする。
    if class_name == "aviutl2Manager" {
        state.best = hwnd;
        state.best_area = i64::MAX;
        return 1;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return 1;
    }
    let width = (rect.right - rect.left).max(0) as i64;
    let height = (rect.bottom - rect.top).max(0) as i64;
    let area = width * height;
    if area > state.best_area {
        state.best = hwnd;
        state.best_area = area;
    }
    1
}

/// AviUtl2 本体のメインウィンドウを返す。
///
/// プラグインウィンドウが分離されている場合、ホストの `GA_ROOT` は
/// プラグイン自身になるため使えない。同一プロセスの可視トップレベル
/// ウィンドウから、HL_Ezing 以外で最も大きいものを本体とみなす。
pub fn aviutl2_main_hwnd() -> Option<HWND> {
    let mut search = MainWindowSearch {
        best: std::ptr::null_mut(),
        best_area: 0,
    };
    unsafe {
        EnumWindows(
            Some(find_main_window_proc),
            &mut search as *mut MainWindowSearch as LPARAM,
        );
    }
    (!search.best.is_null()).then_some(search.best)
}

/// WebView にフォーカスがある状態でも、AviUtl2 本体の Space 再生/停止を
/// 実行できるようにキー入力を転送する。
pub fn forward_space_to_main() -> bool {
    let Some(hwnd) = aviutl2_main_hwnd() else {
        tracing::warn!("AviUtl2 main window was not found for Space forwarding");
        return false;
    };
    let key = VK_SPACE as WPARAM;
    let scan = unsafe { MapVirtualKeyW(VK_SPACE as u32, MAPVK_VK_TO_VSC) } as LPARAM;
    // keydown: repeat=1 + scan code。keyup: previous=1, transition=1 も付ける。
    let down_lparam = 1 | (scan << 16);
    let up_lparam = 1 | (scan << 16) | (1 << 30) | (1 << 31);
    let down = unsafe { PostMessageW(hwnd, WM_KEYDOWN, key, down_lparam) };
    let up = unsafe { PostMessageW(hwnd, WM_KEYUP, key, up_lparam) };
    if down == 0 || up == 0 {
        tracing::warn!("Failed to forward Space to AviUtl2 main window");
        return false;
    }
    tracing::debug!("Forwarded Space to AviUtl2 main window");
    true
}

/// 任意スレッドからWebView2の作成スレッドへJS評価を依頼する。
pub fn evaluate(script: String) {
    eval_queue().lock().unwrap().push_back(script);
    if let Some(hwnd) = HOST_HWND.get() {
        unsafe {
            PostMessageW(hwnd.get() as HWND, WM_HL_EVALUATE, 0 as WPARAM, 0 as LPARAM);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let webview = WEBVIEW_PTR.load(Ordering::Acquire);
    match msg {
        WM_HL_EVALUATE if !webview.is_null() => {
            loop {
                let script = {
                    let mut queue = eval_queue().lock().unwrap();
                    queue.pop_front()
                };
                let Some(script) = script else {
                    break;
                };
                let _ = unsafe { &*webview }.evaluate_script(&script);
            }
            0
        }
        WM_SIZE if !webview.is_null() => {
            let lp = lparam as u32;
            let width = lp & 0xFFFF;
            let height = (lp >> 16) & 0xFFFF;
            let _ = unsafe { &*webview }.set_bounds(Rect {
                position: wry::dpi::Position::Physical(wry::dpi::PhysicalPosition::new(0, 0)),
                size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::new(width, height)),
            });
            0
        }
        WM_DESTROY => 0,
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

struct ParentHandle(NonZeroIsize);

impl HasWindowHandle for ParentHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = Win32WindowHandle::new(self.0);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}
