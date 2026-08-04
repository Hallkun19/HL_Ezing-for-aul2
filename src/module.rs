//! スクリプトモジュール。
//!
//! `.tra2` トラックバースクリプトから `obj.module("HL_Ezing.aux2").getcurve(id, nonce, t)` で
//! 呼ばれるカーブ評価関数を提供する。実行時評価の単一ソース。

use aviutl2::module::ScriptModuleFunctions;

#[aviutl2::plugin(ScriptModule)]
pub(crate) struct HlEzingModule;

impl aviutl2::module::ScriptModule for HlEzingModule {
    fn new(_info: aviutl2::AviUtl2Info) -> aviutl2::AnyResult<Self> {
        Ok(HlEzingModule)
    }

    fn plugin_info(&self) -> aviutl2::module::ScriptModuleTable {
        aviutl2::module::ScriptModuleTable {
            information: format!("HL_Ezing curve evaluator v{}", env!("CARGO_PKG_VERSION")),
            functions: Self::functions(),
        }
    }
}

#[aviutl2::module::functions]
impl HlEzingModule {
    /// カーブIDで登録済みカーブを評価し、tに対するy(0..1, オーバーシュート可)を返す。
    /// 見つからない場合は線形(t)を返す。
    fn getcurve(&self, curve_id: f64, _nonce: f64, t: f64) -> aviutl2::AnyResult<f64> {
        let id = curve_id as u64;
        Ok(crate::curve::registry::get(id)
            .map(|c| c.evaluate(t))
            .unwrap_or(t))
    }
}
