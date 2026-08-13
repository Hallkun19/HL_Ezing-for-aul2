//! WebView(JS)⇔RustのIPCブリッジと、埋め込みアセットの配信。

use std::borrow::Cow;

use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};

use crate::curve::{self, Curve};

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
        "apply" | "start_drag" => handle_apply(&value),
        "drop" => handle_drop(&value),
        "get_tracks" => handle_get_tracks(),
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

/// 選択オブジェクトの適用可能なトラックバー一覧を返す。
fn handle_get_tracks() {
    let result = EDIT_HANDLE.call_edit_section(|edit| {
        let mut objects = edit.get_selected_objects().map_err(|e| e.to_string())?;
        if objects.is_empty() {
            if let Some(obj) = edit.get_focused_object().map_err(|e| e.to_string())? {
                objects.push(obj);
            }
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
                format!(" ({})", idx + 1)
            } else {
                String::new()
            };
            let Ok(items) = EDIT_HANDLE.get_effect_items(&effect_name) else {
                continue;
            };
            for item in items {
                if edit.get_effect_track_info(effect, &item.name).is_err() {
                    continue;
                }
                tracks.push(serde_json::json!({
                    "effect": effect_name,
                    "effect_index": idx,
                    "item": item.name,
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
            send_event(serde_json::json!({"event":"tracks","ok":false,"message":e.to_string(),"tracks":[]}));
        }
    }
}

/// ドロップ時の適用ハンドラ
fn handle_drop(value: &serde_json::Value) {
    let Some(curve_id) = curve::registry::last_applied() else {
        return;
    };
    let target_effect = value.get("effect_name").and_then(|i| i.as_str());
    let target_effect_index = value.get("effect_index").and_then(|i| i.as_u64());
    let target_item = value.get("item_name").and_then(|i| i.as_str());
    let result = EDIT_HANDLE.call_edit_section(|edit| {
        curve::apply::apply_to_single_track(
            edit,
            curve_id,
            target_effect,
            target_effect_index.map(|i| i as usize),
            target_item,
        )
    });
    match result {
        Ok(Ok(count)) => {
            send_event(
                serde_json::json!({"event":"applied","ok":true,"count":count,"id":curve_id}),
            );
        }
        Ok(Err(e)) => {
            send_event(serde_json::json!({"event":"applied","ok":false,"message":e}));
        }
        Err(e) => {
            send_event(serde_json::json!({"event":"applied","ok":false,"message":e.to_string()}));
        }
    }
}

/// 適用コマンド。`{mode, data, item_name}` のカーブを登録し、特定トラックへ適用する。
fn handle_apply(value: &serde_json::Value) {
    let mode = value
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("bezier");
    let data = value
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let target_effect = value.get("effect_name").and_then(|i| i.as_str());
    let target_effect_index = value.get("effect_index").and_then(|i| i.as_u64());
    let target_item = value.get("item_name").and_then(|i| i.as_str());

    // Curveはmodeタグ付きenumなので、modeを付与してデシリアライズする
    let mut tagged = serde_json::Map::new();
    tagged.insert("mode".into(), serde_json::Value::String(mode.to_string()));
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            tagged.insert(k.clone(), v.clone());
        }
    }
    let curve: Curve = match serde_json::from_value(serde_json::Value::Object(tagged)) {
        Ok(c) => c,
        Err(e) => {
            send_event(serde_json::json!({"event":"applied","ok":false,"message":e.to_string()}));
            return;
        }
      };

    let curve_id = curve::registry::register(curve);
    curve::registry::set_last_applied(curve_id);

    let result = EDIT_HANDLE.call_edit_section(|edit| {
        curve::apply::apply_to_single_track(
            edit,
            curve_id,
            target_effect,
            target_effect_index.map(|i| i as usize),
            target_item,
        )
    });
    match result {
        Ok(Ok(count)) => {
            send_event(
                serde_json::json!({"event":"applied","ok":true,"count":count,"id":curve_id}),
            );
        }
        Ok(Err(e)) => {
            send_event(serde_json::json!({"event":"applied","ok":false,"message":e}));
        }
        Err(e) => {
            send_event(serde_json::json!({"event":"applied","ok":false,"message":e.to_string()}));
        }
    }
}

/// Rust→JSへイベントを送る。
pub fn send_event(payload: serde_json::Value) {
    let script = format!("window.hlEzing && window.hlEzing.onEvent({payload})");
    crate::window::evaluate(script);
}
