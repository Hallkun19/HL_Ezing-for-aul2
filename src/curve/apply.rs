//! カーブの適用ロジック。
//!
//! 選択中オブジェクトのトラックバー(移動方法があるもの)に対して、カーブをスクリプト
//! 移動方法(HL_Ezing)として適用する。トラック値はエイリアスファイル形式
//! `left,mid...,right,<mode>,<flags>|<param...>` で `set_effect_item_value` に渡す。
//!
//! 重要な点として、既存の値(中間点)をすべて保持したまま移動方法だけを
//! `HL_Ezing` に差し替える。これにより、各区間(中間点間)に対して同じカーブが
//! 適用される。

use aviutl2::generic::{EditSection, EffectHandle, TrackInfo};

use super::{registry, track_value::TrackValue};

/// スクリプトモジュールの登録名(.tra2から `obj.module` で参照する名前)。
pub const SCRIPT_MODULE_NAME: &str = "HL_Ezing.aux2";
/// トラックの移動方法として設定するスクリプト名。
pub const SCRIPT_MODE_NAME: &str = "HL_Ezing";
/// プロジェクト保存用のキー。
pub const PROJECT_KEY: &str = "hl_ezing_curves";
/// 最後に適用したカーブIDのプロジェクト保存用キー。
pub const LAST_APPLIED_KEY: &str = "hl_ezing_last_applied";
/// 区間ごとのカーブID割り当てのプロジェクト保存用キー。
pub const ASSIGNMENTS_KEY: &str = "hl_ezing_segment_assignments";
/// 区間割り当てIDを通常のカーブIDと区別するためのオフセット。
/// カーブIDの入力上限は 999999 なので、10億以上は割り当てIDとみなせる。
const ASSIGNMENT_PARAM_OFFSET: u64 = 1_000_000_000;

/// 指定またはフォーカスされている単一のトラックバーだけにカーブを適用する。
/// `target_effect` / `target_effect_index` / `target_item` が指定されている場合は、
/// そのエフェクト(インスタンス)・項目名に一致するものだけに適用する。
/// 指定がない場合は最初のアニメトラック1本のみに適用する。
pub fn apply_to_single_track(
    edit: &EditSection,
    curve_id: u64,
    target_effect: Option<&str>,
    target_effect_index: Option<usize>,
    target_item: Option<&str>,
) -> Result<usize, String> {
    let Some(object) = focused_or_first_object(edit)? else {
        return Err("対象のオブジェクトが見つかりません。".into());
    };

    // 単一トラックへの適用では、シークバー位置が取れない場合でも
    // 全區間を上書きせず、先頭区間だけを対象にする。
    let segment = Some(focus_segment(edit).unwrap_or(0));
    let effects = edit.get_effects(object).map_err(|e| e.to_string())?;
    // 同名エフェクトのインスタンスを区別するための名前別インデックス
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for effect in effects {
        let name = edit.get_effect_name(effect).map_err(|e| e.to_string())?;
        let idx = seen.entry(name.clone()).or_insert(0);
        // エフェクト名が指定されている場合は一致するものだけに絞る
        // (同じ名前のトラックバーが複数ある場合、誤って最初の1本に適用されるのを防ぐ)
        if let Some(target_effect) = target_effect
            && name != target_effect
        {
            *idx += 1;
            continue;
        }
        if let Some(target_index) = target_effect_index
            && *idx != target_index
        {
            *idx += 1;
            continue;
        }
        *idx += 1;
        let Ok(items) = crate::ipc::EDIT_HANDLE.get_effect_items(&name) else {
            continue;
        };
        for item in items {
            if edit.get_effect_track_info(effect, &item.name).is_err() {
                continue;
            }
            // 特定項目が指定されている場合は名前でフィルタリング
            if let Some(target) = target_item
                && item.name != target
            {
                continue;
            }
            // 最初に対象となった単一のトラックバーだけに適用して終了する
            if apply_to_track_with_group(edit, object, effect, &item.name, curve_id, segment) {
                return Ok(1);
            }
        }
    }
    Err("適用可能なトラックバーが見つかりませんでした。".into())
}

/// 選択中オブジェクトの全トラックバーへカーブを適用する。
pub fn apply_to_all_tracks(edit: &EditSection, curve_id: u64) -> Result<usize, String> {
    let Some(object) = first_object(edit)? else {
        return Err("対象のオブジェクトが見つかりません。".into());
    };

    let effects = edit.get_effects(object).map_err(|e| e.to_string())?;
    let mut applied = 0usize;
    for effect in effects {
        let name = edit.get_effect_name(effect).map_err(|e| e.to_string())?;
        let Ok(items) = crate::ipc::EDIT_HANDLE.get_effect_items(&name) else {
            continue;
        };
        for item in items {
            // 移動方法を持たない項目は get_effect_track_info が Ok(None) を返す。
            // Err の場合はトラックバーではないので除外する。
            if edit.get_effect_track_info(effect, &item.name).is_err() {
                continue;
            }
            if apply_to_track(edit, object, effect, &item.name, curve_id, None) {
                applied += 1;
            }
        }
    }

    if applied == 0 {
        Err("適用可能なトラックバーが見つかりませんでした。".into())
    } else {
        Ok(applied)
    }
}

/// 選択中オブジェクトへのカーブ適用（全トラックバー）。
pub fn apply_to_selection(edit: &EditSection, curve_id: u64) -> Result<usize, String> {
    apply_to_all_tracks(edit, curve_id)
}

/// 指定された設定項目だけにカーブを適用する。Curve Editor風の右クリックメニュー用。
pub fn apply_to_item(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
    effect_name: &str,
    effect_index: usize,
    item: &str,
    curve_id: u64,
) -> Result<(), String> {
    apply_to_item_on_object(edit, object, effect_name, effect_index, item, curve_id)
}

/// オブジェクトを明示して1本のトラックバーへ適用する。
///
/// ウィンドウ外からのドラッグ&ドロップでは、選択オブジェクトではなく
/// オブジェクト設定に表示されているオブジェクトへ適用したいため、
/// オブジェクトハンドルを呼び出し側から渡せるようにしている。
pub fn apply_to_item_on_object(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
    effect_name: &str,
    effect_index: usize,
    item: &str,
    curve_id: u64,
) -> Result<(), String> {
    let effect = edit
        .find_effect(object, effect_name, effect_index)
        .map_err(|e| e.to_string())?;
    if edit.get_effect_track_info(effect, item).is_err() {
        return Err("対象のトラックバーが見つかりません。".into());
    }
    let segment = Some(focus_segment(edit).unwrap_or(0));
    if apply_to_track_with_group(edit, object, effect, item, curve_id, segment) {
        Ok(())
    } else {
        Err("トラックバーの移動方法を書き換えられませんでした。".into())
    }
}

fn focus_segment(edit: &EditSection) -> Option<usize> {
    edit.get_focus_object_section().ok().flatten()
}

/// オブジェクト設定にフォーカスされているオブジェクトを優先して返す。
/// フォーカスがない場合だけ選択オブジェクトの先頭へフォールバックする。
pub fn focused_or_first_object(
    edit: &EditSection,
) -> Result<Option<aviutl2::generic::ObjectHandle>, String> {
    if let Ok(Some(object)) = edit.get_focused_object() {
        return Ok(Some(object));
    }
    first_object(edit)
}

fn first_object(edit: &EditSection) -> Result<Option<aviutl2::generic::ObjectHandle>, String> {
    let mut objects = edit.get_selected_objects().map_err(|e| e.to_string())?;
    if objects.is_empty()
        && let Some(object) = edit.get_focused_object().map_err(|e| e.to_string())?
    {
        objects.push(object);
    }
    Ok(objects.first().copied())
}

/// ドラッグ&ドロップの照合用に、適用可能なトラックバーを列挙する。
#[derive(Debug, Clone)]
pub struct TrackEntry {
    pub effect: String,
    pub effect_index: usize,
    pub item: String,
}

pub fn collect_tracks(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
) -> Result<Vec<TrackEntry>, String> {
    let effects = edit.get_effects(object).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut tracks = Vec::new();
    for effect in effects {
        let effect_name = edit.get_effect_name(effect).map_err(|e| e.to_string())?;
        let effect_index = {
            let index = *seen.entry(effect_name.clone()).or_insert(0);
            *seen.entry(effect_name.clone()).or_insert(0) += 1;
            index
        };
        let Ok(items) = crate::ipc::EDIT_HANDLE.get_effect_items(&effect_name) else {
            continue;
        };
        for item in items {
            if edit.get_effect_track_info(effect, &item.name).is_err() {
                continue;
            }
            tracks.push(TrackEntry {
                effect: effect_name.clone(),
                effect_index,
                item: item.name,
            });
        }
    }
    Ok(tracks)
}

fn track_info(edit: &EditSection, effect: EffectHandle, item: &str) -> Result<TrackInfo, ()> {
    edit.get_effect_track_info(effect, item).map_err(|_| ())
}

/// グループ化されたトラックバー(色のRGB、XYZなど)にも同じカーブを適用する。
fn apply_to_track_with_group(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
    effect: EffectHandle,
    item: &str,
    curve_id: u64,
    segment: Option<usize>,
) -> bool {
    let info = track_info(edit, effect, item).ok();
    let group_name = info.as_ref().and_then(|info| info.group_name.clone());
    let group_num = info.as_ref().map_or(1, |info| info.group_num);

    if group_name.is_some() && group_num > 1 {
        let Ok(effect_name) = edit.get_effect_name(effect) else {
            return apply_to_track(edit, object, effect, item, curve_id, segment);
        };
        let Ok(items) = crate::ipc::EDIT_HANDLE.get_effect_items(&effect_name) else {
            return apply_to_track(edit, object, effect, item, curve_id, segment);
        };
        let mut applied = false;
        for candidate in items {
            let candidate_info = track_info(edit, effect, &candidate.name).ok();
            if candidate_info
                .as_ref()
                .and_then(|info| info.group_name.as_ref())
                == group_name.as_ref()
            {
                applied |= apply_to_track(edit, object, effect, &candidate.name, curve_id, segment);
            }
        }
        return applied;
    }

    apply_to_track(edit, object, effect, item, curve_id, segment)
}

fn apply_to_track(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
    effect: EffectHandle,
    item: &str,
    curve_id: u64,
    segment: Option<usize>,
) -> bool {
    let Ok(layer_frame) = edit.get_object_layer_frame(object) else {
        return false;
    };
    let start_f = layer_frame.start as f64;
    let end_f = layer_frame.end as f64;

    let info = track_info(edit, effect, item).ok();
    let current_value = edit.get_effect_item_value(effect, item).ok();
    let mut track = current_value
        .as_deref()
        .and_then(|value| TrackValue::parse(value, info.as_ref()).ok())
        .unwrap_or_else(|| {
            let start = edit
                .get_effect_track_value(effect, item, start_f)
                .unwrap_or(0.0);
            let end = edit
                .get_effect_track_value(effect, item, end_f)
                .unwrap_or(start);
            TrackValue {
                values: vec![start, end],
                precision: 2,
                flag: 0,
                movement: None,
                time_control: None,
                reference: None,
            }
        });

    // 移動方法を持たないトラックバーは値が1点だけなので、開始点と終了点の
    // 2点に複製してカーブを適用できるようにする。
    if track.values.len() < 2 {
        let start = edit
            .get_effect_track_value(effect, item, start_f)
            .unwrap_or(0.0);
        let end = edit
            .get_effect_track_value(effect, item, end_f)
            .unwrap_or(start);
        track.ensure_two_points(start, end);
    }

    // 中間点ごとに区切るため、「中間点無視」と時間制御リンクを解除する。
    track.enable_midpoints();

    let nonce = registry::nonce();
    let segment_count = track.values.len().saturating_sub(1).max(1);

    // 既存の割り当てID(負のパラメータ)があれば、その区間リストを引き継ぐ。
    let existing_assignment = track.movement.as_ref().and_then(|movement| {
        if movement.name != SCRIPT_MODE_NAME {
            return None;
        }
        movement
            .parameters
            .first()
            .copied()
            .filter(|value| *value >= ASSIGNMENT_PARAM_OFFSET as f64)
            .map(|value| (value as u64).saturating_sub(ASSIGNMENT_PARAM_OFFSET))
    });

    let mut segment_ids = existing_assignment
        .and_then(registry::assignment)
        .unwrap_or_else(|| track.segment_curve_ids(SCRIPT_MODE_NAME, segment_count, curve_id));
    if segment_ids.is_empty() {
        segment_ids.push(curve_id);
    }
    while segment_ids.len() < segment_count {
        let fill = *segment_ids.last().unwrap_or(&curve_id);
        segment_ids.push(fill);
    }
    segment_ids.truncate(segment_count);

    match segment {
        Some(index) if index < segment_ids.len() => segment_ids[index] = curve_id,
        _ => segment_ids.fill(curve_id),
    }

    let assignment_id = match existing_assignment {
        Some(id) if registry::set_assignment(id, segment_ids.clone()) => id,
        _ => registry::create_assignment(segment_ids.clone()),
    };

    track.set_movement(
        SCRIPT_MODE_NAME,
        vec![
            (ASSIGNMENT_PARAM_OFFSET + assignment_id) as f64,
            nonce as f64,
        ],
    );
    let value = track.to_string();
    tracing::debug!(
        "Apply curve {curve_id} to {item} (segment {:?}, assignment {assignment_id}, ids {:?}): {value}",
        segment,
        segment_ids
    );
    edit.set_effect_item_value(effect, item, &value).is_ok()
}
