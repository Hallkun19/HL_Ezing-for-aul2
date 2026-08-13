//! カーブの適用ロジック。
//!
//! 選択中オブジェクトのトラックバー(移動方法があるもの)に対して、カーブをスクリプト
//! 移動方法(HL_Ezing)として適用する。トラックの値文字列はエイリアスファイル形式
//! `left,right,<flags>@<script>,<param...>` で `set_effect_item_value` に渡す。

use aviutl2::generic::{EditSection, EffectHandle};

use super::registry;

/// スクリプトモジュールの登録名(.tra2から `obj.module` で参照する名前)。
pub const SCRIPT_MODULE_NAME: &str = "HL_Ezing.aux2";
/// トラックの移動方法として設定するスクリプト名。
pub const SCRIPT_MODE_NAME: &str = "HL_Ezing";
/// プロジェクト保存用のキー。
pub const PROJECT_KEY: &str = "hl_ezing_curves";

/// 指定またはフォーカスされている単一のトラックバーだけにカーブを適用する（全プロパティ適用を防止）。
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
    let mut objects = edit.get_selected_objects().map_err(|e| e.to_string())?;
    if objects.is_empty()
        && let Some(object) = edit.get_focused_object().map_err(|e| e.to_string())?
    {
        objects.push(object);
    }
    let Some(object) = objects.first() else {
        return Err("対象のオブジェクトが見つかりません。".into());
    };

    let effects = edit.get_effects(*object).map_err(|e| e.to_string())?;
    // 同名エフェクトのインスタンスを区別するための名前別インデックス
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for effect in effects {
        let name = edit.get_effect_name(effect).map_err(|e| e.to_string())?;
        let idx = seen.entry(name.clone()).or_insert(0);
        // エフェクト名が指定されている場合は一致するものだけに絞る
        // (同じ名前のトラックバーが複数ある場合、誤って最初の1本に適用されるのを防ぐ)
        if let Some(target_effect) = target_effect {
            if name != target_effect {
                *idx += 1;
                continue;
            }
        }
        if let Some(target_index) = target_effect_index {
            if *idx != target_index {
                *idx += 1;
                continue;
            }
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
            if let Some(target) = target_item {
                if item.name != target {
                    continue;
                }
            }
            // 最初に対象となった単一のトラックバーだけに適用して終了する
            if apply_to_track(edit, *object, effect, &item.name, curve_id) {
                return Ok(1);
            }
        }
    }
    Err("適用可能なトラックバーが見つかりませんでした。".into())
}

/// 選択中オブジェクトへのカーブ適用（単一プロパティ限定）。
pub fn apply_to_selection(edit: &EditSection, curve_id: u64) -> Result<usize, String> {
    apply_to_single_track(edit, curve_id, None, None, None)
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
    let effect = edit
        .find_effect(object, effect_name, effect_index)
        .map_err(|e| e.to_string())?;
    if edit.get_effect_track_info(effect, item).is_err() {
        return Err("対象のトラックバーに移動方法がありません。".into());
    }
    if apply_to_track(edit, object, effect, item, curve_id) {
        Ok(())
    } else {
        Err("トラックバーの移動方法を書き換えられませんでした。".into())
    }
}

fn apply_to_track(
    edit: &EditSection,
    object: aviutl2::generic::ObjectHandle,
    effect: EffectHandle,
    item: &str,
    curve_id: u64,
) -> bool {
    let Ok(layer_frame) = edit.get_object_layer_frame(object) else {
        return false;
    };
    let start_f = layer_frame.start as f64;
    let end_f = layer_frame.end as f64;

    // 現在の始点/終点の値を取得して維持する
    let Ok(left) = edit.get_effect_track_value(effect, item, start_f) else {
        return false;
    };
    let Ok(right) = edit.get_effect_track_value(effect, item, end_f) else {
        return false;
    };

    // トラック値をスクリプト移動方法 + カーブIDに書き換える
    // 形式: left,right,HL_Ezing,0|curve_id,nonce
    // (param[0]はAviUtl2のフラグコード用に0を固定、以降がスクリプトパラメータ)
    let nonce = registry::nonce();
    let value = format!("{left},{right},{SCRIPT_MODE_NAME},0|{curve_id},{nonce}");
    edit.set_effect_item_value(effect, item, &value).is_ok()
}
