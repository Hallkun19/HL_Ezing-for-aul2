//! カーブのデータモデル。
//!
//! AE版HL_Ezing2 (original_backup) のJSが送るJSON構造と互換になるように定義する。
//! JS側からは `{mode, data}` のIPCコマンドで送られ、実行時評価はRust側(スクリプトモジュール)で行う。

use serde::{Deserialize, Serialize};

pub mod apply;
pub mod math;
pub mod registry;
pub mod track_value;

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

fn default_amplitude() -> f64 {
    0.5
}

fn default_frequency() -> f64 {
    2.0
}

fn default_decay() -> f64 {
    5.0
}

fn default_cor() -> f64 {
    0.5
}

fn default_period() -> f64 {
    0.5
}

/// 弾性カーブのパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticParams {
    #[serde(default = "default_amplitude")]
    pub amplitude: f64,
    #[serde(default = "default_frequency")]
    pub frequency: f64,
    #[serde(default = "default_decay")]
    pub decay: f64,
    #[serde(default)]
    pub reversed: bool,
}

/// 弾性カーブのデータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticData {
    #[serde(default = "default_amplitude")]
    pub amplitude: f64,
    #[serde(default = "default_frequency")]
    pub frequency: f64,
    #[serde(default = "default_decay")]
    pub decay: f64,
    #[serde(default)]
    pub reversed: bool,
}

/// バウンスカーブのパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BounceParams {
    #[serde(default = "default_cor")]
    pub cor: f64,
    #[serde(default = "default_period")]
    pub period: f64,
    #[serde(default)]
    pub reversed: bool,
}

/// バウンスカーブのデータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BounceData {
    #[serde(default = "default_cor")]
    pub cor: f64,
    #[serde(default = "default_period")]
    pub period: f64,
    #[serde(default)]
    pub reversed: bool,
}

/// デフォルトの振動パラメータ。
pub fn default_elastic_params() -> ElasticParams {
    ElasticParams {
        amplitude: 0.5,
        frequency: 2.0,
        decay: 5.0,
        reversed: false,
    }
}

/// デフォルトのバウンスパラメータ。
pub fn default_bounce_params() -> BounceParams {
    BounceParams {
        cor: 0.5,
        period: 0.5,
        reversed: false,
    }
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

    /// IPC・プロジェクトから読み込んだ値を安全な範囲に正規化する。
    ///
    /// 外部データに NaN、非昇順のアンカー、範囲外のハンドル、0除算を起こす
    /// パラメータが含まれていても、評価時に破綻しないようにする。
    pub fn sanitized(self) -> Self {
        match self {
            Curve::Bezier(data) => Curve::Bezier(sanitize_bezier(data)),
            Curve::Elastic(data) => Curve::Elastic(ElasticData {
                amplitude: finite_or(data.amplitude, 0.5).clamp(0.0, 1.0),
                frequency: finite_or(data.frequency, 2.0).clamp(0.001, 1000.0),
                decay: finite_or(data.decay, 5.0).clamp(0.0, 100.0),
                reversed: data.reversed,
            }),
            Curve::Bounce(data) => Curve::Bounce(BounceData {
                cor: finite_or(data.cor, 0.5).clamp(0.000001, 0.999999),
                period: finite_or(data.period, 0.5).clamp(0.0001, 100.0),
                reversed: data.reversed,
            }),
        }
    }
}

/// デフォルトのベジェカーブ(直線ではなくイーズイン・アウト)を生成する。
pub fn default_bezier() -> BezierData {
    BezierData {
        points: vec![
            BezierPoint {
                anchor: [0.0, 0.0],
                in_handle: [0.0, 0.0],
                out_handle: [0.33, 0.0],
                out_type: "bezier".into(),
                out_elastic: Some(default_elastic_params()),
                out_bounce: Some(default_bounce_params()),
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

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

fn sanitize_bezier(mut data: BezierData) -> BezierData {
    // 極端に多い点は描画・評価の負荷になるため上限を設ける。
    data.points.truncate(256);

    for point in &mut data.points {
        point.anchor[0] = finite_or(point.anchor[0], 0.0).clamp(0.0, 1.0);
        point.anchor[1] = finite_or(point.anchor[1], 0.0).clamp(-10.0, 10.0);
        for handle in [&mut point.in_handle, &mut point.out_handle] {
            handle[0] = finite_or(handle[0], point.anchor[0]);
            handle[1] = finite_or(handle[1], point.anchor[1]);
        }
        if !matches!(point.out_type.as_str(), "bezier" | "elastic" | "bounce") {
            point.out_type = "bezier".into();
        }
        point.out_elastic = point
            .out_elastic
            .take()
            .map(sanitize_elastic_params)
            .or_else(|| (point.out_type == "elastic").then(default_elastic_params));
        point.out_bounce = point
            .out_bounce
            .take()
            .map(sanitize_bounce_params)
            .or_else(|| (point.out_type == "bounce").then(default_bounce_params));
    }

    data.points
        .sort_by(|left, right| left.anchor[0].total_cmp(&right.anchor[0]));

    if data.points.len() < 2 {
        return default_bezier();
    }

    let last_index = data.points.len() - 1;
    let min_gap = 0.001_f64.min(0.5 / last_index as f64);
    for index in 0..data.points.len() {
        let lower = if index == 0 {
            0.0
        } else {
            data.points[index - 1].anchor[0] + min_gap
        };
        let upper = if index == last_index {
            1.0
        } else {
            1.0 - (last_index - index) as f64 * min_gap
        };
        data.points[index].anchor[0] = if index == 0 {
            0.0
        } else if index == last_index {
            1.0
        } else {
            data.points[index].anchor[0].clamp(lower.min(upper), upper.max(lower))
        };
    }

    for index in 0..data.points.len() {
        let prev_x = if index == 0 {
            0.0
        } else {
            data.points[index - 1].anchor[0]
        };
        let anchor_x = data.points[index].anchor[0];
        let next_x = if index == last_index {
            1.0
        } else {
            data.points[index + 1].anchor[0]
        };
        let anchor_y = data.points[index].anchor[1];
        let point = &mut data.points[index];
        point.in_handle[0] = point.in_handle[0].clamp(prev_x, anchor_x);
        point.out_handle[0] = point.out_handle[0].clamp(anchor_x, next_x);
        point.in_handle[1] = point.in_handle[1].clamp(-10.0, 10.0);
        point.out_handle[1] = point.out_handle[1].clamp(-10.0, 10.0);
        // ハンドルが非有限だった場合はアンカー位置に戻す。
        if !point.in_handle[0].is_finite() || !point.in_handle[1].is_finite() {
            point.in_handle = [anchor_x, anchor_y];
        }
        if !point.out_handle[0].is_finite() || !point.out_handle[1].is_finite() {
            point.out_handle = [anchor_x, anchor_y];
        }
    }

    data
}

fn sanitize_elastic_params(params: ElasticParams) -> ElasticParams {
    ElasticParams {
        amplitude: finite_or(params.amplitude, 0.5).clamp(0.0, 1.0),
        frequency: finite_or(params.frequency, 2.0).clamp(0.001, 1000.0),
        decay: finite_or(params.decay, 5.0).clamp(0.0, 100.0),
        reversed: params.reversed,
    }
}

fn sanitize_bounce_params(params: BounceParams) -> BounceParams {
    BounceParams {
        cor: finite_or(params.cor, 0.5).clamp(0.000001, 0.999999),
        period: finite_or(params.period, 0.5).clamp(0.0001, 100.0),
        reversed: params.reversed,
    }
}

#[cfg(test)]
pub fn default_elastic() -> ElasticData {
    ElasticData {
        amplitude: default_elastic_params().amplitude,
        frequency: default_elastic_params().frequency,
        decay: default_elastic_params().decay,
        reversed: default_elastic_params().reversed,
    }
}

#[cfg(test)]
pub fn default_bounce() -> BounceData {
    BounceData {
        cor: default_bounce_params().cor,
        period: default_bounce_params().period,
        reversed: default_bounce_params().reversed,
    }
}
