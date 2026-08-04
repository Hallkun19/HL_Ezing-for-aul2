//! カーブレジストリ。
//!
//! GUI(適用ボタン)・スクリプトモジュール(.tra2 → getcurve)・プロジェクト保存の間で
//! 共有する。IDとカーブデータを対応付けて保持する。スレッド安全。

use std::sync::{OnceLock, RwLock};

use super::Curve;

#[derive(Default)]
pub struct CurveRegistry {
    curves: std::collections::HashMap<u64, Curve>,
    next_id: u64,
}

impl CurveRegistry {
    /// カーブを登録し、IDを返す。IDは1から始まる(0は「無し」扱い)。
    pub fn register(&mut self, curve: Curve) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.curves.insert(id, curve);
        id
    }

    pub fn get(&self, id: u64) -> Option<Curve> {
        self.curves.get(&id).cloned()
    }

    pub fn snapshot(&self) -> Vec<(u64, Curve)> {
        self.curves.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// プロジェクトロード時に全カーブを差し替える。
    pub fn load(&mut self, entries: Vec<(u64, Curve)>) {
        self.curves.clear();
        self.next_id = 1;
        for (id, curve) in entries {
            self.curves.insert(id, curve);
            if id >= self.next_id {
                self.next_id = id + 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.curves.clear();
        self.next_id = 1;
    }
}

static REGISTRY: OnceLock<RwLock<CurveRegistry>> = OnceLock::new();

fn registry() -> &'static RwLock<CurveRegistry> {
    REGISTRY.get_or_init(|| RwLock::new(CurveRegistry::default()))
}

pub fn register(curve: Curve) -> u64 {
    registry().write().unwrap().register(curve)
}

pub fn get(id: u64) -> Option<Curve> {
    registry().read().unwrap().get(id)
}

/// 適用のたびに増えるnonce。トラックパラメータに埋め込み、
/// キャッシュ(スクリプト側)の無効化に使う。
pub fn nonce() -> u64 {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn snapshot() -> Vec<(u64, Curve)> {
    registry().read().unwrap().snapshot()
}

pub fn load(entries: Vec<(u64, Curve)>) {
    registry().write().unwrap().load(entries);
}

pub fn clear() {
    registry().write().unwrap().clear();
}

/// 最後に適用したカーブID(オブジェクト右クリックメニューからの再適用用)。
static LAST_APPLIED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_last_applied(id: u64) {
    LAST_APPLIED.store(id, std::sync::atomic::Ordering::Relaxed);
}

pub fn last_applied() -> Option<u64> {
    let id = LAST_APPLIED.load(std::sync::atomic::Ordering::Relaxed);
    (id != 0).then_some(id)
}
