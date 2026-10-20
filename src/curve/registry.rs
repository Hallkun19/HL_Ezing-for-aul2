//! カーブレジストリ。
//!
//! GUI(適用ボタン)・スクリプトモジュール(.tra2 → getcurve)・プロジェクト保存の間で
//! 共有する。IDとカーブデータを対応付けて保持する。スレッド安全。
//!
//! ID は 1 から始まる。0 は旧バージョンとの互換のために許容するが、
//! 新規登録では使用しない。カーブは `Arc` で保持し、毎フレームの評価時に
//! カーブ全体を clone しないようにしている。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use super::Curve;

/// 最後に適用したカーブが無いことを表す番兵値。
const NO_LAST_APPLIED: u64 = u64::MAX;

pub struct CurveRegistry {
    curves: HashMap<u64, Arc<Curve>>,
    next_id: u64,
    /// トラックバーごとの区間IDリスト。キーは負のIDとしてトラックに埋め込む。
    assignments: HashMap<u64, Vec<u64>>,
    next_assignment: u64,
}

impl Default for CurveRegistry {
    fn default() -> Self {
        Self {
            curves: HashMap::new(),
            // 0 は「カーブなし」として扱われるため、必ず 1 から始める。
            next_id: 1,
            assignments: HashMap::new(),
            next_assignment: 1,
        }
    }
}

impl CurveRegistry {
    /// カーブを登録し、IDを返す。IDは1から始まる。
    pub fn register(&mut self, curve: Curve) -> u64 {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.curves.insert(id, Arc::new(curve));
        id
    }

    /// 指定IDでカーブを登録する。既に存在する場合や 0 の場合は false。
    pub fn register_with_id(&mut self, id: u64, curve: Curve) -> bool {
        if id == 0 || self.curves.contains_key(&id) {
            return false;
        }
        self.curves.insert(id, Arc::new(curve));
        if id >= self.next_id {
            self.next_id = id.saturating_add(1);
        }
        true
    }

    /// 既存IDのカーブを差し替える。存在しない場合は false。
    pub fn update(&mut self, id: u64, curve: Curve) -> bool {
        if let Some(slot) = self.curves.get_mut(&id) {
            *slot = Arc::new(curve);
            true
        } else {
            false
        }
    }

    /// カーブを削除する。
    pub fn remove(&mut self, id: u64) -> Option<Arc<Curve>> {
        self.curves.remove(&id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.curves.contains_key(&id)
    }

    pub fn get(&self, id: u64) -> Option<Arc<Curve>> {
        self.curves.get(&id).cloned()
    }

    /// 区間IDリストを新しい割り当てとして登録する。
    pub fn create_assignment(&mut self, segment_ids: Vec<u64>) -> u64 {
        if self.next_assignment == 0 {
            self.next_assignment = 1;
        }
        let id = self.next_assignment;
        self.next_assignment = self.next_assignment.saturating_add(1);
        self.assignments.insert(id, segment_ids);
        id
    }

    /// 既存の割り当てを差し替える。
    pub fn set_assignment(&mut self, id: u64, segment_ids: Vec<u64>) -> bool {
        if let Some(slot) = self.assignments.get_mut(&id) {
            *slot = segment_ids;
            true
        } else {
            false
        }
    }

    pub fn assignment(&self, id: u64) -> Option<Vec<u64>> {
        self.assignments.get(&id).cloned()
    }

    pub fn assignment_curve(&self, id: u64, index: usize) -> Option<u64> {
        self.assignments
            .get(&id)
            .and_then(|ids| ids.get(index).copied())
    }

    pub fn assignment_snapshot(&self) -> Vec<(u64, Vec<u64>)> {
        let mut entries = self
            .assignments
            .iter()
            .map(|(id, ids)| (*id, ids.clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(id, _)| *id);
        entries
    }

    pub fn load_assignments(&mut self, entries: Vec<(u64, Vec<u64>)>) {
        self.assignments.clear();
        self.next_assignment = 1;
        for (id, ids) in entries {
            self.assignments.insert(id, ids);
            if id >= self.next_assignment {
                self.next_assignment = id.saturating_add(1);
            }
        }
        if self.next_assignment == 0 {
            self.next_assignment = 1;
        }
    }

    /// プロジェクト保存用。ID順に並べて返す。
    pub fn snapshot(&self) -> Vec<(u64, Curve)> {
        let mut entries = self
            .curves
            .iter()
            .map(|(id, curve)| (*id, (**curve).clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(id, _)| *id);
        entries
    }

    /// プロジェクトロード時に全カーブを差し替える。
    pub fn load(&mut self, entries: Vec<(u64, Curve)>) {
        self.curves.clear();
        self.next_id = 1;
        self.assignments.clear();
        self.next_assignment = 1;
        for (id, curve) in entries {
            self.curves.insert(id, Arc::new(curve));
            if id >= self.next_id {
                self.next_id = id.saturating_add(1);
            }
        }
        if self.next_id == 0 {
            self.next_id = 1;
        }
    }

    pub fn clear(&mut self) {
        self.curves.clear();
        self.next_id = 1;
        self.assignments.clear();
        self.next_assignment = 1;
    }
}

static REGISTRY: OnceLock<RwLock<CurveRegistry>> = OnceLock::new();

fn registry() -> &'static RwLock<CurveRegistry> {
    REGISTRY.get_or_init(|| RwLock::new(CurveRegistry::default()))
}

pub fn register(curve: Curve) -> u64 {
    registry().write().unwrap().register(curve)
}

pub fn register_with_id(id: u64, curve: Curve) -> bool {
    registry().write().unwrap().register_with_id(id, curve)
}

pub fn update(id: u64, curve: Curve) -> bool {
    registry().write().unwrap().update(id, curve)
}

pub fn remove(id: u64) -> Option<Arc<Curve>> {
    registry().write().unwrap().remove(id)
}

pub fn contains(id: u64) -> bool {
    registry().read().unwrap().contains(id)
}

pub fn get(id: u64) -> Option<Arc<Curve>> {
    registry().read().unwrap().get(id)
}

/// 適用のたびに増えるnonce。トラックパラメータに埋め込み、
/// キャッシュ(スクリプト側)の無効化に使う。
pub fn nonce() -> u64 {
    static NONCE: AtomicU64 = AtomicU64::new(1);
    NONCE.fetch_add(1, Ordering::Relaxed)
}

pub fn create_assignment(segment_ids: Vec<u64>) -> u64 {
    registry().write().unwrap().create_assignment(segment_ids)
}

pub fn set_assignment(id: u64, segment_ids: Vec<u64>) -> bool {
    registry().write().unwrap().set_assignment(id, segment_ids)
}

pub fn assignment(id: u64) -> Option<Vec<u64>> {
    registry().read().unwrap().assignment(id)
}

pub fn assignment_curve(id: u64, index: usize) -> Option<u64> {
    registry().read().unwrap().assignment_curve(id, index)
}

pub fn assignment_snapshot() -> Vec<(u64, Vec<u64>)> {
    registry().read().unwrap().assignment_snapshot()
}

pub fn load_assignments(entries: Vec<(u64, Vec<u64>)>) {
    registry().write().unwrap().load_assignments(entries);
}

pub fn snapshot() -> Vec<(u64, Curve)> {
    registry().read().unwrap().snapshot()
}

pub fn load(entries: Vec<(u64, Curve)>) {
    registry().write().unwrap().load(entries);
}

pub fn clear() {
    registry().write().unwrap().clear();
    clear_last_applied();
}

/// 最後に適用したカーブID(オブジェクト右クリックメニューからの再適用用)。
static LAST_APPLIED: AtomicU64 = AtomicU64::new(NO_LAST_APPLIED);

pub fn set_last_applied(id: u64) {
    LAST_APPLIED.store(id, Ordering::Relaxed);
}

pub fn clear_last_applied() {
    LAST_APPLIED.store(NO_LAST_APPLIED, Ordering::Relaxed);
}

pub fn last_applied() -> Option<u64> {
    let id = LAST_APPLIED.load(Ordering::Relaxed);
    (id != NO_LAST_APPLIED).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::default_bezier;

    #[test]
    fn first_registered_id_is_one() {
        let mut registry = CurveRegistry::default();
        let id = registry.register(Curve::Bezier(default_bezier()));
        assert_eq!(id, 1);
        assert!(registry.contains(1));
    }

    #[test]
    fn load_keeps_ids_and_advances_next_id() {
        let mut registry = CurveRegistry::default();
        registry.load(vec![
            (3, Curve::Bezier(default_bezier())),
            (7, Curve::Bezier(default_bezier())),
        ]);
        let id = registry.register(Curve::Bezier(default_bezier()));
        assert_eq!(id, 8);
    }

    #[test]
    fn register_with_explicit_id() {
        let mut registry = CurveRegistry::default();
        assert!(registry.register_with_id(5, Curve::Bezier(default_bezier())));
        assert!(!registry.register_with_id(5, Curve::Bezier(default_bezier())));
        assert!(registry.contains(5));
        assert_eq!(registry.register(Curve::Bezier(default_bezier())), 6);
    }

    #[test]
    fn assignments_keep_segment_ids_and_reload() {
        let mut registry = CurveRegistry::default();
        let id = registry.create_assignment(vec![3, 4, 5]);
        assert_eq!(registry.assignment(id), Some(vec![3, 4, 5]));
        assert!(registry.set_assignment(id, vec![6, 7]));
        assert_eq!(registry.assignment_curve(id, 1), Some(7));
        let snapshot = registry.assignment_snapshot();
        let mut loaded = CurveRegistry::default();
        loaded.load_assignments(snapshot);
        assert_eq!(loaded.assignment(id), Some(vec![6, 7]));
        let next = loaded.create_assignment(vec![1]);
        assert!(next > id);
    }

    #[test]
    fn update_and_remove() {
        let mut registry = CurveRegistry::default();
        let id = registry.register(Curve::Bezier(default_bezier()));
        assert!(registry.update(id, Curve::Bezier(default_bezier())));
        assert!(!registry.update(id + 1, Curve::Bezier(default_bezier())));
        assert!(registry.remove(id).is_some());
        assert!(!registry.contains(id));
    }
}
