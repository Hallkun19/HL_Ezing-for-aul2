//! HL_Ezing - AviUtl2向けイージングプラグイン。
//!
//! - GUI: WebView2(HTML/CSS/JS) によるカーブエディタ。AE版HL_Ezing2のGUIを参考に新規構築、
//!   ロジックはoriginal_backupを流用。
//! - 適用: カーブをプロジェクトへ保存(ID管理)し、選択オブジェクトのトラックバーへ
//!   `HL_Ezing.tra2` スクリプト移動方法として適用。
//! - 実行時評価: スクリプトモジュール `getcurve` がカーブを評価する。

mod curve;
mod ipc;
mod module;
mod trackbar;
mod window;

use aviutl2::AnyResult;
use curve::Curve;

#[aviutl2::plugin(GenericPlugin)]
pub struct HlEzingPlugin {
    window: window::HlEzingWindow,
    module: aviutl2::generic::SubPlugin<module::HlEzingModule>,
}

impl aviutl2::generic::GenericPlugin for HlEzingPlugin {
    fn new(info: aviutl2::AviUtl2Info) -> AnyResult<Self> {
        Self::init_logging();
        tracing::info!("Initializing HL_Ezing...");
        trackbar::set_version(info.version.0);
        if trackbar::is_supported() {
            tracing::info!(
                "Trackbar drag/drop is available (AviUtl2 version {}).",
                info.version.0
            );
        } else {
            tracing::warn!(
                "Trackbar drag/drop is unavailable for AviUtl2 version {} (profile/RTTI mismatch).",
                info.version.0
            );
        }
        let window = window::HlEzingWindow::create()?;
        let module =
            aviutl2::generic::SubPlugin::<module::HlEzingModule>::new_script_module(&info)?;
        Ok(HlEzingPlugin { window, module })
    }

    fn plugin_info(&self) -> aviutl2::generic::GenericPluginTable {
        aviutl2::generic::GenericPluginTable {
            name: "HL_Ezing".to_string(),
            information: format!(
                "HL_Ezing v{} by halkun19 - AviUtl2 easing curve editor",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn register(&mut self, registry: &mut aviutl2::generic::HostAppHandle) {
        if let Err(e) = registry.register_window_client("HL_Ezing", &self.window) {
            tracing::error!("Failed to register window client: {e:?}");
        }
        registry.register_script_module(Some(curve::apply::SCRIPT_MODULE_NAME), &self.module);
        ipc::EDIT_HANDLE.init(registry.create_edit_handle());
        registry.register_object_item_menu(
            "HL_Ezing\\このトラックバーに最後のカーブを適用",
            |object, effect, effect_index, item| {
                let Some(curve_id) = curve::registry::last_applied() else {
                    tracing::warn!("No curve has been applied from HL_Ezing yet.");
                    return;
                };
                match ipc::EDIT_HANDLE.call_edit_section(|edit| {
                    curve::apply::apply_to_item(edit, object, effect, effect_index, item, curve_id)
                }) {
                    Ok(Ok(())) => {
                        tracing::info!("Applied curve {} to {}/{}.", curve_id, effect, item)
                    }
                    Ok(Err(error)) => tracing::warn!("Failed to apply curve: {}", error),
                    Err(error) => tracing::warn!("Failed to edit track: {}", error),
                }
            },
        );
        tracing::info!("HL_Ezing registered.");
    }

    fn on_project_load(&mut self, project: &mut aviutl2::generic::ProjectFile) {
        trackbar::invalidate_cache();
        curve::registry::clear_last_applied();
        match project.deserialize::<Vec<(u64, Curve)>>(curve::apply::PROJECT_KEY) {
            Ok(entries) => {
                const MAX_CURVES: usize = 4096;
                if entries.len() > MAX_CURVES {
                    tracing::warn!("Too many curves in project: {}", entries.len());
                    curve::registry::clear();
                } else {
                    let entries = entries
                        .into_iter()
                        .map(|(id, curve)| (id, curve.sanitized()))
                        .collect();
                    curve::registry::load(entries);
                }
                tracing::info!(
                    "Loaded {} curves from project.",
                    curve::registry::snapshot().len()
                );
            }
            Err(e) => {
                curve::registry::clear();
                tracing::info!("No curves in project ({e}).");
            }
        }

        match project.deserialize::<Vec<(u64, Vec<u64>)>>(curve::apply::ASSIGNMENTS_KEY) {
            Ok(entries) => {
                const MAX_ASSIGNMENTS: usize = 100_000;
                if entries.len() > MAX_ASSIGNMENTS {
                    tracing::warn!("Too many segment assignments in project: {}", entries.len());
                    curve::registry::load_assignments(Vec::new());
                } else {
                    let entries = entries
                        .into_iter()
                        .map(|(id, mut ids)| {
                            ids.truncate(256);
                            (id, ids)
                        })
                        .collect();
                    curve::registry::load_assignments(entries);
                }
                tracing::info!(
                    "Loaded {} segment assignments from project.",
                    curve::registry::assignment_snapshot().len()
                );
            }
            Err(e) => {
                curve::registry::load_assignments(Vec::new());
                tracing::info!("No segment assignments in project ({e}).");
            }
        }

        // 旧バージョンでは保存していないため、無い場合は「最後のカーブなし」。
        if let Ok(id) = project.deserialize::<u64>(curve::apply::LAST_APPLIED_KEY)
            && curve::registry::contains(id)
        {
            curve::registry::set_last_applied(id);
        }

        window::evaluate(
            "window.hlEzing && window.hlEzing.onEvent({event:'projectLoaded'})".into(),
        );
    }

    fn on_project_save(&mut self, project: &mut aviutl2::generic::ProjectFile) {
        let entries = curve::registry::snapshot();
        if let Err(e) = project.serialize(curve::apply::PROJECT_KEY, &entries) {
            tracing::warn!("Failed to save curves to project: {e}");
        }
        let last_applied = curve::registry::last_applied().unwrap_or(u64::MAX);
        if let Err(e) = project.serialize(curve::apply::LAST_APPLIED_KEY, &last_applied) {
            tracing::warn!("Failed to save last applied curve: {e}");
        }
        let assignments = curve::registry::assignment_snapshot();
        if let Err(e) = project.serialize(curve::apply::ASSIGNMENTS_KEY, &assignments) {
            tracing::warn!("Failed to save segment assignments: {e}");
        }
    }

    fn on_clear_cache(&mut self, _edit_section: &aviutl2::generic::EditSection) {
        trackbar::invalidate_cache();
        // カーブデータはプロジェクト内に保持しており、キャッシュ破棄の影響は受けない。
        tracing::debug!("Cache cleared.");
    }

    // 区間バーの更新はポーリングせず、AviUtl2 のイベントで通知する。
    // イベントスレッドでは EditHandle を呼べないため、WebView へ
    // 「更新が必要」とだけ伝え、実際の取得は JS 側から IPC で行う。
    fn event_change_edit_frame(&mut self) {
        notify_segments_dirty();
    }

    fn event_change_focus_object(&mut self) {
        trackbar::invalidate_cache();
        notify_object_changed();
        notify_segments_dirty();
    }

    fn event_update_object_info(&mut self) {
        trackbar::invalidate_cache();
        notify_object_changed();
        notify_segments_dirty();
    }

    fn event_change_scene_info(&mut self) {
        trackbar::invalidate_cache();
        notify_object_changed();
        notify_segments_dirty();
    }
}

fn notify_segments_dirty() {
    window::evaluate("window.hlEzing && window.hlEzing.onEvent({event:'segments_dirty'})".into());
}

fn notify_object_changed() {
    window::evaluate("window.hlEzing && window.hlEzing.onEvent({event:'object_changed'})".into());
}

impl HlEzingPlugin {
    fn init_logging() {
        let _ = aviutl2::tracing_subscriber::fmt()
            .with_max_level(if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            })
            .event_format(aviutl2::logger::AviUtl2Formatter)
            .with_writer(aviutl2::logger::AviUtl2LogWriter)
            .try_init();
    }
}

#[aviutl2::generic::menus]
impl HlEzingPlugin {
    #[object(name = "HL_Ezing\\最後に適用したカーブを再適用")]
    fn menu_reapply(&mut self) -> AnyResult<()> {
        let Some(curve_id) = curve::registry::last_applied() else {
            anyhow::bail!("HL_Ezingで適用したカーブがありません。");
        };
        if !ipc::EDIT_HANDLE.is_ready() {
            anyhow::bail!("編集セクションの初期化が完了していません。");
        }
        let result = ipc::EDIT_HANDLE
            .call_edit_section(|edit| curve::apply::apply_to_selection(edit, curve_id))?
            .map_err(aviutl2::anyhow::Error::msg)?;
        tracing::info!("Reapplied curve {} to {} tracks.", curve_id, result);
        Ok(())
    }

    #[object(name = "HL_Ezing\\ドラッグ適用の診断情報をログ出力")]
    fn menu_debug_trackbars(&mut self) -> AnyResult<()> {
        trackbar::log_debug_snapshot();
        Ok(())
    }
}

aviutl2::register_generic_plugin!(HlEzingPlugin);
