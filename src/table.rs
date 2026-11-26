use crate::core::{Row, MAX_BATCH, MAX_VALUE};

pub(crate) const MAX_COLUMNS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn from_csv(input: &str) -> Result<Self, String> {
        if input.len() > 2 * 1024 * 1024 {
            return Err("CSV 文件不得超过 2 MB。".into());
        }
        let mut reader = csv::Reader::from_reader(input.trim_start_matches('\u{feff}').as_bytes());
        let headers = reader
            .headers()
            .map_err(|e| format!("CSV 表头无法读取：{e}"))?
            .iter()
            .map(str::to_owned)
            .collect();
        let rows = reader
            .records()
            .take(MAX_BATCH + 1)
            .enumerate()
            .map(|(i, record)| {
                record
                    .map(|r| r.iter().map(str::to_owned).collect())
                    .map_err(|e| format!("CSV 第 {} 条数据无法读取：{e}", i + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_records(headers, rows)
    }

    pub fn from_records(headers: Vec<String>, rows: Vec<Vec<String>>) -> Result<Self, String> {
        if !(1..=MAX_COLUMNS).contains(&headers.len()) {
            return Err(format!("表格需要 1～{MAX_COLUMNS} 列。"));
        }
        let headers: Vec<_> = headers.into_iter().map(|h| h.trim().to_owned()).collect();
        if headers.iter().any(|h| h.len() > 128) {
            return Err("表头名称不得超过 128 字节。".into());
        }
        if rows.is_empty() {
            return Err("表格没有数据行。".into());
        }
        if rows.len() > MAX_BATCH {
            return Err(format!("单批最多 {MAX_BATCH} 条。"));
        }
        for (i, row) in rows.iter().enumerate() {
            if row.len() != headers.len() {
                return Err(format!("第 {} 条数据与表头列数不同。", i + 1));
            }
            if row.iter().any(|cell| cell.len() > MAX_VALUE) {
                return Err(format!("第 {} 条数据存在过长字段。", i + 1));
            }
        }
        Ok(Self { headers, rows })
    }

    pub fn match_columns(&self, fields: &[String]) -> Vec<Option<usize>> {
        let keys: Vec<_> = self.headers.iter().map(|s| column_key(s)).collect();
        let mut candidates = Vec::with_capacity(fields.len());
        for field in fields {
            let field_key = column_key(field);
            let mut best = 0;
            let mut column = None;
            for (i, key) in keys.iter().enumerate() {
                let score = if field == &self.headers[i] {
                    4
                } else if field_key == *key && !key.is_empty() {
                    3
                } else {
                    alias_score(&field_key, key)
                };
                if score > best {
                    best = score;
                    column = Some(i);
                } else if score == best {
                    column = None;
                }
            }
            candidates.push((best, column));
        }
        candidates
            .iter()
            .enumerate()
            .map(|(index, (score, column))| {
                // Even a unique alias is unsafe if two template fields claim that source.
                column.filter(|_| {
                    !candidates
                        .iter()
                        .enumerate()
                        .any(|(other, (s, c))| other != index && c == column && s >= score)
                })
            })
            .collect()
    }

    pub fn mapped_rows(
        &self,
        fields: &[String],
        columns: &[Option<usize>],
        fixed: &Row,
    ) -> Result<Vec<Row>, String> {
        if fields.len() != columns.len() {
            return Err("请重新选择模板字段对应的列。".into());
        }
        let mut bindings = Vec::new();
        let mut constants = Row::new();
        for (field, column) in fields.iter().zip(columns) {
            match column {
                Some(i) if *i < self.headers.len() => bindings.push((field, *i)),
                None if fixed.contains_key(field) => {
                    constants.insert(field.clone(), fixed[field].clone());
                }
                _ => return Err(format!("请为「{field}」选择表格列或填写固定值。")),
            }
        }
        Ok(self
            .rows
            .iter()
            .map(|row| {
                let mut values = constants.clone();
                for &(field, i) in &bindings {
                    values.insert(field.clone(), row[i].clone());
                }
                values
            })
            .collect())
    }
}

fn column_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn attribute(key: &str) -> &str {
    match key {
        "型号"
        | "元件型号"
        | "器件型号"
        | "物料型号"
        | "规格型号"
        | "制造商料号"
        | "制造商型号"
        | "mpn"
        | "model"
        | "partnumber"
        | "manufacturerpartnumber" => "型号",
        "封装" | "封装类型" | "封装规格" | "package" | "packagetype" | "footprint" => {
            "封装"
        }
        "名称" | "品名" | "元件名称" | "物料名称" | "name" | "partname" => "名称",
        "物料编码" | "物料编号" | "料号" | "itemnumber" | "itemcode" => "物料编码",
        "制造商" | "生产厂商" | "manufacturer" | "mfr" => "制造商",
        "品牌" | "brand" => "品牌",
        "备注" | "备注信息" | "note" | "notes" | "remark" | "remarks" => "备注",
        "阻值" | "电阻值" | "resistance" => "阻值",
        "容值" | "电容值" | "capacitance" => "容值",
        "数值" | "元件值" | "value" => "数值",
        "精度" | "公差" | "容差" | "误差" | "tolerance" => "精度",
        "耐压" | "额定电压" | "额定耐压" | "voltage" | "ratedvoltage" | "voltagerating" => {
            "耐压"
        }
        "功率" | "额定功率" | "power" | "ratedpower" | "powerrating" => "功率",
        "介质" | "电介质" | "dielectric" => "介质",
        "极性" | "polarity" => "极性",
        "沟道" | "沟道类型" | "channel" | "channeltype" => "沟道",
        "批次" | "批号" | "批次号" | "lot" | "lotnumber" | "batch" | "batchnumber" => "批次",
        _ => key,
    }
}

fn alias_score(field: &str, header: &str) -> u8 {
    let field = attribute(field);
    let header = attribute(header);
    if !field.is_empty() && field == header {
        2
    } else if matches!(
        (field, header),
        ("数值", "容值" | "阻值") | ("容值" | "阻值", "数值")
    ) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn csv_maps_aliases_in_template_order_and_preserves_values() {
        let table = Table::from_csv(
            "\u{feff} Foot_print ,VALUE,Remarks,Tolerance\r\n0603,\"10,5 kΩ\",ignored,\r\n",
        )
        .unwrap();
        let fields = names(&["阻值", "封装", "精度"]);
        let columns = table.match_columns(&fields);
        assert_eq!(columns, [Some(1), Some(0), Some(3)]);
        let rows = table.mapped_rows(&fields, &columns, &Row::new()).unwrap();
        assert_eq!(rows[0].len(), 3);
        assert_eq!(rows[0]["阻值"], "10,5 kΩ");
        assert_eq!(rows[0]["封装"], "0603");
        assert_eq!(rows[0]["精度"], "");
        assert_eq!(table.match_columns(&names(&["容值"])), [Some(1)]);
    }

    #[test]
    fn ambiguous_headers_require_manual_choice_and_exact_name_wins() {
        let table = Table::from_csv("MPN,MPN,封装,Footprint,VALUE,未知\nX,Y,A,B,10,C\n").unwrap();
        let fields = names(&["型号", "封装", "阻值", "容值", "备注"]);
        let columns = table.match_columns(&fields);
        assert_eq!(columns, [None, Some(2), None, None, None]);
        assert!(table
            .mapped_rows(&fields, &columns, &Row::new())
            .unwrap_err()
            .contains("型号"));
        let corrected = [Some(1), Some(2), Some(4), Some(4), Some(5)];
        assert_eq!(
            table.mapped_rows(&fields, &corrected, &Row::new()).unwrap()[0]["型号"],
            "Y"
        );
        assert_eq!(
            table.match_columns(&names(&["数值", "阻值"])),
            [Some(4), None]
        );
        assert!(table
            .mapped_rows(&fields, &[Some(6); 5], &Row::new())
            .is_err());
    }

    #[test]
    fn fixed_values_fill_only_fields_without_columns_in_every_record() {
        let table = Table::from_csv("Value\n10 kΩ\n1 kΩ\n").unwrap();
        let fields = names(&["阻值", "封装", "备注"]);
        let columns = table.match_columns(&fields);
        let fixed = Row::from([
            ("封装".to_string(), "0603".to_string()),
            ("备注".to_string(), String::new()),
            ("阻值".to_string(), "unused".to_string()),
        ]);
        let rows = table.mapped_rows(&fields, &columns, &fixed).unwrap();
        assert_eq!(rows[1]["阻值"], "1 kΩ");
        for row in rows {
            assert_eq!(row["封装"], "0603");
            assert_eq!(row["备注"], "");
        }
        assert!(table
            .mapped_rows(&fields, &[Some(99), None, None], &fixed)
            .is_err());
    }

    #[test]
    fn rows_without_template_fields_still_have_one_label_per_record() {
        let table = Table::from_csv("ignored\na\nb\n").unwrap();
        assert_eq!(
            table.mapped_rows(&[], &[], &Row::new()).unwrap(),
            vec![Row::new(); 2]
        );
        assert!(Table::from_csv("a,b\n1\n").is_err());
        assert!(Table::from_csv("a,b\n").is_err());
    }
}
