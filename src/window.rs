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
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, PostMessageW, RegisterClassExW,
    WM_DESTROY, WM_SIZE, WNDCLASSEXW, WS_POPUP,
};
use wry::{Rect, WebContext, WebView, WebViewBuilder};

const CLASS_NAME: &[u8] = b"HL_EzingHost\0";
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
            .with_ipc_handler(crate::ipc::handle_ipc)
            .with_devtools(true)
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
            while let Some(script) = eval_queue().lock().unwrap().pop_front() {
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
