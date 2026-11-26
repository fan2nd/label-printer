use crate::core::MAX_BATCH;
use crate::table::{Table, MAX_COLUMNS};
use calamine::{open_workbook_auto, Data, ExcelDateTime, Reader, SheetType, Sheets};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

pub struct Workbook {
    reader: Sheets<BufReader<File>>,
    sheets: Vec<String>,
}

pub fn open_workbook(path: &Path) -> Result<Workbook, String> {
    let reader = open_workbook_auto(path)
        .map_err(|_| "无法打开 Excel 文件。请确认文件可以打开，且未设置打开密码。")?;
    let sheets: Vec<_> = reader
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect();
    if sheets.is_empty() {
        return Err("Excel 文件中没有工作表。".into());
    }
    Ok(Workbook { reader, sheets })
}

impl Workbook {
    pub fn sheet_names(&self) -> &[String] {
        &self.sheets
    }

    pub fn table(&mut self, sheet: &str) -> Result<Table, String> {
        let mut cells = SheetCells::default();
        let read_error = || format!("无法读取工作表「{sheet}」。");
        if let Sheets::Xlsx(book) = &mut self.reader {
            let mut reader = book
                .worksheet_cells_reader(sheet)
                .map_err(|_| read_error())?;
            while let Some((cell, style)) =
                reader.next_cell_with_style_id().map_err(|_| read_error())?
            {
                let format = reader
                    .styles()
                    .get(style)
                    .and_then(|style| style.number_format.as_ref())
                    .map(|number| number.format_code.as_str())
                    .unwrap_or_default();
                cells.insert(
                    cell.get_position(),
                    &Data::from(cell.get_value().clone()),
                    format,
                )?;
            }
        } else {
            let range = self
                .reader
                .worksheet_range(sheet)
                .map_err(|_| read_error())?;
            let (top, left) = range.start().unwrap_or_default();
            for (y, x, value) in range.used_cells() {
                cells.insert((top + y as u32, left + x as u32), value, "")?;
            }
        }
        cells
            .into_table()
            .map_err(|e| format!("工作表「{sheet}」：{e}"))
    }
}

// Keep only populated cells. Empty gaps in a worksheet can span thousands of rows.
#[derive(Default)]
struct SheetCells {
    rows: BTreeMap<u32, Vec<(u32, String)>>,
    columns: Option<(u32, u32)>,
}

impl SheetCells {
    fn insert(&mut self, (row, col): (u32, u32), value: &Data, format: &str) -> Result<(), String> {
        let text = match value {
            Data::Empty => return Ok(()),
            Data::DateTime(date) => format_date(*date),
            Data::Float(number) => format_number(*number, format),
            value => value.to_string(),
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        let (left, right) = self.columns.get_or_insert((col, col));
        *left = (*left).min(col);
        *right = (*right).max(col);
        if *right - *left >= MAX_COLUMNS as u32 {
            return Err(format!("表格最多支持 {MAX_COLUMNS} 列。"));
        }
        self.rows.entry(row).or_default().push((col, text));
        if self.rows.len() > MAX_BATCH + 1 {
            return Err(format!("单批最多 {MAX_BATCH} 条。"));
        }
        Ok(())
    }

    fn into_table(self) -> Result<Table, String> {
        let (left, right) = self.columns.ok_or("工作表为空。")?;
        let mut rows = self.rows.into_values().map(|cells| {
            let mut row = vec![String::new(); (right - left + 1) as usize];
            for (col, value) in cells {
                row[(col - left) as usize] = value;
            }
            row
        });
        let headers = rows.next().ok_or("缺少表头。")?;
        Table::from_records(headers, rows.collect())
    }
}

fn format_number(value: f64, format: &str) -> String {
    // Excel commonly stores part/package codes as padded integers (e.g. 0603).
    if !format.is_empty() && format.bytes().all(|b| b == b'0') && value.fract() == 0.0 {
        let sign = if value < 0.0 { "-" } else { "" };
        return format!("{sign}{:0width$.0}", value.abs(), width = format.len());
    }
    if let Some(percent) = format.strip_suffix('%') {
        let decimals = percent.strip_prefix("0.").unwrap_or("");
        if percent == "0" || (!decimals.is_empty() && decimals.bytes().all(|b| b == b'0')) {
            return format!("{:.*}%", decimals.len(), 100.0 * value);
        }
    }
    value.to_string()
}

fn format_date(date: ExcelDateTime) -> String {
    if date.is_duration() {
        let millis = (date.as_f64() * 86_400_000.0).round() as i64;
        let sign = if millis < 0 { "-" } else { "" };
        let ms = millis.unsigned_abs();
        let mut result = format!(
            "{sign}{}:{:02}:{:02}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1_000 % 60
        );
        if ms % 1000 != 0 {
            result.push_str(&format!(".{:03}", ms % 1_000));
        }
        return result;
    }
    let (year, month, day, hour, min, sec, ms) = date.to_ymd_hms_milli();
    let calendar = format!("{year:04}-{month:02}-{day:02}");
    let mut time = format!("{hour:02}:{min:02}:{sec:02}");
    if ms != 0 {
        time.push_str(&format!(".{ms:03}"));
    }
    if (0.0..1.0).contains(&date.as_f64()) {
        time
    } else if (hour, min, sec, ms) == (0, 0, 0, 0) {
        calendar
    } else {
        format!("{calendar} {time}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn xlsx_imports_a_selected_sheet_using_displayed_codes_and_cached_values() {
        fn require_send<T: Send>() {}
        require_send::<Workbook>();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("清单.xlsx");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        let files = [
            (
                "_rels/.rels",
                r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="空表" sheetId="1" r:id="rId1"/><sheet name="电阻" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/styles.xml",
                r#"<styleSheet><numFmts count="1"><numFmt numFmtId="164" formatCode="0000"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="9"/></cellXfs></styleSheet>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData></sheetData></worksheet>",
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<worksheet><sheetData>
                <row r="4"><c r="B4" t="inlineStr"><is><t>编号</t></is></c><c r="C4" t="inlineStr"><is><t>封装</t></is></c><c r="D4" t="inlineStr"><is><t>精度</t></is></c></row>
                <row r="7"><c r="B7" t="inlineStr"><is><t>0009</t></is></c><c r="C7" s="1"><v>603</v></c><c r="D7" s="2"><f>1/100</f><v>0.01</v></c></row>
                <row r="10"><c r="B10" t="inlineStr"><is><t>0010</t></is></c></row>
                </sheetData></worksheet>"#,
            ),
        ];
        for (name, xml) in files {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        drop(zip.finish().unwrap());
        let mut workbook = open_workbook(&path).unwrap();
        assert_eq!(workbook.sheet_names(), ["空表", "电阻"]);
        assert!(workbook.table("空表").unwrap_err().contains("空表"));
        let table = workbook.table("电阻").unwrap();
        assert_eq!(table.headers, ["编号", "封装", "精度"]);
        assert_eq!(table.rows[0], ["0009", "0603", "1%"]);
        assert_eq!(table.rows[1], ["0010", "", ""]);
    }
}
