//! トラックバー値文字列の読み書き。
//!
//! AviUtl2 のエイリアス形式トラックバー値は次のような形式になっている。
//!
//! ```text
//! 0.00,100.00,HL_Ezing,0|3,17
//! 0.00,50.00,100.00,HL_Ezing,0|3,17
//! ```
//!
//! 先頭のカンマ区切りが開始点・中間点・終了点の値、その次の2つが移動方法名と
//! フラグ、`|` 以降がスクリプトパラメータ（存在する場合）と時間制御データ。
//!
//! この実装は [aviutl2_track_parser](https://github.com/sevenc-nanashi/aviutl2_track_parser)
//! (MIT License, Copyright (c) sevenc-nanashi) の解析方針を参考に、
//! 本プラグインで必要な範囲へ絞って実装したもの。
//!
//! 重要な点として、既存の値をすべて保持したまま移動方法だけを差し替える。
//! これにより中間点を壊さず、各区間に対してカーブを適用できる。

/// 「参照式」フラグ。
pub const FLAG_REFERENCE: u32 = 0b0000_1000;
/// 「中間点無視」フラグ。
pub const FLAG_TWOPOINT: u32 = 0b0000_0100;
/// 「中間点と制御点をリンク」フラグ。
pub const FLAG_LINK_MIDPOINTS_AND_CONTROL_POINTS: u32 = 0b0001_0000;

/// トラックバーの移動方法。
#[derive(Debug, Clone, PartialEq)]
pub struct Movement {
    pub name: String,
    pub parameters: Vec<f64>,
}

/// トラックバーの値。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackValue {
    /// 開始点・中間点・終了点の値。
    pub values: Vec<f64>,
    /// 小数部の桁数。
    pub precision: usize,
    /// トラックバーのフラグ。
    pub flag: u32,
    /// 移動方法。`None` は「移動無し」。
    pub movement: Option<Movement>,
    /// 時間制御データの生文字列。適用時は通常 `None` にする。
    pub time_control: Option<String>,
    /// 参照式。
    pub reference: Option<String>,
}

impl TrackValue {
    /// 区間ごとのカーブIDリストを返す。
    ///
    /// 新形式: `[0, id0, id1, ..., nonce]`
    /// 旧形式: `[id, nonce]`
    ///
    /// 旧形式や未適用のトラックでは、足りない区間を `default_id` または
    /// 直前のIDで埋める。区間数は呼び出し側が値の点数から決める。
    pub fn segment_curve_ids(
        &self,
        mode_name: &str,
        segment_count: usize,
        default_id: u64,
    ) -> Vec<u64> {
        let count = segment_count.max(1);
        let mut ids = Vec::new();
        if let Some(movement) = &self.movement
            && movement.name == mode_name
        {
            let params = &movement.parameters;
            if params.first().copied() == Some(0.0) && params.len() >= 3 {
                ids = params[1..params.len() - 1]
                    .iter()
                    .map(|value| *value as u64)
                    .filter(|id| *id > 0)
                    .collect();
            } else if let Some(id) = params.first().map(|value| *value as u64)
                && id > 0
            {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            ids.push(default_id);
        }
        let fill = *ids.last().unwrap_or(&default_id);
        while ids.len() < count {
            ids.push(fill);
        }
        ids.truncate(count);
        ids
    }

    /// 値文字列を解析する。
    ///
    /// `info` は `EditSection::get_effect_track_info` の結果。
    /// 移動無しのトラックバーでは `None` になるため、その場合も解析できる。
    pub fn parse(track: &str, info: Option<&aviutl2::generic::TrackInfo>) -> Result<Self, String> {
        if track.is_empty() {
            return Err("empty track value".into());
        }

        let sections: Vec<&str> = track.split('|').collect();
        let first: Vec<&str> = sections[0].split(',').collect();
        if first.is_empty() {
            return Err("empty track value".into());
        }

        // 移動無しで単一値の場合は最小形式。
        if first.len() == 1 {
            let value = parse_number(first[0])?;
            return Ok(Self {
                values: vec![value],
                precision: precision_of(first[0]),
                flag: 0,
                movement: None,
                time_control: None,
                reference: None,
            });
        }

        if first.len() < 3 {
            return Err(format!("invalid track value: {track}"));
        }

        let flag = first[first.len() - 1]
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("invalid track flag: {}", first[first.len() - 1]))?;
        let movement_name = first[first.len() - 2];
        let values = first[..first.len() - 2]
            .iter()
            .map(|value| parse_number(value))
            .collect::<Result<Vec<_>, _>>()?;
        if values.is_empty() {
            return Err("track value has no points".into());
        }

        let mut movement = if movement_name == "移動無し" {
            None
        } else {
            Some(Movement {
                name: movement_name.to_string(),
                parameters: info.map(|info| info.params.clone()).unwrap_or_default(),
            })
        };

        let mut section_index = 1;
        let reference = if flag & FLAG_REFERENCE != 0 {
            let reference = sections
                .get(section_index)
                .ok_or_else(|| "missing reference section".to_string())?
                .to_string();
            section_index += 1;
            Some(reference)
        } else {
            None
        };

        // スクリプトパラメータは TrackInfo.params が正だが、文字列側にしか
        // 存在しないケースも壊さないように残りセクション数からも判定する。
        if let Some(movement) = &mut movement {
            let timecontrol_sections =
                usize::from(info.map(|info| info.timecontrol).unwrap_or(false));
            let has_parameter_section = !info.map(|info| info.params.is_empty()).unwrap_or(true)
                || sections.len() > section_index + timecontrol_sections;
            if has_parameter_section
                && sections.len() > section_index
                && let Ok(parameters) = parse_parameters(sections[section_index])
            {
                movement.parameters = parameters;
                section_index += 1;
            }
        }

        let time_control = if info.map(|info| info.timecontrol).unwrap_or(false) {
            sections
                .get(section_index)
                .map(|section| (*section).to_string())
        } else {
            None
        };

        Ok(Self {
            values,
            precision: precision_of(first[0]),
            flag,
            movement,
            time_control,
            reference,
        })
    }

    /// 適用時に中間点を有効にするため、中間点無視関連のフラグを落とす。
    pub fn enable_midpoints(&mut self) {
        self.flag &= !FLAG_TWOPOINT;
        self.flag &= !FLAG_LINK_MIDPOINTS_AND_CONTROL_POINTS;
        self.time_control = None;
    }

    /// 値が1点しかない場合、開始点と終了点の2点に複製する。
    pub fn ensure_two_points(&mut self, start_value: f64, end_value: f64) {
        match self.values.len() {
            0 => self.values = vec![start_value, end_value],
            1 => {
                let value = self.values[0];
                self.values = vec![value, value];
            }
            _ => {}
        }
    }

    /// 移動方法を設定する。値・フラグ・精度は保持する。
    pub fn set_movement(&mut self, name: &str, parameters: Vec<f64>) {
        self.movement = Some(Movement {
            name: name.to_string(),
            parameters,
        });
    }
}

impl std::fmt::Display for TrackValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let values = self
            .values
            .iter()
            .map(|value| format_number(*value, self.precision))
            .collect::<Vec<_>>()
            .join(",");

        if self.movement.is_none()
            && self.values.len() == 1
            && self.flag == 0
            && self.time_control.is_none()
            && self.reference.is_none()
        {
            return f.write_str(&values);
        }

        let mut flag = self.flag;
        if self.reference.is_some() {
            flag |= FLAG_REFERENCE;
        } else {
            flag &= !FLAG_REFERENCE;
        }

        let movement_name = self
            .movement
            .as_ref()
            .map_or("移動無し", |movement| movement.name.as_str());
        write!(f, "{values},{movement_name},{flag}")?;

        if let Some(reference) = &self.reference {
            write!(f, "|{reference}")?;
        }
        if let Some(movement) = &self.movement
            && !movement.parameters.is_empty()
        {
            let parameters = movement
                .parameters
                .iter()
                .map(|value| serialize_float(*value))
                .collect::<Vec<_>>()
                .join(",");
            write!(f, "|{parameters}")?;
        }
        if let Some(time_control) = &self.time_control {
            write!(f, "|{time_control}")?;
        }
        Ok(())
    }
}

fn parse_number(value: &str) -> Result<f64, String> {
    let value = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("invalid track value: {value}"))?;
    if !value.is_finite() {
        return Err(format!("non-finite track value: {value}"));
    }
    Ok(value)
}

fn parse_parameters(value: &str) -> Result<Vec<f64>, String> {
    value
        .split(',')
        .filter(|parameter| !parameter.is_empty())
        .map(parse_number)
        .collect()
}

fn precision_of(value: &str) -> usize {
    value
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

fn format_number(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return format!("{:.precision$}", 0.0);
    }
    format!("{value:.precision$}")
}

fn serialize_float(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let value = format!("{value:.6}");
    let value = value.trim_end_matches('0').trim_end_matches('.');
    if value == "-0" || value.is_empty() {
        "0".into()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2::generic::TrackInfo;

    fn info(mode: &str, params: Vec<f64>, timecontrol: bool) -> TrackInfo {
        let mut info = TrackInfo::default();
        info.mode = Some(mode.to_string());
        info.params = params;
        info.timecontrol = timecontrol;
        info
    }

    #[test]
    fn parse_single_value_without_movement() {
        let track = TrackValue::parse("42.00", None).unwrap();
        assert_eq!(track.values, vec![42.0]);
        assert_eq!(track.precision, 2);
        assert!(track.movement.is_none());
        assert_eq!(track.to_string(), "42.00");
    }

    #[test]
    fn parse_midpoints_and_roundtrip() {
        let text = "0.00,50.00,100.00,HL_Ezing,0";
        let track =
            TrackValue::parse(text, Some(&info("HL_Ezing", vec![3.0, 17.0], false))).unwrap();
        assert_eq!(track.values, vec![0.0, 50.0, 100.0]);
        assert_eq!(track.movement.as_ref().unwrap().parameters, vec![3.0, 17.0]);
        // TrackInfo.params を正としてパラメータセクションを補完する。
        assert_eq!(track.to_string(), "0.00,50.00,100.00,HL_Ezing,0|3,17");
    }

    #[test]
    fn set_movement_preserves_midpoints() {
        let mut track = TrackValue::parse(
            "0.00,100.00,250.00,直線移動,0",
            Some(&info("直線移動", vec![], false)),
        )
        .unwrap();
        track.enable_midpoints();
        track.set_movement("HL_Ezing", vec![2.0, 9.0]);
        assert_eq!(track.values, vec![0.0, 100.0, 250.0]);
        assert_eq!(track.to_string(), "0.00,100.00,250.00,HL_Ezing,0|2,9");
    }

    #[test]
    fn parse_with_reference() {
        let text = "0.00,100.00,HL_Ezing,8|$|3,17";
        let track =
            TrackValue::parse(text, Some(&info("HL_Ezing", vec![3.0, 17.0], false))).unwrap();
        assert_eq!(track.reference.as_deref(), Some("$"));
        assert_eq!(track.to_string(), text);
    }

    #[test]
    fn parse_timecontrol_without_corrupting_it() {
        let text = "0.00,100.00,直線移動(時間制御),0|0.25,1,0.191176,1";
        let track =
            TrackValue::parse(text, Some(&info("直線移動(時間制御)", vec![], true))).unwrap();
        assert_eq!(track.time_control.as_deref(), Some("0.25,1,0.191176,1"));
        assert_eq!(track.to_string(), text);
    }

    #[test]
    fn segment_ids_from_old_single_curve_format() {
        let track = TrackValue::parse(
            "0.00,50.00,100.00,HL_Ezing,0",
            Some(&info("HL_Ezing", vec![3.0, 17.0], false)),
        )
        .unwrap();
        assert_eq!(track.segment_curve_ids("HL_Ezing", 3, 9), vec![3, 3, 3]);
    }

    #[test]
    fn segment_ids_from_new_list_format() {
        let track = TrackValue::parse(
            "0.00,50.00,100.00,HL_Ezing,0",
            Some(&info("HL_Ezing", vec![0.0, 3.0, 4.0, 17.0], false)),
        )
        .unwrap();
        assert_eq!(track.segment_curve_ids("HL_Ezing", 2, 9), vec![3, 4]);
        assert_eq!(track.segment_curve_ids("HL_Ezing", 4, 9), vec![3, 4, 4, 4]);
    }

    #[test]
    fn segment_ids_fall_back_to_default_for_other_movement() {
        let track = TrackValue::parse(
            "0.00,100.00,直線移動,0",
            Some(&info("直線移動", vec![], false)),
        )
        .unwrap();
        assert_eq!(track.segment_curve_ids("HL_Ezing", 2, 7), vec![7, 7]);
    }

    #[test]
    fn ensure_two_points_duplicates_single_value() {
        let mut track = TrackValue::parse("42.00", None).unwrap();
        track.ensure_two_points(42.0, 42.0);
        track.set_movement("HL_Ezing", vec![1.0, 2.0]);
        assert_eq!(track.to_string(), "42.00,42.00,HL_Ezing,0|1,2");
    }
}
