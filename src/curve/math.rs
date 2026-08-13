//! カーブ評価の数学。
//!
//! AE版HL_Ezing2 (original_backup/js/main.js) の `cppElastic` / `cppBounce` と
//! サムネイル描画のベジェ評価をRustに移植したもの。
//! 実行時評価はこのRust実装を単一ソースとして使う（.tra2 → スクリプトモジュール経由）。

use super::{BezierData, BezierPoint};

/// 3次ベジェの座標を評価する。
pub fn cubic_bezier(t: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    let u = 1.0 - t;
    let tt = t * t;
    let uu = u * u;
    let uuu = uu * u;
    let ttt = tt * t;
    uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u * tt * p2 + ttt * p3
}

/// 3次ベジェのx=tとなるtをニュートン法で求める（xは0..1に正規化済み）。
/// ハンドルが端点近くに固まっているとニュートン法が収束しないことがあるため、
/// 収束しない場合は二分法で厳密に解く。
fn solve_cubic_bezier_t(x: f64, c1x: f64, c2x: f64) -> f64 {
    let mut t = x;
    for _ in 0..8 {
        let e = cubic_bezier(t, 0.0, c1x, c2x, 1.0) - x;
        if e.abs() < 1e-6 {
            return t.clamp(0.0, 1.0);
        }
        let derivative =
            (3.0 * c1x - 6.0 * c2x + 3.0) * t * t + (-6.0 * c1x + 6.0 * c2x) * t + 3.0 * c1x;
        if derivative.abs() < 1e-6 {
            break;
        }
        t -= e / derivative;
    }
    if (cubic_bezier(t, 0.0, c1x, c2x, 1.0) - x).abs() < 1e-6 {
        return t.clamp(0.0, 1.0);
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..60 {
        let mid = (lo + hi) * 0.5;
        if cubic_bezier(mid, 0.0, c1x, c2x, 1.0) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// 複数アンカーのベジェカーブを評価する。
///
/// セグメントはアンカーのx座標で区間を判定し、セグメント種別(bezier/elastic/bounce)に応じて
/// 評価する。yはアンカーの絶対座標で補間する（original_backupのサムネイル描画と同一の挙動）。
/// 始点・終点のアンカーはYを上下に動かせるため、端ではそのアンカーのYを返す。
pub fn evaluate_bezier(data: &BezierData, t: f64) -> f64 {
    let points = &data.points;
    if points.len() < 2 {
        return t;
    }
    let first = &points[0];
    let last = &points[points.len() - 1];
    if t <= first.anchor[0] {
        return first.anchor[1];
    }
    if t >= last.anchor[0] {
        return last.anchor[1];
    }
    for i in 0..points.len() - 1 {
        let p_start = &points[i];
        let p_end = &points[i + 1];
        if t >= p_start.anchor[0] && t <= p_end.anchor[0] {
            return evaluate_segment(p_start, p_end, t);
        }
    }
    last.anchor[1]
}

fn evaluate_segment(p_start: &BezierPoint, p_end: &BezierPoint, t: f64) -> f64 {
    let segment_duration = p_end.anchor[0] - p_start.anchor[0];
    if segment_duration.abs() < 1e-6 {
        return p_start.anchor[1];
    }
    let local_t = (t - p_start.anchor[0]) / segment_duration;
    let seg_type = p_start.out_type.as_str();
    match seg_type {
        "elastic" => {
            if let Some(p) = &p_start.out_elastic {
                p_start.anchor[1]
                    + elastic(local_t, p.amplitude, p.frequency, p.decay, p.reversed)
                        * (p_end.anchor[1] - p_start.anchor[1])
            } else {
                p_start.anchor[1] + local_t * (p_end.anchor[1] - p_start.anchor[1])
            }
        }
        "bounce" => {
            if let Some(p) = &p_start.out_bounce {
                p_start.anchor[1]
                    + bounce(local_t, p.cor, p.period, p.reversed)
                        * (p_end.anchor[1] - p_start.anchor[1])
            } else {
                p_start.anchor[1] + local_t * (p_end.anchor[1] - p_start.anchor[1])
            }
        }
        _ => {
            let x1 = (p_start.out_handle[0] - p_start.anchor[0]) / segment_duration;
            let x2 = (p_end.in_handle[0] - p_start.anchor[0]) / segment_duration;
            let t_for_x = solve_cubic_bezier_t(local_t, x1, x2);
            cubic_bezier(
                t_for_x,
                p_start.anchor[1],
                p_start.out_handle[1],
                p_end.in_handle[1],
                p_end.anchor[1],
            )
        }
    }
}

/// 弾性カーブ。AE版HL_Ezing2の`cppElastic`の移植。
pub fn elastic(p: f64, amplitude: f64, frequency: f64, decay: f64, reversed: bool) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let mut p = p;
    if reversed {
        p = 1.0 - p;
    }
    let w = 2.0 * std::f64::consts::PI * frequency;
    let ek = (-decay).exp();

    let d1 = |p: f64, d: f64| {
        let an = w * p;
        let cs = an.cos();
        let sn = an.sin();
        if d == 0.0 {
            return cs + w * (1.0 - p) * sn;
        }
        let ekt = ek.powf(p);
        d * ekt * cs + w * (ekt - ek) * sn
    };
    let d2 = |p: f64, d: f64| {
        let an = w * p;
        let cs = an.cos();
        let sn = an.sin();
        let w2 = w * w;
        if d == 0.0 {
            return w2 * (1.0 - p) * cs - 2.0 * w * sn;
        }
        let ekt = ek.powf(p);
        ((w2 - d * d) * ekt - w2 * ek) * cs - 2.0 * w * d * ekt * sn
    };
    let fn_ = |p: f64, d: f64| {
        if p <= 0.0 {
            return 0.0;
        }
        if p >= 1.0 {
            return 1.0;
        }
        let c = if d == 0.0 {
            1.0 - p
        } else {
            let t = (-d).exp();
            (t.powf(p) - t) / (1.0 - t)
        };
        1.0 - c * (2.0 * std::f64::consts::PI * frequency * p).cos()
    };

    let mut et = (0.5 - (decay / frequency).sqrt() * 0.05) / frequency;
    for _ in 0..3 {
        et -= d1(et, decay) / d2(et, decay);
    }
    let ex = fn_(et, decay);
    let tmp = fn_(p, decay);
    let ret = if p < et {
        (amplitude * (ex - 1.0) + 1.0) / ex * tmp
    } else {
        amplitude * (tmp - 1.0) + 1.0
    };
    if reversed { 1.0 - ret } else { ret }
}

/// バウンスカーブ。AE版HL_Ezing2の`cppBounce`の移植。
pub fn bounce(p: f64, cor: f64, period: f64, reversed: bool) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    // cor=1.0(完全弾性)は式が退化するため上限を詰める。
    let c = cor.clamp(0.0, 0.999999);
    let d = period.max(0.0001);
    let mut p = p;
    if reversed {
        p = 1.0 - p;
    }
    let l = d * (1.0 / (1.0 - c) - 0.5);
    let f1 = |pr: f64, co: f64| (((co - 1.0) * pr + 1.0).ln() / co.ln()).floor();
    let f2 = |pr: f64, co: f64| {
        pr + 0.5 + 1.0 / (co - 1.0) - (co + 1.0) * co.powf(f1(pr + 0.5, co)) / (2.0 * co - 2.0)
    };
    let t = 4.0 * f2(p / d, c) * f2(p / d, c) - c.powf(2.0 * f1(p / d + 0.5, c));
    let ret = if l > 1.0 {
        let n = ((1.0 + (c - 1.0) * (1.0 / d + 0.5)).ln() / c.ln()).floor();
        if p < d * ((c.powf(n) - 1.0) / (c - 1.0) - 0.5) {
            t
        } else {
            0.0
        }
    } else if p < l {
        t
    } else {
        0.0
    };
    if reversed {
        1.0 - (1.0 + ret)
    } else {
        1.0 + ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{default_bezier, default_bounce, default_elastic};

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn elastic_endpoints() {
        let d = default_elastic();
        assert!(approx(
            elastic(0.0, d.amplitude, d.frequency, d.decay, false),
            0.0
        ));
        assert!(approx(
            elastic(1.0, d.amplitude, d.frequency, d.decay, false),
            1.0
        ));
        // 反転の対称性
        let v = elastic(0.3, d.amplitude, d.frequency, d.decay, false);
        let r = elastic(0.7, d.amplitude, d.frequency, d.decay, true);
        assert!((v - (1.0 - r)).abs() < 1e-6);
    }

    #[test]
    fn bounce_endpoints() {
        let d = default_bounce();
        assert!(approx(bounce(0.0, d.cor, d.period, false), 0.0));
        assert!(approx(bounce(1.0, d.cor, d.period, false), 1.0));
    }

    #[test]
    fn bezier_linear_is_linear() {
        // ハンドルを比例位置(y=0.33, 0.67)にすると直線になる
        let data = super::BezierData {
            points: vec![
                super::BezierPoint {
                    anchor: [0.0, 0.0],
                    in_handle: [0.0, 0.0],
                    out_handle: [1.0 / 3.0, 1.0 / 3.0],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
                super::BezierPoint {
                    anchor: [1.0, 1.0],
                    in_handle: [2.0 / 3.0, 2.0 / 3.0],
                    out_handle: [1.0, 1.0],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
            ],
        };
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            assert!((evaluate_bezier(&data, t) - t).abs() < 1e-6, "t={t}");
        }
    }

    #[test]
    fn bezier_default_is_ease_in_out() {
        let data = default_bezier();
        assert!((evaluate_bezier(&data, 0.0) - 0.0).abs() < 1e-9);
        assert!((evaluate_bezier(&data, 1.0) - 1.0).abs() < 1e-9);
        assert!((evaluate_bezier(&data, 0.5) - 0.5).abs() < 1e-9);
        // 中央は直線(0.5)より遅く進む(ease-in-out)
        assert!(evaluate_bezier(&data, 0.25) < 0.25);
        assert!(evaluate_bezier(&data, 0.75) > 0.75);
    }

    #[test]
    fn cubic_bezier_basic() {
        assert!(approx(cubic_bezier(0.5, 0.0, 0.0, 1.0, 1.0), 0.5));
        assert!(approx(cubic_bezier(0.0, 0.0, 0.0, 1.0, 1.0), 0.0));
        assert!(approx(cubic_bezier(1.0, 0.0, 0.0, 1.0, 1.0), 1.0));
    }

    #[test]
    fn bezier_endpoints_moveable_vertical() {
        // 始点Y=0.3, 終点Y=0.9 の直線カーブ。端ではそのYを返す
        let data = BezierData {
            points: vec![
                BezierPoint {
                    anchor: [0.0, 0.3],
                    in_handle: [0.0, 0.3],
                    out_handle: [1.0 / 3.0, 0.5],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
                BezierPoint {
                    anchor: [1.0, 0.9],
                    in_handle: [2.0 / 3.0, 0.7],
                    out_handle: [1.0, 0.9],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
            ],
        };
        assert!(approx(evaluate_bezier(&data, 0.0), 0.3));
        assert!(approx(evaluate_bezier(&data, 1.0), 0.9));
        // 直線(比例ハンドル)なら中間も端点間を線形に補間
        assert!((evaluate_bezier(&data, 0.5) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn bezier_bunched_handles_no_jump() {
        // 始点・始点のハンドル・次の点のハンドルが同じY(=0)で、両ハンドルが
        // Xも端点近く(0.99)に固まっているケース。旧ニュートン法は収束せず
        // カーブが跳ね上がっていたが、二分法フォールバックにより滑らかになる。
        let data = BezierData {
            points: vec![
                BezierPoint {
                    anchor: [0.0, 0.0],
                    in_handle: [0.0, 0.0],
                    out_handle: [0.99, 0.0],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
                BezierPoint {
                    anchor: [1.0, 1.0],
                    in_handle: [0.99, 0.0],
                    out_handle: [1.0, 1.0],
                    out_type: "bezier".into(),
                    out_elastic: None,
                    out_bounce: None,
                },
            ],
        };
        // 全域で単調増加(跳ね上がりなし)
        let mut prev = -1.0;
        for i in 0..=200 {
            let t = i as f64 / 200.0;
            let y = evaluate_bezier(&data, t);
            assert!(y >= prev - 1e-9, "not monotonic at t={t}: {y} < {prev}");
            prev = y;
        }
        // 参照値(二分法で厳密解): x=0.9 で約0.16、旧実装は0.31前後まで跳ね上がっていた
        let v = evaluate_bezier(&data, 0.9);
        assert!((v - 0.164).abs() < 0.01, "unexpected jump: y(0.9)={v}");
    }
}
