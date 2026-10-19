//! WebView(JS)⇔RustのIPCブリッジと、埋め込みアセットの配信。

use std::borrow::Cow;

use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};

use crate::curve::{self, Curve, apply};

/// フロントエンドのURL(カスタムプロトコル)。
pub const FRONTEND_URL: &str = "hl-ezing://localhost/index.html";

/// プロジェクト編集用のグローバルハンドル。
pub static EDIT_HANDLE: aviutl2::generic::GlobalEditHandle =
    aviutl2::generic::GlobalEditHandle::new();

/// 埋め込みアセット (path, content_type, content)。
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "/index.html",
        "text/html; charset=utf-8",
        include_str!("../ui-react/index.html"),
    ),
    (
        "/assets/index.css",
        "text/css; charset=utf-8",
        include_str!("../ui-react/assets/index.css"),
    ),
    (
        "/assets/main.js",
        "application/javascript; charset=utf-8",
        include_str!("../ui-react/assets/main.js"),
    ),
];

/// カスタムプロトコルでアセットを配信する。
pub fn serve_asset(_id: wry::WebViewId, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    for (asset_path, content_type, content) in ASSETS {
        if path == *asset_path {
            return Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, *content_type)
                .body(Cow::Borrowed(content.as_bytes()))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Cow::Borrowed(b"error" as &[u8]))
                        .unwrap()
                });
        }
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Cow::Borrowed(b"not found" as &[u8]))
        .unwrap()
}

/// JS (`window.ipc.postMessage`) からのIPCメッセージを処理する。
pub fn handle_ipc(request: Request<String>) {
    let body = request.body().clone();
    tracing::debug!("IPC: {body}");
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return;
    };
    let Some(cmd) = value.get("cmd").and_then(|c| c.as_str()) else {
        return;
    };
    match cmd {
        "apply" => handle_apply_request(&value, "applied"),
        "drop" => handle_apply_request(&value, "applied"),
        "start_drag" => handle_start_drag(&value),
        "get_tracks" => handle_get_tracks(),
        "get_curves" => handle_get_curves(),
        "create_curve" => handle_create_curve(&value),
        "update_curve" => handle_update_curve(&value),
        "delete_curve" => handle_delete_curve(&value),
        "hello" => {
            tracing::info!("Frontend is ready (IPC round-trip OK).");
            send_event(serde_json::json!({"event":"hello","ok":true}));
        }
        "getVersion" => send_event(serde_json::json!({
            "event": "version",
            "name": "HL_Ezing",
            "author": "halkun19",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        _ => tracing::warn!("Unknown IPC command: {cmd}"),
    }
}

/// 登録済みカーブ一覧を返す。フロントエンドのID切り替え用。
fn handle_get_curves() {
    let curves = curve::registry::snapshot()
        .into_iter()
        .map(|(id, curve)| curve_to_json(id, &curve))
        .collect::<Vec<_>>();
    send_event(serde_json::json!({
        "event": "curves",
        "ok": true,
        "curves": curves,
        "last_applied": curve::registry::last_applied(),
    }));
}

fn handle_create_curve(value: &serde_json::Value) {
    let curve = match parse_curve(value) {
        Ok(curve) => curve.sanitized(),
        Err(error) => {
            send_event(serde_json::json!({"event":"curve_created","ok":false,"message":error}));
            return;
        }
    };
    let requested_id = value.get("id").and_then(|id| id.as_u64());
    let id = match requested_id {
        Some(id) if curve::registry::contains(id) => id,
        Some(id) if id > 0 => {
            if !curve::registry::register_with_id(id, curve) {
                send_event(serde_json::json!({
                    "event": "curve_created",
                    "ok": false,
                    "id": id,
                    "message": "指定されたカーブIDは使用できません。",
                }));
                return;
            }
            id
        }
        Some(_) => {
            send_event(serde_json::json!({
                "event": "curve_created",
                "ok": false,
                "message": "カーブIDは1以上で指定してください。",
            }));
            return;
        }
        None => curve::registry::register(curve),
    };
    let stored = curve::registry::get(id)
        .map(|curve| curve_to_json(id, &curve))
        .unwrap_or(serde_json::Value::Null);
    send_event(serde_json::json!({
        "event": "curve_created",
        "ok": true,
        "curve": stored,
    }));
}

fn handle_update_curve(value: &serde_json::Value) {
    let Some(id) = value.get("id").and_then(|id| id.as_u64()) else {
        send_event(
            serde_json::json!({"event":"curve_updated","ok":false,"message":"カーブIDがありません。"}),
        );
        return;
    };
    let curve = match parse_curve(value) {
        Ok(curve) => curve.sanitized(),
        Err(error) => {
            send_event(
                serde_json::json!({"event":"curve_updated","ok":false,"id":id,"message":error}),
            );
            return;
        }
    };
    if !curve::registry::update(id, curve.clone()) {
        send_event(
            serde_json::json!({"event":"curve_updated","ok":false,"id":id,"message":"指定されたカーブIDが存在しません。"}),
        );
        return;
    }
    send_event(serde_json::json!({
        "event": "curve_updated",
        "ok": true,
        "curve": curve_to_json(id, &curve),
    }));
}

fn handle_delete_curve(value: &serde_json::Value) {
    let Some(id) = value.get("id").and_then(|id| id.as_u64()) else {
        send_event(
            serde_json::json!({"event":"curve_deleted","ok":false,"message":"カーブIDがありません。"}),
        );
        return;
    };
    if curve::registry::remove(id).is_none() {
        send_event(
            serde_json::json!({"event":"curve_deleted","ok":false,"id":id,"message":"指定されたカーブIDが存在しません。"}),
        );
        return;
    }
    if curve::registry::last_applied() == Some(id) {
        curve::registry::clear_last_applied();
    }
    send_event(serde_json::json!({"event":"curve_deleted","ok":true,"id":id}));
}

/// ドラッグ開始。現状のAviUtl2公開APIではウィンドウ外のトラックバー位置を
/// 取得できないため、ここでは「適用」は行わず、ドラッグ状態だけを返す。
fn handle_start_drag(value: &serde_json::Value) {
    let Some(id) = value.get("curve_id").and_then(|id| id.as_u64()) else {
        send_event(
            serde_json::json!({"event":"drag_started","ok":false,"message":"カーブIDがありません。"}),
        );
        return;
    };
    if !curve::registry::contains(id) {
        send_event(
            serde_json::json!({"event":"drag_started","ok":false,"message":"指定されたカーブIDが存在しません。"}),
        );
        return;
    }
    send_event(serde_json::json!({"event":"drag_started","ok":true,"id":id}));
}

/// 選択オブジェクトの適用可能なトラックバー一覧を返す。
fn handle_get_tracks() {
    let result = EDIT_HANDLE.call_edit_section(|edit| {
        let mut objects = edit.get_selected_objects().map_err(|e| e.to_string())?;
        if objects.is_empty()
            && let Some(obj) = edit.get_focused_object().map_err(|e| e.to_string())?
        {
            objects.push(obj);
        }
        let Some(object) = objects.first() else {
            return Err("オブジェクトが選択されていません。".to_string());
        };

        let mut tracks = Vec::<serde_json::Value>::new();
        let effects = edit.get_effects(*object).map_err(|e| e.to_string())?;
        // 同名エフェクトが複数ある場合にラベルを区別するための出現回数
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for effect in &effects {
            let name = edit.get_effect_name(*effect).map_err(|e| e.to_string())?;
            *counts.entry(name).or_insert(0) += 1;
        }
        let mut seen = std::collections::HashMap::<String, usize>::new();
        for effect in effects {
            let effect_name = edit.get_effect_name(effect).map_err(|e| e.to_string())?;
            let idx = *seen.entry(effect_name.clone()).or_insert(0);
            *seen.entry(effect_name.clone()).or_insert(0) += 1;
            let suffix = if counts.get(&effect_name).copied().unwrap_or(0) > 1 {
                format!("（{}番目）", idx + 1)
            } else {
                String::new()
            };
            let Ok(items) = EDIT_HANDLE.get_effect_items(&effect_name) else {
                continue;
            };
            for item in items {
                // 移動方法を持たない項目は Ok(None)、トラックバーでない項目は Err。
                let info = match edit.get_effect_track_info(effect, &item.name) {
                    Ok(info) => info,
                    Err(_) => continue,
                };
                let curve_id = info
                    .as_ref()
                    .filter(|info| info.mode == apply::SCRIPT_MODE_NAME)
                    .and_then(|info| info.params.first())
                    .map(|id| *id as u64);
                tracks.push(serde_json::json!({
                    "effect": effect_name,
                    "effect_index": idx,
                    "item": item.name,
                    "curve_id": curve_id,
                    "label": format!("{} / {}{}", effect_name, item.name, suffix),
                }));
            }
        }
        Ok(tracks)
    });

    match result {
        Ok(Ok(tracks)) => {
            send_event(serde_json::json!({"event":"tracks","ok":true,"tracks":tracks}));
        }
        Ok(Err(e)) => {
            send_event(serde_json::json!({"event":"tracks","ok":false,"message":e,"tracks":[]}));
        }
        Err(e) => {
            send_event(
                serde_json::json!({"event":"tracks","ok":false,"message":e.to_string(),"tracks":[]}),
            );
        }
    }
}

/// 適用リクエスト。`{curve_id, mode, data, ...}` のカーブを登録/更新し、
/// 対象トラックへ適用する。`apply` と `drop` で共通。
fn handle_apply_request(value: &serde_json::Value, event: &str) {
    let requested_id = value.get("curve_id").and_then(|id| id.as_u64());
    let parsed_curve = if value.get("data").is_some() {
        match parse_curve(value) {
            Ok(curve) => Some(curve.sanitized()),
            Err(error) => {
                send_event(serde_json::json!({"event":event,"ok":false,"message":error}));
                return;
            }
        }
    } else {
        None
    };

    let (curve_id, created) = match (requested_id, parsed_curve) {
        (Some(id), Some(curve)) => {
            if curve::registry::contains(id) {
                curve::registry::update(id, curve);
                (id, false)
            } else {
                (curve::registry::register(curve), true)
            }
        }
        (Some(id), None) => {
            if curve::registry::contains(id) {
                (id, false)
            } else {
                send_event(
                    serde_json::json!({"event":event,"ok":false,"message":"指定されたカーブIDが存在しません。"}),
                );
                return;
            }
        }
        (None, Some(curve)) => (curve::registry::register(curve), true),
        (None, None) => {
            send_event(
                serde_json::json!({"event":event,"ok":false,"message":"カーブデータがありません。"}),
            );
            return;
        }
    };

    let target_effect = value.get("effect_name").and_then(|i| i.as_str());
    let target_effect_index = value.get("effect_index").and_then(|i| i.as_u64());
    let target_item = value.get("item_name").and_then(|i| i.as_str());

    let result = EDIT_HANDLE.call_edit_section(|edit| {
        if target_item.is_some() || target_effect.is_some() || target_effect_index.is_some() {
            apply::apply_to_single_track(
                edit,
                curve_id,
                target_effect,
                target_effect_index.map(|i| i as usize),
                target_item,
            )
        } else {
            apply::apply_to_all_tracks(edit, curve_id)
        }
    });

    match result {
        Ok(Ok(count)) => {
            curve::registry::set_last_applied(curve_id);
            let stored = curve::registry::get(curve_id)
                .map(|curve| curve_to_json(curve_id, &curve))
                .unwrap_or(serde_json::Value::Null);
            send_event(serde_json::json!({
                "event": event,
                "ok": true,
                "count": count,
                "id": curve_id,
                "created": created,
                "curve": stored,
            }));
        }
        Ok(Err(e)) => {
            if created {
                // 適用できなかった新規カーブはプロジェクトに残さない。
                curve::registry::remove(curve_id);
            }
            send_event(serde_json::json!({"event":event,"ok":false,"message":e}));
        }
        Err(e) => {
            if created {
                curve::registry::remove(curve_id);
            }
            send_event(serde_json::json!({"event":event,"ok":false,"message":e.to_string()}));
        }
    }
}

fn parse_curve(value: &serde_json::Value) -> Result<Curve, String> {
    let mode = value
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("bezier");
    let data = value
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Curveはmodeタグ付きenumなので、modeを付与してデシリアライズする
    let mut tagged = serde_json::Map::new();
    tagged.insert("mode".into(), serde_json::Value::String(mode.to_string()));
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            if key != "mode" {
                tagged.insert(key.clone(), value.clone());
            }
        }
    }
    serde_json::from_value(serde_json::Value::Object(tagged)).map_err(|e| e.to_string())
}

fn curve_to_json(id: u64, curve: &Curve) -> serde_json::Value {
    let mut value = serde_json::to_value(curve).unwrap_or(serde_json::Value::Null);
    let mode = value
        .get("mode")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("bezier".into()));
    if let Some(object) = value.as_object_mut() {
        object.remove("mode");
    }
    serde_json::json!({
        "id": id,
        "mode": mode,
        "data": value,
    })
}

/// Rust→JSへイベントを送る。
pub fn send_event(payload: serde_json::Value) {
    // JSONを直接スクリプトへ埋め込まず、JSON文字列として埋め込んで
    // JSON.parse させることで、U+2028/U+2029 などによる構文破壊を避ける。
    let serialized = serde_json::to_string(&payload).unwrap_or_else(|_| "null".into());
    let literal = serde_json::to_string(&serialized).unwrap_or_else(|_| "\"null\"".into());
    let script = format!("window.hlEzing && window.hlEzing.onEvent(JSON.parse({literal}))");
    crate::window::evaluate(script);
}
