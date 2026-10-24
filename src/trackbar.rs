//! AviUtl2 のトラックバーを実行時に探索するためのモジュール。
//!
//! AviUtl2 の設定画面は Direct2D で描画されており、トラックバーは個別の
//! HWND ではない。公開 SDK にも「カーソル下のトラックバー」を取得する API が
//! ないため、このモジュールでは自プロセス内の `EffectItemNumberControl`
//! オブジェクトを走査して、対応する `TrackBarControl` の矩形を取得する。
//!
//! バージョンアップ対策:
//! - vtable は絶対アドレスを埋め込まず、PE の RTTI からクラス名で解決する。
//! - レイアウトのオフセットだけを `LayoutProfile` に集約する。
//! - 解決に失敗した場合はドラッグ適用を無効化し、既存のクリック適用だけ残す。
//! - 診断情報は `debug_snapshot()` / メニューからログに出力できる。
//!
//! バージョンアップで動かなくなったときは、まず AviUtl2 のログに出力された
//! `HL_Ezing trackbar diagnostics` を確認し、`LayoutProfile` のオフセットを
//! 更新する。vtable の解決自体は RTTI 名が変わらない限り自動で追従する。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, RGN_DIFF,
    SetWindowRgn,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
    PAGE_WRITECOPY, VirtualQuery,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GA_ROOT, GetAncestor, GetCursorPos,
    GetWindowThreadProcessId, HWND_TOPMOST, IsChild, LWA_ALPHA, RegisterClassExW, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP, WindowFromPoint,
};

/// AviUtl2 のバージョン。`AviUtl2Info::version.0` をそのまま保持する。
static VERSION: AtomicU32 = AtomicU32::new(0);

/// ドラッグ監視スレッドの多重起動防止。
static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);
static VTABLES: OnceLock<Option<Vtables>> = OnceLock::new();

struct TargetCacheEntry {
    stored_at: Instant,
    main_window: usize,
    targets: Vec<TrackBarTarget>,
}

type TargetCache = Mutex<Option<TargetCacheEntry>>;
static TARGET_CACHE: OnceLock<TargetCache> = OnceLock::new();

const CACHE_MAX_AGE: Duration = Duration::from_secs(8);

const CLASS_EFFECT_ITEM_CONTROL: &str = "EffectItemNumberControl@Component@@";
const CLASS_SLIDER_INPUT: &str = "SliderInputControl@Control@@";
const CLASS_TRACKBAR: &str = "TrackBarControl@Control@@";
const CLASS_EFFECT_SETTING: &str = "EffectSetting@Component@@";
const CLASS_EFFECT_SETTING_SELECT: &str = "EffectSettingSelect@Component@@";

/// AviUtl2 2.1.x のクラスレイアウト。
///
/// ここだけがバージョン依存。vtable は RTTI から解決するので、この構造体に
/// アドレスを書く必要はない。オフセットが変わった場合は `debug_snapshot()` の
/// 出力を見ながらこの値を更新する。
#[derive(Debug, Clone, Copy)]
struct LayoutProfile {
    label: &'static str,
    /// EffectItemNumberControl::item (Effect::EffectItemNumber*)
    item_control_item: usize,
    /// EffectItemNumberControl のオブジェクトサイズ。
    /// 埋め込み SliderInputControl を動的に探す範囲として使う。
    item_control_size: usize,
    /// EffectItemNumberControl::setting (EffectSetting*)
    item_control_context: usize,
    /// EffectSetting::effect (shared_ptr<EffectBase> の先頭)
    setting_effect_base: usize,
    /// EffectBase::name (const wchar_t*)
    effect_base_name: usize,
    /// EffectItemNumber::name (std::wstring)
    item_name_wstring: usize,
    /// SliderInputControl::visible (bool)
    slider_visible: usize,
    /// SliderInputControl::rect (x, y, w, h)
    slider_rect: usize,
}

const LAYOUT_2_1_X: LayoutProfile = LayoutProfile {
    label: "AviUtl2 2.1.x",
    item_control_item: 0x278,
    item_control_size: 0x1B48,
    item_control_context: 0x0E8,
    setting_effect_base: 0x0F8,
    effect_base_name: 0x40,
    item_name_wstring: 0x10,
    slider_visible: 0x0C,
    slider_rect: 0x50,
};

fn layout_profile(version: u32) -> Option<&'static LayoutProfile> {
    // 動作確認済みの範囲だけを許可する。未知のバージョンでは
    // 誤ったレイアウトで別の行を指すより、D&D を無効化する。
    // 2.1.15 は 2.1.14 と同じ UI レイアウト。クラス名と RTTI は
    // 2.1.15 のバイナリでも確認済み。将来レイアウトが変わった場合は
    // この範囲を広げず、新しい LayoutProfile を追加する。
    if (2_010_300..=2_011_500).contains(&version) {
        Some(&LAYOUT_2_1_X)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RectI {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

impl RectI {
    fn contains(self, x: i32, y: i32) -> bool {
        self.w > 0
            && self.h > 0
            && x >= self.x
            && x < self.x.saturating_add(self.w)
            && y >= self.y
            && y < self.y.saturating_add(self.h)
    }

    fn area(self) -> i64 {
        i64::from(self.w.max(0)) * i64::from(self.h.max(0))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TrackBarTarget {
    pub control: usize,
    pub trackbar: usize,
    pub context: usize,
    pub context_vtable: usize,
    pub slider_offset: usize,
    pub rect: RectI,
    pub visible: bool,
    pub item_name: String,
    pub label: String,
    pub effect_name: Option<String>,
    /// 同名エフェクトが複数ある場合の、画面上の並び順。
    pub effect_rank: usize,
}

/// ドラッグ中のドロップ先を囲む、クリック透過の枠ウィンドウ。
struct HighlightOverlay {
    hwnd: HWND,
}

static OVERLAY_CLASS: OnceLock<Vec<u16>> = OnceLock::new();

fn overlay_class() -> &'static [u16] {
    OVERLAY_CLASS.get_or_init(|| {
        let class_name = "HL_EzingTrackbarHighlight\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if hinstance.is_null() {
            return class_name;
        }
        let brush = unsafe { CreateSolidBrush(0x00F8BD38) }; // RGB(56, 189, 248)
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: brush,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        unsafe {
            RegisterClassExW(&class);
        }
        class_name
    })
}

impl HighlightOverlay {
    fn new() -> Option<Self> {
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if hinstance.is_null() {
            return None;
        }
        let class_name = overlay_class();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            SetLayeredWindowAttributes(hwnd, 0, 210, LWA_ALPHA);
        }
        Some(Self { hwnd })
    }

    fn show(&self, screen_x: i32, screen_y: i32, width: i32, height: i32) {
        if width < 8 || height < 8 {
            self.hide();
            return;
        }
        let border = 3;
        let outer = unsafe { CreateRectRgn(0, 0, width, height) };
        if outer.is_null() {
            return;
        }
        let inner = unsafe { CreateRectRgn(border, border, width - border, height - border) };
        if !inner.is_null() {
            unsafe {
                CombineRgn(outer, outer, inner, RGN_DIFF);
                DeleteObject(inner as _);
            }
        }
        let region_set = unsafe { SetWindowRgn(self.hwnd, outer, 1) };
        if region_set == 0 {
            unsafe {
                DeleteObject(outer as _);
            }
            return;
        }
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                screen_x,
                screen_y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for HighlightOverlay {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

#[derive(Debug, Clone)]
struct Vtables {
    effect_item_control: usize,
    slider_input: usize,
    trackbar: Option<usize>,
    effect_setting: Option<usize>,
    effect_setting_select: Option<usize>,
}

#[derive(Debug, Clone)]
struct Section {
    start: usize,
    end: usize,
    executable: bool,
}

#[derive(Debug, Clone)]
struct ModuleInfo {
    base: usize,
    image_size: usize,
    sections: Vec<Section>,
}

pub fn set_version(version: u32) {
    VERSION.store(version, Ordering::Release);
}

fn version() -> u32 {
    VERSION.load(Ordering::Acquire)
}

/// この AviUtl2 でドラッグ適用を試せるか。
pub fn is_supported() -> bool {
    layout_profile(version()).is_some()
        && vtables().is_some_and(|vtables| vtables.effect_setting.is_some())
}

fn vtables() -> Option<&'static Vtables> {
    VTABLES.get_or_init(resolve_vtables).as_ref()
}

// ---------------------------------------------------------------------------
// メモリ読み取り
// ---------------------------------------------------------------------------

fn read_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    address.checked_add(len)?;
    let mut buffer = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const core::ffi::c_void,
            buffer.as_mut_ptr().cast(),
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(buffer)
}

fn read_u8(address: usize) -> Option<u8> {
    read_bytes(address, 1).map(|b| b[0])
}

fn read_u16(address: usize) -> Option<u16> {
    read_bytes(address, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(address: usize) -> Option<u32> {
    read_bytes(address, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|v| v as i32)
}

fn read_usize(address: usize) -> Option<usize> {
    read_bytes(address, std::mem::size_of::<usize>())
        .and_then(|b| b.try_into().ok())
        .map(usize::from_le_bytes)
}

fn read_wide_string(address: usize, max_units: usize) -> Option<String> {
    if address < 0x10000 || max_units == 0 {
        return None;
    }
    let mut units = Vec::with_capacity(max_units.min(128));
    for index in 0..max_units {
        let unit = read_u16(address + index * 2)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
        if units.len() >= max_units {
            break;
        }
    }
    if units.is_empty() {
        return None;
    }
    let text = String::from_utf16_lossy(&units);
    (!text.trim().is_empty() && !text.chars().any(char::is_control)).then_some(text)
}

fn read_std_wstring(address: usize, offset: usize) -> Option<String> {
    // MSVC x64 の std::wstring:
    //   +0x00 バッファ (SSO時はここに文字列そのもの)
    //   +0x10 長さ
    //   +0x18 容量
    let base = address.checked_add(offset)?;
    let length = read_usize(base.checked_add(0x10)?)?;
    let capacity = read_usize(base.checked_add(0x18)?)?;
    if length == 0 || length > 512 {
        return None;
    }
    let buffer = if capacity >= 8 {
        read_usize(base)?
    } else {
        base
    };
    let mut bytes = vec![0u8; length * 2];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            buffer as *const core::ffi::c_void,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut read,
        )
    };
    if ok == 0 || read != bytes.len() {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let text = String::from_utf16_lossy(&units);
    (!text.trim().is_empty() && !text.chars().any(char::is_control)).then_some(text)
}

// ---------------------------------------------------------------------------
// PE / RTTI
// ---------------------------------------------------------------------------

fn module_info() -> Option<ModuleInfo> {
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    if base == 0 {
        return None;
    }
    let e_lfanew = read_u32(base + 0x3C)? as usize;
    let nt = base.checked_add(e_lfanew)?;
    if read_u32(nt)? != 0x0000_4550 {
        return None;
    }
    let number_of_sections = read_u16(nt + 6)? as usize;
    let size_of_optional_header = read_u16(nt + 20)? as usize;
    let optional = nt + 24;
    if read_u16(optional)? != 0x020B {
        // AviUtl2 は x64 なので PE32+ のみ対応。
        return None;
    }
    let image_size = read_u32(optional + 56)? as usize;
    let sections_start = optional + size_of_optional_header;
    let mut sections = Vec::with_capacity(number_of_sections);
    for index in 0..number_of_sections {
        let header = sections_start + index * 40;
        let virtual_size = read_u32(header + 8)? as usize;
        let virtual_address = read_u32(header + 12)? as usize;
        let raw_size = read_u32(header + 16)? as usize;
        let characteristics = read_u32(header + 36)?;
        let start = base.checked_add(virtual_address)?;
        // 破損 PE の巨大なセクション長で Vec を確保しないよう上限を設ける。
        let len = virtual_size
            .max(raw_size)
            .min(image_size)
            .min(64 * 1024 * 1024);
        if len == 0 {
            continue;
        }
        sections.push(Section {
            start,
            end: start.checked_add(len)?,
            executable: characteristics & 0x2000_0000 != 0,
        });
    }
    Some(ModuleInfo {
        base,
        image_size,
        sections,
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 型名から TypeDescriptor を探す。RTTI の型名は
/// `. ? AV<クラス名>` の形式で `.rdata` に格納されている。
fn find_type_descriptor(module: &ModuleInfo, class_name: &str) -> Option<usize> {
    let needle = format!(".?AV{class_name}").into_bytes();
    for section in &module.sections {
        if section.executable {
            continue;
        }
        let len = section.end.saturating_sub(section.start);
        let Some(data) = read_bytes(section.start, len) else {
            continue;
        };
        let mut search = 0usize;
        while let Some(position) = find_subslice(&data[search..], &needle) {
            let absolute = section.start + search + position;
            if absolute >= 16 {
                let type_descriptor = absolute - 16;
                if find_class_hierarchy_descriptor(module, type_descriptor).is_some() {
                    return Some(type_descriptor);
                }
            }
            search += position + 1;
        }
    }
    None
}

/// TypeDescriptor を参照している CompleteObjectLocator (COL) を探す。
///
/// MSVC の RTTI:
///   COL + 0x00 signature (=1)
///   COL + 0x04 offset
///   COL + 0x08 cdOffset
///   COL + 0x0C pTypeDescriptor (RVA)
///   COL + 0x10 pClassDescriptor (ClassHierarchyDescriptor の RVA)
///   COL + 0x14 pSelf (RVA)
fn find_class_hierarchy_descriptor(module: &ModuleInfo, type_descriptor: usize) -> Option<usize> {
    let td_rva = u32::try_from(type_descriptor.checked_sub(module.base)?).ok()?;
    for section in &module.sections {
        if section.executable {
            continue;
        }
        let len = section.end.saturating_sub(section.start);
        let Some(data) = read_bytes(section.start, len) else {
            continue;
        };
        for offset in (12..len.saturating_sub(20)).step_by(4) {
            if u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) != td_rva
            {
                continue;
            }
            let col = section.start + offset - 12;
            let signature = read_u32(col).unwrap_or(0);
            let p_type_descriptor = read_u32(col + 12).unwrap_or(0);
            let p_class_descriptor = read_u32(col + 16).unwrap_or(u32::MAX) as usize;
            if signature != 1
                || p_type_descriptor != td_rva
                || p_class_descriptor >= module.image_size
            {
                continue;
            }
            // ClassHierarchyDescriptor
            let chd = module.base + p_class_descriptor;
            let number_of_base_classes = read_u32(chd + 8).unwrap_or(0);
            let base_class_array = read_u32(chd + 12).unwrap_or(u32::MAX) as usize;
            if number_of_base_classes > 0 && base_class_array < module.image_size {
                return Some(col);
            }
        }
    }
    None
}

fn is_executable_address(module: &ModuleInfo, address: usize) -> bool {
    module
        .sections
        .iter()
        .any(|section| section.executable && address >= section.start && address < section.end)
}

/// COL への参照を探し、その直後を vtable とみなす。
fn find_vtable(module: &ModuleInfo, col: usize) -> Option<usize> {
    let col_rva = col.checked_sub(module.base)?;
    let expected = module.base.checked_add(col_rva)?;
    for section in &module.sections {
        if section.executable {
            continue;
        }
        let len = section.end.saturating_sub(section.start);
        let Some(data) = read_bytes(section.start, len) else {
            continue;
        };
        for offset in (0..len.saturating_sub(8)).step_by(8) {
            let location = section.start + offset;
            if !location.is_multiple_of(8) {
                continue;
            }
            let value = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;
            if value != expected {
                continue;
            }
            let vtable = location + 8;
            if let Some(first_method) = read_usize(vtable)
                && is_executable_address(module, first_method)
            {
                return Some(vtable);
            }
        }
    }
    None
}

fn resolve_class_vtable(module: &ModuleInfo, class_name: &str) -> Option<usize> {
    let type_descriptor = find_type_descriptor(module, class_name)?;
    let col = find_class_hierarchy_descriptor(module, type_descriptor)?;
    find_vtable(module, col)
}

fn resolve_vtables() -> Option<Vtables> {
    let module = module_info().or_else(|| {
        tracing::warn!("HL_Ezing trackbar RTTI: could not read the AviUtl2 PE headers");
        None
    })?;
    let effect_item_control =
        resolve_class_vtable(&module, CLASS_EFFECT_ITEM_CONTROL).or_else(|| {
            tracing::warn!("HL_Ezing trackbar RTTI: failed to resolve {CLASS_EFFECT_ITEM_CONTROL}");
            None
        })?;
    let slider_input = resolve_class_vtable(&module, CLASS_SLIDER_INPUT).or_else(|| {
        tracing::warn!("HL_Ezing trackbar RTTI: failed to resolve {CLASS_SLIDER_INPUT}");
        None
    })?;
    let trackbar = resolve_class_vtable(&module, CLASS_TRACKBAR);
    let effect_setting = resolve_class_vtable(&module, CLASS_EFFECT_SETTING);
    let effect_setting_select = resolve_class_vtable(&module, CLASS_EFFECT_SETTING_SELECT);
    tracing::info!(
        "HL_Ezing trackbar RTTI: base=0x{:X}, EffectItemNumberControl=0x{:X}, SliderInputControl=0x{:X}, TrackBarControl={:?}, EffectSetting={:?}, EffectSettingSelect={:?}",
        module.base,
        effect_item_control,
        slider_input,
        trackbar.map(|v| format!("0x{v:X}")),
        effect_setting.map(|v| format!("0x{v:X}")),
        effect_setting_select.map(|v| format!("0x{v:X}")),
    );
    Some(Vtables {
        effect_item_control,
        slider_input,
        trackbar,
        effect_setting,
        effect_setting_select,
    })
}

// ---------------------------------------------------------------------------
// オブジェクト走査
// ---------------------------------------------------------------------------

fn readable_private_regions() -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut address = 0x1_0000usize;
    loop {
        let mut info = unsafe { std::mem::zeroed::<MEMORY_BASIC_INFORMATION>() };
        let result = unsafe {
            VirtualQuery(
                address as *const core::ffi::c_void,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if result == 0 {
            break;
        }
        let base = info.BaseAddress as usize;
        let size = info.RegionSize;
        if size == 0 {
            break;
        }
        let protect = info.Protect & 0xFF;
        let readable = matches!(
            protect,
            PAGE_READONLY
                | PAGE_READWRITE
                | PAGE_WRITECOPY
                | PAGE_EXECUTE_READ
                | PAGE_EXECUTE_READWRITE
                | PAGE_EXECUTE_WRITECOPY
        ) && info.Protect & PAGE_GUARD == 0
            && info.Protect & PAGE_NOACCESS == 0;
        if info.State == MEM_COMMIT && info.Type == MEM_PRIVATE && readable {
            regions.push((base, size));
        }
        let Some(next) = base.checked_add(size) else {
            break;
        };
        if next <= address {
            break;
        }
        address = next;
    }
    regions
}

fn find_setting_base(context: usize, setting_vtable: usize) -> Option<usize> {
    // 2.1.14 では +0xE8 は EffectSetting 本体の先頭ではなく、
    // その内部コントロールを指す。vtable を後方走査して本体を探す。
    for offset in (0..=0x400).step_by(8) {
        let address = context.checked_sub(offset)?;
        if address < 0x10000 {
            break;
        }
        if read_usize(address) == Some(setting_vtable) {
            return Some(address);
        }
    }
    None
}

fn read_effect_name(target: &TrackBarTarget, layout: &LayoutProfile) -> Option<String> {
    if target.context < 0x10000 {
        return None;
    }
    let vtables = vtables()?;
    let setting_vtable = vtables.effect_setting?;
    let setting_base = find_setting_base(target.context, setting_vtable)?;
    let effect_base = read_usize(setting_base.checked_add(layout.setting_effect_base)?)?;
    if effect_base < 0x10000 {
        return None;
    }
    let name = read_usize(effect_base.checked_add(layout.effect_base_name)?)?;
    read_wide_string(name, 96)
}

fn validate_candidate(
    address: usize,
    layout: &LayoutProfile,
    vtables: &Vtables,
) -> Vec<TrackBarTarget> {
    if !address.is_multiple_of(8) {
        return Vec::new();
    }
    let Some(item_address) = address.checked_add(layout.item_control_item) else {
        return Vec::new();
    };
    let Some(item) = read_usize(item_address) else {
        return Vec::new();
    };
    if item < 0x10000 {
        return Vec::new();
    }
    let Some(item_name) = read_std_wstring(item, layout.item_name_wstring) else {
        return Vec::new();
    };
    let context = address
        .checked_add(layout.item_control_context)
        .and_then(read_usize)
        .unwrap_or(0);
    let context_vtable = if context >= 0x10000 {
        read_usize(context).unwrap_or(0)
    } else {
        0
    };
    // EffectItemNumberControl の内部には SliderInputControl が埋め込まれている。
    // そのオフセットはバージョンによって変わる可能性があるため、固定値ではなく
    // オブジェクト内の vtable ポインタを走査して動的に見つける。
    let mut targets = Vec::new();
    let scan_start = 0x200;
    let scan_end = layout.item_control_size.saturating_sub(8);
    if scan_end <= scan_start {
        return targets;
    }
    let Some(object_bytes) = read_bytes(address + scan_start, scan_end - scan_start) else {
        return targets;
    };
    for offset in (0..=object_bytes.len().saturating_sub(8)).step_by(8) {
        let value = u64::from_le_bytes([
            object_bytes[offset],
            object_bytes[offset + 1],
            object_bytes[offset + 2],
            object_bytes[offset + 3],
            object_bytes[offset + 4],
            object_bytes[offset + 5],
            object_bytes[offset + 6],
            object_bytes[offset + 7],
        ]) as usize;
        if value != vtables.slider_input {
            continue;
        }
        let slider_offset = scan_start + offset;
        let Some(slider) = address.checked_add(slider_offset) else {
            continue;
        };
        let Some(rect_addr) = slider.checked_add(layout.slider_rect) else {
            continue;
        };
        let Some(x) = read_i32(rect_addr) else {
            continue;
        };
        let Some(y) = read_i32(rect_addr + 4) else {
            continue;
        };
        let Some(w) = read_i32(rect_addr + 8) else {
            continue;
        };
        let Some(h) = read_i32(rect_addr + 12) else {
            continue;
        };
        let rect = RectI { x, y, w, h };
        if rect.w <= 0 || rect.h <= 0 || rect.w > 20_000 || rect.h > 20_000 {
            continue;
        }
        let visible = read_u8(slider + layout.slider_visible).unwrap_or(1) != 0;
        let mut target = TrackBarTarget {
            control: address,
            trackbar: slider,
            context,
            context_vtable,
            slider_offset,
            rect,
            visible,
            item_name: item_name.clone(),
            label: item_name.clone(),
            effect_name: None,
            effect_rank: 0,
        };
        target.effect_name = read_effect_name(&target, layout);
        targets.push(target);
    }
    targets
}

fn assign_effect_ranks(targets: &mut [TrackBarTarget]) {
    // 同じ EffectSetting インスタンスに属するコントロールをグループ化し、
    // 画面上の Y 座標順で同名エフェクトの何番目かを求める。
    let mut groups: HashMap<usize, (i32, Option<String>)> = HashMap::new();
    for target in targets.iter() {
        let key = if target.context != 0 {
            target.context
        } else {
            target.control
        };
        let entry = groups
            .entry(key)
            .or_insert((target.rect.y, target.effect_name.clone()));
        entry.0 = entry.0.min(target.rect.y);
        if entry.1.is_none() {
            entry.1 = target.effect_name.clone();
        }
    }
    for target in targets.iter_mut() {
        let Some(effect_name) = target.effect_name.as_ref() else {
            continue;
        };
        let key = if target.context != 0 {
            target.context
        } else {
            target.control
        };
        let Some((own_y, _)) = groups.get(&key) else {
            continue;
        };
        let rank = groups
            .values()
            .filter(|(y, name)| name.as_ref() == Some(effect_name) && y < own_y)
            .count();
        target.effect_rank = rank;
    }
}

fn scan_targets() -> Result<Vec<TrackBarTarget>, String> {
    let layout = layout_profile(version())
        .ok_or_else(|| format!("AviUtl2 のバージョン {} は未対応です。", version()))?;
    let vtables = vtables().ok_or_else(|| {
        "AviUtl2 の RTTI からトラックバーの vtable を解決できませんでした。".to_string()
    })?;

    let started = Instant::now();
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut raw_effect_item_control = 0usize;
    let mut raw_slider_input = 0usize;
    for (region_base, region_size) in readable_private_regions() {
        let region_end = region_base.saturating_add(region_size);
        let mut address = region_base;
        while address < region_end {
            let len = (region_end - address).min(1024 * 1024);
            let Some(buffer) = read_bytes(address, len) else {
                address = address.saturating_add(len);
                continue;
            };
            for (index, chunk) in buffer.chunks_exact(8).enumerate() {
                let value = u64::from_le_bytes([
                    chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
                ]) as usize;
                if value == vtables.slider_input {
                    raw_slider_input += 1;
                }
                if value != vtables.effect_item_control {
                    continue;
                }
                raw_effect_item_control += 1;
                let candidate = address + index * 8;
                if !seen.insert(candidate) {
                    continue;
                }
                targets.extend(validate_candidate(candidate, layout, vtables));
            }
            address = address.saturating_add(len);
        }
    }
    targets.sort_by_key(|target| (target.rect.y, target.rect.x));
    assign_effect_ranks(&mut targets);
    tracing::debug!(
        "HL_Ezing trackbar scan: {} controls (raw effect_item={}, slider={}) in {:?}",
        targets.len(),
        raw_effect_item_control,
        raw_slider_input,
        started.elapsed()
    );
    Ok(targets)
}

// ---------------------------------------------------------------------------
// 座標判定
// ---------------------------------------------------------------------------

fn left_button_down() -> bool {
    (unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16 & 0x8000) != 0
}

fn main_window() -> Option<HWND> {
    let host = crate::window::host_hwnd()?;
    let root = unsafe { GetAncestor(host, GA_ROOT) };
    (!root.is_null()).then_some(root)
}

fn root_window_at(point: POINT) -> Option<HWND> {
    let hwnd = unsafe { WindowFromPoint(point) };
    if hwnd.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.is_null() {
        return None;
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(root, &mut process_id) };
    (process_id == unsafe { GetCurrentProcessId() }).then_some(root)
}

fn is_over_plugin_window(point: POINT) -> bool {
    let Some(host) = crate::window::host_hwnd() else {
        return false;
    };
    let hwnd = unsafe { WindowFromPoint(point) };
    if hwnd.is_null() {
        return false;
    }
    hwnd == host || unsafe { IsChild(host, hwnd) != 0 }
}

fn cache() -> &'static TargetCache {
    TARGET_CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_targets(main: HWND) -> Option<Vec<TrackBarTarget>> {
    let guard = cache().lock().ok()?;
    let entry = guard.as_ref()?;
    (entry.stored_at.elapsed() < CACHE_MAX_AGE && entry.main_window == main as usize)
        .then(|| entry.targets.clone())
}

fn store_targets(main: HWND, targets: &[TrackBarTarget]) {
    // 空の結果をキャッシュすると、設定画面を開いた後の初回ドラッグが
    // 8秒間失敗するため保存しない。
    if targets.is_empty() {
        return;
    }
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(TargetCacheEntry {
            stored_at: Instant::now(),
            main_window: main as usize,
            targets: targets.to_vec(),
        });
    }
}

pub fn invalidate_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

static SCAN_LOCK: Mutex<()> = Mutex::new(());

fn scan_and_store() -> Result<Vec<TrackBarTarget>, String> {
    let main =
        main_window().ok_or_else(|| "AviUtl2 のメインウィンドウを特定できません。".to_string())?;
    let _lock = SCAN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(targets) = cached_targets(main) {
        return Ok(targets);
    }
    let targets = scan_targets()?;
    store_targets(main, &targets);
    Ok(targets)
}

/// ドラッグ開始前にバックグラウンドでスキャンを温める。
pub fn warm_cache() {
    if main_window().and_then(cached_targets).is_some() {
        return;
    }
    std::thread::spawn(|| {
        let _ = scan_and_store();
    });
}

fn hit_test_targets(
    targets: &[TrackBarTarget],
    root: HWND,
    screen: POINT,
) -> Option<(usize, HWND)> {
    // 対象矩形は AviUtl2 メインウィンドウのクライアント座標なので、
    // ドロップ先のルートウィンドウからスクリーン座標へ変換する。
    // これによりプラグインウィンドウが分離（フローティング）していても、
    // メインウィンドウ上のトラックバーを正しく判定できる。
    let any_visible = targets.iter().any(|target| target.visible);
    let mut best: Option<(i64, usize)> = None;
    for (index, target) in targets.iter().enumerate() {
        if any_visible && !target.visible {
            continue;
        }
        let Some(rect) = screen_rect(root, target.rect) else {
            continue;
        };
        if rect.contains(screen.x, screen.y) {
            let area = rect.area();
            if best.as_ref().is_none_or(|(best_area, _)| area < *best_area) {
                best = Some((area, index));
            }
        }
    }
    best.map(|(_, index)| (index, root))
}

fn screen_rect(root: HWND, rect: RectI) -> Option<RectI> {
    let mut origin = POINT { x: 0, y: 0 };
    (unsafe { ClientToScreen(root, &mut origin) } != 0).then_some(RectI {
        x: origin.x.checked_add(rect.x)?,
        y: origin.y.checked_add(rect.y)?,
        w: rect.w,
        h: rect.h,
    })
}

// ---------------------------------------------------------------------------
// 適用
// ---------------------------------------------------------------------------

fn item_matches(a: &str, b: &str) -> bool {
    a == b || a.rsplit("::").next() == Some(b) || b.rsplit("::").next() == Some(a)
}

fn resolve_descriptor(
    tracks: &[crate::curve::apply::TrackEntry],
    target: &TrackBarTarget,
) -> Option<crate::curve::apply::TrackEntry> {
    let item_candidates = tracks
        .iter()
        .filter(|track| item_matches(&track.item, &target.item_name))
        .cloned()
        .collect::<Vec<_>>();

    let mut candidates = if let Some(effect_name) = target.effect_name.as_ref() {
        item_candidates
            .iter()
            .filter(|track| &track.effect == effect_name)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if candidates.is_empty() {
        // エフェクト名が取れない場合だけ、item 名が一意なときに限り許可する。
        // 同名 item が複数ある状況で適当なエフェクトへ適用しない。
        if item_candidates.len() == 1 {
            return item_candidates.into_iter().next();
        }
        return None;
    }
    if candidates.len() == 1 {
        return candidates.pop();
    }
    if target.effect_rank >= candidates.len() {
        return None;
    }
    candidates.into_iter().nth(target.effect_rank)
}

pub fn apply_target(
    target: &TrackBarTarget,
    curve_id: u64,
    group_apply: bool,
) -> Result<(), String> {
    if !crate::ipc::EDIT_HANDLE.is_ready() {
        return Err("編集セクションの初期化が完了していません。".into());
    }
    let result = crate::ipc::EDIT_HANDLE.call_edit_section(|edit| -> Result<(), String> {
        let mut objects = Vec::new();
        if let Ok(Some(object)) = edit.get_focused_object() {
            objects.push(object);
        }
        if let Ok(selected) = edit.get_selected_objects() {
            objects.extend(selected);
        }
        if objects.is_empty()
            && let Some(object) = crate::curve::apply::focused_or_first_object(edit)?
        {
            objects.push(object);
        }
        if objects.is_empty() {
            return Err("適用対象のオブジェクトが見つかりません。".to_string());
        }

        for object in objects {
            let Ok(tracks) = crate::curve::apply::collect_tracks(edit, object) else {
                continue;
            };
            tracing::info!(
                "HL_Ezing drag resolve: effect={:?}, item={}, object={:?}, item candidates={:?}, effects={:?}",
                target.effect_name,
                target.item_name,
                edit.get_object_layer_frame(object).ok(),
                tracks
                    .iter()
                    .filter(|track| item_matches(&track.item, &target.item_name))
                    .map(|track| format!("{}/{}#{}", track.effect, track.item, track.effect_index))
                    .take(20)
                    .collect::<Vec<_>>(),
                {
                    let mut names =
                        tracks.iter().map(|track| track.effect.clone()).collect::<Vec<_>>();
                    names.sort();
                    names.dedup();
                    names.into_iter().take(30).collect::<Vec<_>>()
                }
            );
            if let Some(descriptor) = resolve_descriptor(&tracks, target) {
                return crate::curve::apply::apply_to_item_on_object(
                    edit,
                    object,
                    &descriptor.effect,
                    descriptor.effect_index,
                    &descriptor.item,
                    curve_id,
                    group_apply,
                );
            }
        }
        Err("ドロップ位置のトラックバーを特定できませんでした。".to_string())
    });
    match result {
        Ok(inner) => inner,
        Err(error) => Err(error.to_string()),
    }
}

/// ドラッグ開始を受け付け、左ボタンが離された位置のトラックバーへ適用する。
/// 実際のスキャンと適用はワーカースレッドで行うため、WebView の IPC を
/// ブロックしない。
pub fn start_drag(curve_id: u64, group_apply: bool) -> bool {
    if layout_profile(version()).is_none() || vtables().is_none() {
        tracing::warn!("HL_Ezing drag: unsupported AviUtl2 version or unresolved RTTI");
        return false;
    }
    if DRAG_ACTIVE.swap(true, Ordering::AcqRel) {
        tracing::debug!("HL_Ezing drag: already active");
        return false;
    }
    warm_cache();
    std::thread::spawn(move || {
        let _guard = DragGuard;
        let result = std::panic::catch_unwind(|| monitor_drag(curve_id, group_apply))
            .unwrap_or_else(|_| Err("ドラッグ処理中にエラーが発生しました。".to_string()));
        if let Err(message) = result {
            tracing::warn!("HL_Ezing drag cancelled: {message}");
            crate::ipc::send_event(json!({
                "event": "drag_cancelled",
                "ok": false,
                "message": message,
            }));
        }
    });
    true
}

struct DragGuard;

impl Drop for DragGuard {
    fn drop(&mut self) {
        DRAG_ACTIVE.store(false, Ordering::Release);
    }
}

fn monitor_drag(curve_id: u64, group_apply: bool) -> Result<(), String> {
    if !crate::curve::registry::contains(curve_id) {
        return Err("指定されたカーブIDが存在しません。".into());
    }
    let main = main_window().ok_or_else(|| "AviUtl2 のウィンドウを特定できません。".to_string())?;
    let started = Instant::now();
    // フロントエンドは数ピクセル動かしてから start_drag を送る。通常は
    // すでに左ボタンが押されているが、念のため短時間だけ待つ。
    while started.elapsed() < Duration::from_millis(300) && !left_button_down() {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !left_button_down() {
        return Err("ドラッグが開始されませんでした。".into());
    }

    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err("カーソル位置を取得できませんでした。".into());
    }
    let mut release_point = point;

    // スキャンは prepare_drag で開始済み。ここではスキャンを待たずに
    // マウス位置を記録し、ボタンが離れた瞬間の位置を保持する。
    let overlay = HighlightOverlay::new();
    let mut targets = cached_targets(main);
    while started.elapsed() < Duration::from_secs(60) {
        if unsafe { GetCursorPos(&mut point) } == 0 {
            break;
        }
        if !left_button_down() {
            break;
        }
        release_point = point;
        if targets.is_none() {
            targets = cached_targets(main);
        }
        if let Some(targets) = targets.as_ref() {
            if !is_over_plugin_window(point)
                && let Some(root) = root_window_at(point)
                && let Some(hit) = hit_test_targets(targets, root, point)
            {
                if let Some(overlay) = overlay.as_ref()
                    && let Some(rect) = screen_rect(root, targets[hit.0].rect)
                {
                    overlay.show(rect.x, rect.y, rect.w, rect.h);
                }
            } else if let Some(overlay) = overlay.as_ref() {
                overlay.hide();
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if left_button_down() {
        return Err("ドラッグがタイムアウトしました。".into());
    }
    if let Some(overlay) = overlay.as_ref() {
        overlay.hide();
    }

    let targets = match targets.or_else(|| cached_targets(main)) {
        Some(targets) => targets,
        None => scan_and_store()?,
    };
    if is_over_plugin_window(release_point) {
        return Err("HL_Ezing ウィンドウ上では適用できません。".to_string());
    }
    let hit = match root_window_at(release_point)
        .and_then(|root| hit_test_targets(&targets, root, release_point))
    {
        Some(hit) => hit,
        None => {
            tracing::info!(
                "HL_Ezing drag release miss: cursor=({}, {}), main=0x{:X}, targets={}",
                release_point.x,
                release_point.y,
                main as usize,
                targets.len()
            );
            return Err("カーソル下にトラックバーがありません。".to_string());
        }
    };
    let target = &targets[hit.0];
    apply_target(target, curve_id, group_apply)?;
    crate::curve::registry::set_last_applied(curve_id);
    let curve = crate::curve::registry::get(curve_id);
    crate::ipc::send_event(json!({
        "event": "applied",
        "ok": true,
        "count": 1,
        "id": curve_id,
        "created": false,
        "curve": curve.as_ref().map(|curve| crate::ipc::curve_to_json(curve_id, curve)),
        "target": {
            "effect": target.effect_name,
            "item": target.item_name,
            "control": format!("0x{:X}", target.control),
        },
    }));
    Ok(())
}

// ---------------------------------------------------------------------------
// 診断
// ---------------------------------------------------------------------------

pub fn debug_snapshot() -> serde_json::Value {
    let version = version();
    let module = module_info();
    let layout = layout_profile(version);
    let vtables = vtables();
    let targets = scan_targets();
    let host = crate::window::host_hwnd();
    let root = host.and_then(|hwnd| {
        let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
        (!root.is_null()).then_some(root)
    });
    json!({
        "version": version,
        "profile": layout.map(|layout| layout.label),
        "module_base": module.as_ref().map(|module| format!("0x{:X}", module.base)),
        "image_size": module.as_ref().map(|module| format!("0x{:X}", module.image_size)),
        "vtables": vtables.map(|vtables| json!({
            "effect_item_control": format!("0x{:X}", vtables.effect_item_control),
            "slider_input": format!("0x{:X}", vtables.slider_input),
            "trackbar": vtables.trackbar.map(|value| format!("0x{value:X}")),
            "effect_setting": vtables.effect_setting.map(|value| format!("0x{value:X}")),
            "effect_setting_select": vtables.effect_setting_select.map(|value| format!("0x{value:X}")),
        })),
        "layout": layout.map(|layout| json!({
            "item": format!("0x{:X}", layout.item_control_item),
            "size": format!("0x{:X}", layout.item_control_size),
            "context": format!("0x{:X}", layout.item_control_context),
            "effect_base": format!("0x{:X}", layout.setting_effect_base),
            "effect_name": format!("0x{:X}", layout.effect_base_name),
            "item_name": format!("0x{:X}", layout.item_name_wstring),
            "visible": format!("0x{:X}", layout.slider_visible),
            "rect": format!("0x{:X}", layout.slider_rect),
        })),
        "host": host.map(|hwnd| format!("0x{:X}", hwnd as usize)),
        "root": root.map(|hwnd| format!("0x{:X}", hwnd as usize)),
        "targets": match targets {
            Ok(targets) => json!(targets.into_iter().map(|target| json!({
                "control": format!("0x{:X}", target.control),
                "trackbar": format!("0x{:X}", target.trackbar),
                "slider_offset": format!("0x{:X}", target.slider_offset),
                "context": format!("0x{:X}", target.context),
                "context_vtable": format!("0x{:X}", target.context_vtable),
                "rect": { "x": target.rect.x, "y": target.rect.y, "w": target.rect.w, "h": target.rect.h },
                "visible": target.visible,
                "item": target.item_name,
                "label": target.label,
                "effect": target.effect_name,
                "effect_rank": target.effect_rank,
            })).collect::<Vec<_>>()),
            Err(error) => json!({ "error": error }),
        },
    })
}

pub fn log_debug_snapshot() {
    // 数秒かかるメモリ走査で UI スレッドを止めないよう、ワーカーで実行する。
    std::thread::spawn(|| {
        let snapshot = debug_snapshot();
        match serde_json::to_string_pretty(&snapshot) {
            Ok(text) => tracing::info!("HL_Ezing trackbar diagnostics:\n{text}"),
            Err(error) => tracing::warn!("Failed to serialize trackbar diagnostics: {error}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_edges() {
        let rect = RectI {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        assert!(rect.contains(10, 20));
        assert!(rect.contains(39, 59));
        assert!(!rect.contains(40, 59));
        assert!(!rect.contains(10, 60));
    }

    #[test]
    fn item_matching_handles_group_prefix() {
        assert!(item_matches("X", "X"));
        assert!(item_matches("group::X", "X"));
        assert!(item_matches("X", "group::X"));
        assert!(!item_matches("X", "Y"));
    }
}
