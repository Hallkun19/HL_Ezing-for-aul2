//! カーブのデータモデル。
//!
//! AE版HL_Ezing2 (original_backup) のJSが送るJSON構造と互換になるように定義する。
//! JS側からは `{mode, data}` のIPCコマンドで送られ、実行時評価はRust側(スクリプトモジュール)で行う。

use serde::{Deserialize, Serialize};

pub mod apply;
pub mod math;
pub mod registry;

/// ベジェカーブの1点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BezierPoint {
    pub anchor: [f64; 2],
    #[serde(rename = "inHandle")]
    pub in_handle: [f64; 2],
    #[serde(rename = "outHandle")]
    pub out_handle: [f64; 2],
    /// セグメント種別。`bezier` | `elastic` | `bounce`。
    #[serde(rename = "outType", default)]
    pub out_type: String,
    #[serde(
        rename = "outElasticParams",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub out_elastic: Option<ElasticParams>,
    #[serde(
        rename = "outBounceParams",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub out_bounce: Option<BounceParams>,
}

/// ベジェ(複数アンカー)のデータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BezierData {
    pub points: Vec<BezierPoint>,
}

/// 弾性カーブのパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticParams {
    pub amplitude: f64,
    pub frequency: f64,
    pub decay: f64,
    pub reversed: bool,
}

/// 弾性カーブのデータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticData {
    pub amplitude: f64,
    pub frequency: f64,
    pub decay: f64,
    pub reversed: bool,
}

/// バウンスカーブのパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BounceParams {
    pub cor: f64,
    pub period: f64,
    pub reversed: bool,
}

/// バウンスカーブのデータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BounceData {
    pub cor: f64,
    pub period: f64,
    pub reversed: bool,
}

/// カーブ全体。プロジェクト保存用にmodeタグ付きでシリアライズする。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum Curve {
    #[serde(rename = "bezier")]
    Bezier(BezierData),
    #[serde(rename = "elastic")]
    Elastic(ElasticData),
    #[serde(rename = "bounce")]
    Bounce(BounceData),
}

impl Curve {
    /// t(0.0〜1.0)に対するy(0.0〜1.0、オーバーシュート可)を評価する。
    pub fn evaluate(&self, t: f64) -> f64 {
        match self {
            Curve::Bezier(data) => math::evaluate_bezier(data, t),
            Curve::Elastic(data) => {
                math::elastic(t, data.amplitude, data.frequency, data.decay, data.reversed)
            }
            Curve::Bounce(data) => math::bounce(t, data.cor, data.period, data.reversed),
        }
    }
}

/// デフォルトのベジェカーブ(直線)を生成する。
#[cfg(test)]
pub fn default_bezier() -> BezierData {
    BezierData {
        points: vec![
            BezierPoint {
                anchor: [0.0, 0.0],
                in_handle: [0.0, 0.0],
                out_handle: [0.33, 0.0],
                out_type: "bezier".into(),
                out_elastic: None,
                out_bounce: None,
            },
            BezierPoint {
                anchor: [1.0, 1.0],
                in_handle: [0.67, 1.0],
                out_handle: [1.0, 1.0],
                out_type: "bezier".into(),
                out_elastic: None,
                out_bounce: None,
            },
        ],
    }
}

#[cfg(test)]
pub fn default_elastic() -> ElasticData {
    ElasticData {
        amplitude: 0.5,
        frequency: 2.0,
        decay: 5.0,
        reversed: false,
    }
}

#[cfg(test)]
pub fn default_bounce() -> BounceData {
    BounceData {
        cor: 0.5,
        period: 0.5,
        reversed: false,
    }
}
