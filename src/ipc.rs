//! WebView(JS)⇔RustのIPCブリッジと、埋め込みアセットの配信。

use std::borrow::Cow;

use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};

use crate::curve::{self, Curve, apply};

/// フロントエンドのURL(カスタムプロトコル)。
pub const FRONTEND_URL: &str = "hl-ezing://localhost/index.html";

/// プロジェクト編集用のグローバルハンドル。
pub static EDIT_HANDLE: aviutl2::generic::GlobalEditHandle =
    aviutl2::generic::GlobalEditHandle::new();

/// WebView2 のカスタムプロトコル変換を含めた、埋め込み UI の URL 判定。
pub(crate) fn is_trusted_ui_uri(uri: &str) -> bool {
    uri == "about:blank"
        || uri.starts_with("hl-ezing://localhost/")
        || uri.starts_with("http://hl-ezing.localhost/")
        || uri.starts_with("https://hl-ezing.localhost/")
}

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
    // 埋め込み UI 以外のドキュメントから IPC が呼ばれても実行しない。
    // Windows の WebView2 はカスタムプロトコルを
    // http://hl-ezing.localhost/ に変換するため、その形も許可する。
    let uri = request.uri();
    if !is_trusted_ui_uri(&uri.to_string()) {
        tracing::warn!("Rejected IPC from unexpected origin: {uri}");
        return;
    }
    let body = request.body().clone();
    if body.len() > 4 * 1024 * 1024 {
        tracing::warn!("Rejected oversized IPC message: {} bytes", body.len());
        return;
    }
    tracing::debug!("IPC: {body}");
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return;
    };
    let Some(cmd) = value.get("cmd").and_then(|c| c.as_str()) else {
        return;
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match cmd {
        "apply" => handle_apply_request(&value, "applied"),
        "drop" => handle_apply_request(&value, "applied"),
        "start_drag" => handle_start_drag(&value),
        "prepare_drag" => crate::trackbar::warm_cache(),
        "host_space" => {
            crate::window::forward_space_to_main();
        }
        "get_segments" => handle_get_segments(),
        "select_segment" => handle_select_segment(&value),
        "debug_trackbars" => {
            crate::trackbar::log_debug_snapshot();
            send_event(serde_json::json!({
                "event": "trackbar_debug",
                "ok": true,
                "message": "診断情報をバックグラウンドでログへ出力しています。",
            }));
        }
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
    }));
    if result.is_err() {
        tracing::error!("IPC handler panicked for command {cmd}");
        send_event(serde_json::json!({
            "event": "ipc_error",
            "ok": false,
            "message": "IPC処理中にエラーが発生しました。",
        }));
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

/// ドラッグ開始。実際のトラックバー探索と適用は `trackbar` モジュールの
/// ワーカースレッドが行う。ここではカーブIDの検証と開始可否だけを返す。
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
    if !crate::trackbar::is_supported() {
        send_event(serde_json::json!({
            "event": "drag_started",
            "ok": false,
            "id": id,
            "message": "このAviUtl2ではドラッグ適用を利用できません（RTTI解決失敗）。診断ログを確認してください。",
        }));
        return;
    }
    if !crate::trackbar::start_drag(id) {
        send_event(serde_json::json!({
            "event": "drag_started",
            "ok": false,
            "id": id,
            "message": "ドラッグを開始できませんでした。",
        }));
        return;
    }
    send_event(serde_json::json!({"event":"drag_started","ok":true,"id":id}));
}

/// オブジェクトの区間情報と、現在フォーカスされている区間を返す。
fn handle_get_segments() {
    if !EDIT_HANDLE.is_ready() {
        send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "frames": [],
            "message": "編集セクションの初期化が完了していません。",
        }));
        return;
    }
    let result = EDIT_HANDLE.call_edit_section(|edit| -> Result<serde_json::Value, String> {
        let object = apply::focused_or_first_object(edit)?
            .ok_or_else(|| "オブジェクトが選択されていません。".to_string())?;
        if let Ok(count) = apply::migrate_legacy_tracks(edit, object)
            && count > 0
        {
            tracing::info!("Migrated {count} legacy HL_Ezing tracks.");
        }
        let frames = edit
            .get_object_section_frames(object)
            .map_err(|e| e.to_string())?;
        let focus = edit.get_focus_object_section().ok().flatten();
        let layer_frame = edit.get_object_layer_frame(object).ok();
        Ok(serde_json::json!({
            "frames": frames,
            "focus": focus,
            "layer_frame": layer_frame.map(|frame| serde_json::json!({
                "layer": frame.layer,
                "start": frame.start,
                "end": frame.end,
            })),
        }))
    });
    match result {
        Ok(Ok(mut payload)) => {
            payload["event"] = "segments".into();
            payload["ok"] = true.into();
            send_event(payload);
        }
        Ok(Err(message)) => send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "frames": [],
            "message": message,
        })),
        Err(error) => send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "frames": [],
            "message": error.to_string(),
        })),
    }
}

/// 区間バーから区間を選び、AviUtl2 のカーソルをその区間の先頭へ移動する。
fn handle_select_segment(value: &serde_json::Value) {
    let Some(index) = value.get("index").and_then(|index| index.as_u64()) else {
        return;
    };
    if !EDIT_HANDLE.is_ready() {
        send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "message": "編集セクションの初期化が完了していません。",
        }));
        return;
    }
    let result = EDIT_HANDLE.call_edit_section(|edit| -> Result<serde_json::Value, String> {
        let object = apply::focused_or_first_object(edit)?
            .ok_or_else(|| "オブジェクトが選択されていません。".to_string())?;
        let frames = edit
            .get_object_section_frames(object)
            .map_err(|e| e.to_string())?;
        let layer_frame = edit
            .get_object_layer_frame(object)
            .map_err(|e| e.to_string())?;
        let index = index as usize;
        let frame = *frames
            .get(index)
            .ok_or_else(|| "区間が見つかりません。".to_string())?;
        edit.set_cursor_layer_frame(layer_frame.layer, frame)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "frames": frames,
            "focus": index,
            "layer_frame": {
                "layer": layer_frame.layer,
                "start": layer_frame.start,
                "end": layer_frame.end,
            },
        }))
    });
    match result {
        Ok(Ok(mut payload)) => {
            payload["event"] = "segments".into();
            payload["ok"] = true.into();
            send_event(payload);
        }
        Ok(Err(message)) => send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "message": message,
        })),
        Err(error) => send_event(serde_json::json!({
            "event": "segments",
            "ok": false,
            "message": error.to_string(),
        })),
    }
}

/// 選択オブジェクトの適用可能なトラックバー一覧を返す。
fn handle_get_tracks() {
    if !EDIT_HANDLE.is_ready() {
        send_event(serde_json::json!({
            "event": "tracks",
            "ok": false,
            "message": "編集セクションの初期化が完了していません。",
            "tracks": [],
        }));
        return;
    }
    let result = EDIT_HANDLE.call_edit_section(|edit| {
        let Some(object) = apply::focused_or_first_object(edit)? else {
            return Err("オブジェクトが選択されていません。".to_string());
        };
        if let Ok(count) = apply::migrate_legacy_tracks(edit, object)
            && count > 0
        {
            tracing::info!("Migrated {count} legacy HL_Ezing tracks.");
        }

        let mut tracks = Vec::<serde_json::Value>::new();
        let effects = edit.get_effects(object).map_err(|e| e.to_string())?;
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
                    .mode
                    .as_deref()
                    .filter(|mode| *mode == apply::SCRIPT_MODE_NAME)
                    .and_then(|_| info.params.first())
                    .and_then(|value| {
                        let value = *value as u64;
                        if value >= apply::ASSIGNMENT_PARAM_OFFSET {
                            curve::registry::assignment_curve(
                                value - apply::ASSIGNMENT_PARAM_OFFSET,
                                0,
                            )
                        } else {
                            (value > 0).then_some(value)
                        }
                    });
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
    if !EDIT_HANDLE.is_ready() {
        send_event(serde_json::json!({
            "event": event,
            "ok": false,
            "message": "編集セクションの初期化が完了していません。",
        }));
        return;
    }
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

pub(crate) fn curve_to_json(id: u64, curve: &Curve) -> serde_json::Value {
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
