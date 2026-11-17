use crate::model::{
    CompanyConfig, NodeReview, Preview, RawSourcePreview, RunResult, SourceBinding, SourcePreview,
    TextOptions, TracePart, Workflow,
};
use crate::workflow::{identifier, ordered_nodes, required_str, required_strings, sql_name};
use calamine::{open_workbook, Dimensions, Reader, Xlsx};
use duckdb::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};
static RESULT_QUERY_LOCK: Mutex<()> = Mutex::new(());
pub(crate) fn result_query_guard() -> Result<MutexGuard<'static, ()>, String> {
    RESULT_QUERY_LOCK.lock().map_err(|_| "结果查询状态异常，请重新打开程序".into())
}

pub(crate) fn lit(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn n(id: &str) -> Result<String, String> {
    Ok(format!("n_{}", identifier(id)?.trim_matches('"')))
}
fn table(id: &str) -> Result<String, String> {
    Ok(sql_name(&n(id)?))
}
fn query_i64(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|e| e.to_string())
}
fn execute(conn: &Connection, sql: &str) -> Result<(), String> {
    conn.execute_batch(sql)
        .map_err(|e| format!("SQL 执行失败：{e}; 语句：{sql}"))
}
fn amount_text(item:&Value,source:&str)->String{
    let trim_special=item.get("trim_special_space").and_then(Value::as_bool).unwrap_or(false);
    let mut text=if trim_special {format!("TRIM(CAST({source} AS VARCHAR), {})",lit(" \u{a0}\u{202f}\u{2007}\u{3000}"))}
        else {format!("TRIM(CAST({source} AS VARCHAR))")};
    if item.get("parentheses_negative").and_then(Value::as_bool)==Some(true){
        text=format!("CASE WHEN regexp_full_match({text}, '[(].*[)]') THEN '-' || TRIM(SUBSTR({text},2,LENGTH({text})-2)) ELSE {text} END");
    }
    if let Some(prefix)=item.get("currency_prefix").and_then(Value::as_str).filter(|prefix|!prefix.is_empty()){
        let len=prefix.chars().count();
        // The minus sign can have been introduced by parentheses.
        text=format!("CASE WHEN starts_with({text}, {}) THEN TRIM(SUBSTR({text},{})) WHEN starts_with({text},{}) THEN '-' || TRIM(SUBSTR({text},{})) ELSE {text} END",
            lit(prefix),len+1,lit(&format!("-{prefix}")),len+2);
    }
    if item.get("thousands").and_then(Value::as_bool)==Some(true){
        text=format!("CASE WHEN regexp_full_match({text}, '[+-]?[0-9]{{1,3}}(,[0-9]{{3}})+([.][0-9]+)?') THEN REPLACE({text},',','') ELSE {text} END");
    }
    text
}
fn config_array<'a>(v: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    v.get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("缺少参数 {name}"))
}
fn incoming<'a>(wf: &'a Workflow, id: &str) -> Vec<&'a str> {
    wf.edges
        .iter()
        .filter(|e| e.target == id)
        .map(|e| e.source.as_str())
        .collect()
}
fn hash_file(path: &Path) -> Result<String, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let len = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        hasher.update(&buf[..len]);
    }
    Ok(hex::encode(hasher.finalize()))
}
fn validate_headers(headers: &[String]) -> Result<(), String> {
    if headers.is_empty() || headers.iter().any(|h| h.is_empty() || h.starts_with("__")) {
        return Err("表头为空或使用保留前缀 __".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    if headers.iter().any(|h| !unique.insert(h)) {
        return Err("表头存在重复列，须先整理".into());
    }
    Ok(())
}

fn text_options(binding: &SourceBinding) -> Result<(TextOptions, u8), String> {
    let extension = Path::new(&binding.path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let defaults = TextOptions {
        delimiter: if extension == "tsv" {
            "\t".into()
        } else {
            ",".into()
        },
        ..TextOptions::default()
    };
    let options = binding.text.clone().unwrap_or(defaults);
    let delimiter = match options.delimiter.as_str() {
        "," => b',',
        "\t" => b'\t',
        ";" => b';',
        "|" => b'|',
        _ => return Err("分隔符只支持逗号、Tab、分号或竖线".into()),
    };
    if options.header_record == 0 {
        return Err("表头记录序号必须从 1 开始".into());
    }
    if !matches!(
        options.encoding.to_ascii_lowercase().as_str(),
        "utf-8" | "gb18030"
    ) {
        return Err("编码只支持 UTF-8 或 GB18030".into());
    }
    Ok((options, delimiter))
}

/// Decode one physical line at a time. A GB18030 character cannot contain LF,
/// so quoted CSV records can still span several decoded physical lines.
struct DecodedLines<R: BufRead> {
    source: R,
    encoding: String,
    pending: Vec<u8>,
    offset: usize,
    first: bool,
}

#[derive(Default)]
struct LineTracker {
    boundaries: VecDeque<usize>,
    completed: usize,
    previous_cr: bool,
}

impl LineTracker {
    fn line_at(&mut self, byte_offset: usize) -> usize {
        while self.boundaries.front().is_some_and(|at| *at <= byte_offset) {
            self.boundaries.pop_front();
            self.completed += 1;
        }
        self.completed + 1
    }
}

struct LineTrackingRead<R: Read> {
    source: R,
    byte_offset: usize,
    tracker: Rc<RefCell<LineTracker>>,
}

impl<R: Read> Read for LineTrackingRead<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.source.read(out)?;
        let mut tracker = self.tracker.borrow_mut();
        for (index, byte) in out[..count].iter().enumerate() {
            let at = self.byte_offset + index;
            if *byte == b'\r' {
                tracker.boundaries.push_back(at);
            } else if *byte == b'\n' && !tracker.previous_cr {
                tracker.boundaries.push_back(at);
            }
            tracker.previous_cr = *byte == b'\r';
        }
        self.byte_offset += count;
        Ok(count)
    }
}

impl<R: BufRead> Read for DecodedLines<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.offset == self.pending.len() {
            let mut raw = Vec::new();
            if self.source.read_until(b'\n', &mut raw)? == 0 {
                return Ok(0);
            }
            if self.first && raw.starts_with(&[0xef, 0xbb, 0xbf]) {
                raw.drain(..3);
            }
            self.first = false;
            let decoded = if self.encoding.eq_ignore_ascii_case("utf-8") {
                std::str::from_utf8(&raw)
                    .map(|s| s.to_owned())
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "文件不是 UTF-8；请在输入设置中选择 GB18030 编码",
                        )
                    })?
            } else {
                encoding_rs::GB18030
                    .decode_without_bom_handling_and_without_replacement(&raw)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "GB18030 解码失败；请检查文件编码",
                        )
                    })?
                    .into_owned()
            };
            self.pending = decoded.into_bytes();
            self.offset = 0;
        }
        let size = out.len().min(self.pending.len() - self.offset);
        out[..size].copy_from_slice(&self.pending[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
}

fn open_text_reader(
    binding: &SourceBinding,
    flexible: bool,
) -> Result<(csv::Reader<Box<dyn Read>>, Rc<RefCell<LineTracker>>), String> {
    let (options, delimiter) = text_options(binding)?;
    let file = File::open(&binding.path).map_err(|e| e.to_string())?;
    let mut buffered = BufReader::new(file);
    let source: Box<dyn Read> = if options.encoding.eq_ignore_ascii_case("utf-8") {
        if buffered
            .fill_buf()
            .map_err(|e| e.to_string())?
            .starts_with(&[0xef, 0xbb, 0xbf])
        {
            buffered.consume(3);
        }
        Box::new(buffered)
    } else {
        Box::new(DecodedLines {
            source: buffered,
            encoding: options.encoding,
            pending: Vec::new(),
            offset: 0,
            first: true,
        })
    };
    let tracker = Rc::new(RefCell::new(LineTracker::default()));
    let tracked: Box<dyn Read> = Box::new(LineTrackingRead {
        source,
        byte_offset: 0,
        tracker: tracker.clone(),
    });
    let reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(flexible)
        .delimiter(delimiter)
        .from_reader(tracked);
    Ok((reader, tracker))
}

fn text_csv_error(error: csv::Error) -> String {
    format!("文本读取失败（如为中文系统导出，请尝试 GB18030 编码）：{error}")
}

fn text_row_number(
    record: &csv::StringRecord,
    tracker: &Rc<RefCell<LineTracker>>,
) -> Result<usize, String> {
    let position = record.position().ok_or("无法定位文本原始行")?;
    Ok(tracker.borrow_mut().line_at(position.byte() as usize))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    DelimitedText,
    Xlsx,
}

fn source_kind(path: &Path) -> Result<SourceKind, String> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "csv" | "txt" | "tsv" => Ok(SourceKind::DelimitedText),
        "xlsx" => Ok(SourceKind::Xlsx),
        _ => Err("目前支持 CSV、分隔符 TXT／TSV 和 XLSX；XML／IDoc 须按实际结构另行接入".into()),
    }
}

struct XlsxHeader {
    first_col: u32,
    width: usize,
}

fn emit_xlsx_row<F>(
    sheet: &str,
    row: u32,
    cells: &[(u32, String)],
    header: &mut Option<XlsxHeader>,
    visit: &mut F,
) -> Result<bool, String>
where
    F: FnMut(&str, bool, usize, Vec<String>) -> Result<bool, String>,
{
    if header.is_none() {
        let first_col = cells
            .iter()
            .map(|(col, _)| *col)
            .min()
            .ok_or("Excel 工作表为空")?;
        let last_col = cells.iter().map(|(col, _)| *col).max().unwrap();
        let width = (last_col - first_col + 1) as usize;
        let mut values = vec![String::new(); width];
        for (col, value) in cells {
            values[(col - first_col) as usize] = value.clone();
        }
        validate_headers(&values)?;
        *header = Some(XlsxHeader { first_col, width });
        return visit(sheet, true, row as usize + 1, values);
    }
    let header = header.as_ref().unwrap();
    let mut values = vec![String::new(); header.width];
    for (col, value) in cells {
        let offset = col.checked_sub(header.first_col).map(|v| v as usize);
        match offset {
            Some(index) if index < header.width => values[index] = value.clone(),
            _ => {
                return Err(format!(
                    "Excel 原始行 {} 存在表头以外的列，请先整理",
                    row + 1
                ))
            }
        }
    }
    visit(sheet, false, row as usize + 1, values)
}

/// Visit rows as the XLSX XML is parsed; at most one row is buffered here.
/// Shared strings are still loaded by calamine when the workbook is opened.
fn scan_xlsx<F>(binding: &SourceBinding, mut visit: F) -> Result<(String, bool), String>
where
    F: FnMut(&str, bool, usize, Vec<String>) -> Result<bool, String>,
{
    let mut workbook: Xlsx<_> =
        open_workbook::<Xlsx<_>, _>(Path::new(&binding.path)).map_err(|e| e.to_string())?;
    let sheet = binding
        .sheet
        .clone()
        .or_else(|| workbook.sheet_names().first().cloned())
        .ok_or("Excel 没有工作表")?;
    let mut reader = workbook
        .worksheet_cells_reader(&sheet)
        .map_err(|e| e.to_string())?;
    let mut current_row: Option<u32> = None;
    let mut cells: Vec<(u32, String)> = Vec::new();
    let mut header = None;
    loop {
        let cell = reader.next_cell().map_err(|e| e.to_string())?;
        let Some(cell) = cell else { break };
        let (row, col) = cell.get_position();
        if let Some(previous) = current_row {
            if row < previous {
                return Err("Excel 单元格行号不是升序，请先整理".into());
            }
            if row != previous {
                if !emit_xlsx_row(&sheet, previous, &cells, &mut header, &mut visit)? {
                    return Ok((sheet, true));
                }
                cells.clear();
                for empty_row in previous + 1..row {
                    if !emit_xlsx_row(&sheet, empty_row, &[], &mut header, &mut visit)? {
                        return Ok((sheet, true));
                    }
                }
            }
        }
        current_row = Some(row);
        let value: calamine::Data = cell.get_value().clone().into();
        cells.push((col, value.to_string()));
    }
    let final_row = current_row.ok_or("Excel 工作表为空")?;
    let stopped = !emit_xlsx_row(&sheet, final_row, &cells, &mut header, &mut visit)?;
    Ok((sheet, stopped))
}

pub(crate) fn validate_layout(layout: &crate::model::TableLayout) -> Result<(), String> {
    if layout.first_col == 0
        || layout.last_col < layout.first_col
        || layout.last_col - layout.first_col > 512
    {
        return Err("结构整理的列范围无效，且一次最多处理 513 列".into());
    }
    if layout.header_rows.is_empty()
        || layout.header_rows.len() > 10
        || layout
            .header_rows
            .iter()
            .any(|r| *r == 0 || *r >= layout.data_start_row)
        || layout.header_rows.windows(2).any(|p| p[0] >= p[1])
        || layout.data_start_row == 0
        || layout
            .data_end_row
            .is_some_and(|end| end < layout.data_start_row)
        || layout
            .fill_merged_data_cols
            .iter()
            .any(|col| *col < layout.first_col || *col > layout.last_col)
    {
        return Err("结构整理的表头行、数据起止行或补值列无效".into());
    }
    Ok(())
}

fn grid_headers(layout: &crate::model::TableLayout) -> Vec<String> {
    (layout.first_col..=layout.last_col)
        .map(|col| format!("col_{col}"))
        .collect()
}

fn emit_grid_row<F>(
    sheet: &str,
    row: u32,
    cells: &[(u32, String)],
    layout: &crate::model::TableLayout,
    visit: &mut F,
) -> Result<bool, String>
where
    F: FnMut(&str, usize, Vec<String>) -> Result<bool, String>,
{
    let mut values = vec![String::new(); (layout.last_col - layout.first_col + 1) as usize];
    for (col, value) in cells {
        let physical_col = col + 1;
        if physical_col >= layout.first_col && physical_col <= layout.last_col {
            values[(physical_col - layout.first_col) as usize] = value.clone();
        }
    }
    visit(sheet, row as usize + 1, values)
}

/// In grid mode the source keeps physical rows and cells unchanged; structure is a later node.
fn scan_xlsx_grid<F>(
    binding: &SourceBinding,
    mut visit: F,
) -> Result<(String, Vec<Dimensions>, bool), String>
where
    F: FnMut(&str, usize, Vec<String>) -> Result<bool, String>,
{
    let layout = binding.layout.as_ref().ok_or("缺少结构整理配置")?;
    validate_layout(layout)?;
    let mut workbook: Xlsx<_> =
        open_workbook::<Xlsx<_>, _>(Path::new(&binding.path)).map_err(|e| e.to_string())?;
    let sheet = binding
        .sheet
        .clone()
        .or_else(|| workbook.sheet_names().first().cloned())
        .ok_or("Excel 没有工作表")?;
    let merges = workbook
        .worksheet_merge_cells(&sheet)
        .ok_or("工作表不存在")?
        .map_err(|e| e.to_string())?;
    let mut reader = workbook
        .worksheet_cells_reader(&sheet)
        .map_err(|e| e.to_string())?;
    let mut current_row: Option<u32> = None;
    let mut cells: Vec<(u32, String)> = Vec::new();
    loop {
        let cell = reader.next_cell().map_err(|e| e.to_string())?;
        let Some(cell) = cell else { break };
        let (row, col) = cell.get_position();
        if let Some(previous) = current_row {
            if row < previous {
                return Err("Excel 单元格行号不是升序".into());
            }
            if row != previous {
                if !emit_grid_row(&sheet, previous, &cells, layout, &mut visit)? {
                    return Ok((sheet, merges, true));
                }
                cells.clear();
                for empty in previous + 1..row {
                    if !emit_grid_row(&sheet, empty, &[], layout, &mut visit)? {
                        return Ok((sheet, merges, true));
                    }
                }
            }
        } else {
            for empty in 0..row {
                if !emit_grid_row(&sheet, empty, &[], layout, &mut visit)? {
                    return Ok((sheet, merges, true));
                }
            }
        }
        current_row = Some(row);
        let value: calamine::Data = cell.get_value().clone().into();
        if layout.strict_columns
            && row as usize + 1 >= layout.data_start_row
            && layout
                .data_end_row
                .is_none_or(|end| row as usize + 1 <= end)
            && (col + 1 < layout.first_col || col + 1 > layout.last_col)
            && !value.to_string().trim().is_empty()
        {
            return Err(format!(
                "原始第 {} 行、第 {} 列在所选明细列范围外仍有值；请调整范围或明确确认排除",
                row + 1,
                col + 1
            ));
        }
        cells.push((col, value.to_string()));
    }
    let last = current_row.ok_or("Excel 工作表为空")?;
    if !emit_grid_row(&sheet, last, &cells, layout, &mut visit)? {
        return Ok((sheet, merges, true));
    }
    let merge_last = merges
        .iter()
        .map(|region| region.end.0)
        .max()
        .unwrap_or(last);
    let final_row = layout
        .data_end_row
        .map_or(last.max(merge_last), |end| last.max(end as u32 - 1));
    for empty in last + 1..=final_row {
        if !emit_grid_row(&sheet, empty, &[], layout, &mut visit)? {
            return Ok((sheet, merges, true));
        }
    }
    Ok((sheet, merges, false))
}

pub fn inspect_raw_source(
    binding: &SourceBinding,
    limit: usize,
) -> Result<RawSourcePreview, String> {
    let path = Path::new(&binding.path);
    let cap = limit.clamp(1, 200);
    if source_kind(path)? == SourceKind::DelimitedText {
        let (mut reader, tracker) = open_text_reader(binding, true)?;
        let mut rows = Vec::new();
        let mut physical_rows = Vec::new();
        let mut width = 0;
        let mut truncated = false;
        for record in reader.records() {
            let record = record.map_err(text_csv_error)?;
            if rows.len() >= cap {
                truncated = true;
                break;
            }
            width = width.max(record.len());
            physical_rows.push(text_row_number(&record, &tracker)?);
            rows.push(record.iter().map(str::to_string).collect::<Vec<_>>());
        }
        for row in &mut rows {
            row.resize(width, String::new());
        }
        Ok(RawSourcePreview {
            columns: (1..=width).map(|n| format!("第{n}列")).collect(),
            rows,
            physical_rows,
            truncated,
            sheet: String::new(),
        })
    } else if source_kind(path)? == SourceKind::Xlsx {
        let mut workbook: Xlsx<_> = open_workbook::<Xlsx<_>, _>(path).map_err(|e| e.to_string())?;
        let sheet = binding
            .sheet
            .clone()
            .or_else(|| workbook.sheet_names().first().cloned())
            .ok_or("Excel 没有工作表")?;
        let mut cells = workbook
            .worksheet_cells_reader(&sheet)
            .map_err(|e| e.to_string())?;
        let mut raw_rows: BTreeMap<usize, BTreeMap<usize, String>> = BTreeMap::new();
        let mut width = 0usize;
        let mut truncated = false;
        loop {
            let Some(cell) = cells.next_cell().map_err(|e| e.to_string())? else {
                break;
            };
            let (row, col) = cell.get_position();
            if row as usize >= cap {
                truncated = true;
                break;
            }
            let value: calamine::Data = cell.get_value().clone().into();
            raw_rows
                .entry(row as usize + 1)
                .or_default()
                .insert(col as usize, value.to_string());
            width = width.max(col as usize + 1);
        }
        let end = raw_rows.keys().next_back().copied().unwrap_or(0);
        let rows = (1..=end)
            .map(|row| {
                let mut values = vec![String::new(); width];
                if let Some(cells) = raw_rows.get(&row) {
                    for (col, value) in cells {
                        values[*col] = value.clone();
                    }
                }
                values
            })
            .collect();
        Ok(RawSourcePreview {
            columns: (1..=width).map(|n| format!("第{n}列")).collect(),
            rows,
            physical_rows: (1..=end).collect(),
            truncated,
            sheet,
        })
    } else {
        unreachable!()
    }
}

pub fn inspect_source(binding: &SourceBinding, limit: usize) -> Result<SourcePreview, String> {
    let path = Path::new(&binding.path);
    let cap = limit.clamp(1, 200);
    if source_kind(path)? == SourceKind::DelimitedText {
        let (mut reader, tracker) = open_text_reader(binding, true)?;
        let (options, _) = text_options(binding)?;
        let mut records = reader.records();
        for _ in 1..options.header_record {
            let skipped = records
                .next()
                .ok_or("找不到指定的表头记录")?
                .map_err(text_csv_error)?;
            text_row_number(&skipped, &tracker)?;
        }
        let header = records
            .next()
            .ok_or("找不到指定的表头记录")?
            .map_err(text_csv_error)?;
        text_row_number(&header, &tracker)?;
        let headers = header.iter().map(str::to_string).collect::<Vec<_>>();
        validate_headers(&headers)?;
        let mut rows = Vec::new();
        let mut physical_rows = Vec::new();
        let mut truncated = false;
        for record in records {
            let record = record.map_err(text_csv_error)?;
            if rows.len() >= cap {
                truncated = true;
                break;
            }
            if record.len() != headers.len() {
                return Err(format!(
                    "原始行 {} 的列数与表头不一致",
                    text_row_number(&record, &tracker)?
                ));
            }
            physical_rows.push(text_row_number(&record, &tracker)?);
            rows.push(record.iter().map(str::to_string).collect());
        }
        Ok(SourcePreview {
            headers,
            rows,
            physical_rows,
            truncated,
            sheet: String::new(),
        })
    } else if source_kind(path)? == SourceKind::Xlsx {
        if let Some(layout) = &binding.layout {
            let headers = grid_headers(layout);
            let mut rows = Vec::new();
            let mut physical_rows = Vec::new();
            let (sheet, _, truncated) = scan_xlsx_grid(binding, |_, row_no, row| {
                if rows.len() >= cap {
                    return Ok(false);
                }
                physical_rows.push(row_no);
                rows.push(row);
                Ok(true)
            })?;
            return Ok(SourcePreview {
                headers,
                rows,
                physical_rows,
                truncated,
                sheet,
            });
        }
        let mut headers = Vec::new();
        let mut rows = Vec::new();
        let mut physical_rows = Vec::new();
        let (sheet, truncated) = scan_xlsx(binding, |_, is_header, row_no, row| {
            if is_header {
                headers = row;
                return Ok(true);
            }
            if rows.len() >= cap {
                return Ok(false);
            }
            physical_rows.push(row_no);
            rows.push(row);
            Ok(true)
        })?;
        Ok(SourcePreview {
            headers,
            rows,
            physical_rows,
            truncated,
            sheet,
        })
    } else {
        unreachable!()
    }
}

pub fn inspect_raw_window(
    binding: &SourceBinding,
    start_row: usize,
    start_col: u32,
) -> Result<RawSourcePreview, String> {
    if source_kind(Path::new(&binding.path))? != SourceKind::Xlsx {
        return inspect_raw_slice(binding,start_row,start_col,false);
    }
    if start_row == 0 || start_col == 0 || start_col > 16384 {
        return Err("物理行列须从1开始，列不能超出XLSX范围".into());
    }
    let end_row = start_row.checked_add(19).ok_or("行号超出范围")?;
    let end_col = start_col.saturating_add(19).min(16384);
    let mut workbook: Xlsx<_> =
        open_workbook(&binding.path).map_err(|e: calamine::XlsxError| e.to_string())?;
    let sheet = binding
        .sheet
        .clone()
        .or_else(|| workbook.sheet_names().first().cloned())
        .ok_or("没有工作表")?;
    let mut reader = workbook
        .worksheet_cells_reader(&sheet)
        .map_err(|e| e.to_string())?;
    let mut rows = vec![vec![String::new(); (end_col - start_col + 1) as usize]; 20];
    let mut last = 0;
    let mut truncated = false;
    while let Some(cell) = reader.next_cell().map_err(|e| e.to_string())? {
        let (r, c) = cell.get_position();
        let r = r as usize + 1;
        let c = c + 1;
        if r > end_row {
            truncated = true;
            break;
        }
        last = last.max(r);
        if r >= start_row && c >= start_col && c <= end_col {
            let value: calamine::Data = cell.get_value().clone().into();
            rows[r - start_row][(c - start_col) as usize] = value.to_string();
        }
    }
    let count = last.saturating_sub(start_row - 1).min(20);
    rows.truncate(count);
    Ok(RawSourcePreview {
        columns: (start_col..=end_col).map(|c| format!("第{c}列")).collect(),
        rows,
        physical_rows: (start_row..start_row + count).collect(),
        truncated,
        sheet,
    })
}

pub fn inspect_raw_slice(binding:&SourceBinding,start_row:usize,start_col:u32,tail:bool)->Result<RawSourcePreview,String>{
    if start_row==0||start_col==0||start_col>16384{return Err("物理行列须从1开始，列不能超出16384".into());}
    if !tail&&source_kind(Path::new(&binding.path))?==SourceKind::Xlsx{
        return inspect_raw_window(binding,start_row,start_col);
    }
    let mut rows=VecDeque::new();
    let mut positions=VecDeque::new();
    let mut truncated=false;
    let end_col=start_col.saturating_add(19).min(16384);
    let width=(end_col-start_col+1) as usize;
    let sheet;
    if source_kind(Path::new(&binding.path))?==SourceKind::DelimitedText{
        sheet=String::new();
        let (mut reader,tracker)=open_text_reader(binding,true)?;
        for record in reader.records(){
            let record=record.map_err(text_csv_error)?;
            let row=text_row_number(&record,&tracker)?;
            if !tail&&row<start_row{continue;}
            if rows.len()==20{
                truncated=true;
                if !tail{break;}
                rows.pop_front();positions.pop_front();
            }
            rows.push_back((start_col..=end_col).map(|column|record.get(column as usize-1).unwrap_or("").to_owned()).collect::<Vec<_>>());
            positions.push_back(row);
        }
    }else{
        let mut workbook:Xlsx<_>=open_workbook(&binding.path).map_err(|error:calamine::XlsxError|error.to_string())?;
        sheet=binding.sheet.clone().or_else(||workbook.sheet_names().first().cloned()).ok_or("没有工作表")?;
        let mut reader=workbook.worksheet_cells_reader(&sheet).map_err(|error|error.to_string())?;
        let mut selected=BTreeMap::<usize,Vec<String>>::new();
        let mut last=0;
        while let Some(cell)=reader.next_cell().map_err(|error|error.to_string())?{
            let (row,column)=cell.get_position();
            let row=row as usize+1;let column=column+1;
            last=last.max(row);
            let first=last.saturating_sub(19).max(1);
            selected.retain(|position,_|*position>=first);
            if column>=start_col&&column<=end_col{
                let values=selected.entry(row).or_insert_with(||vec![String::new();width]);
                let value:calamine::Data=cell.get_value().clone().into();
                values[(column-start_col) as usize]=value.to_string();
            }
        }
        truncated=last>20;
        for row in last.saturating_sub(19).max(1)..=last{
            positions.push_back(row);
            rows.push_back(selected.remove(&row).unwrap_or_else(||vec![String::new();width]));
        }
    }
    Ok(RawSourcePreview{columns:(start_col..=end_col).map(|column|format!("第{column}列")).collect(),rows:rows.into(),physical_rows:positions.into(),truncated,sheet})
}

pub fn check_reading_extent(binding: &SourceBinding) -> Result<(), crate::model::ReadingIssue> {
    let error = |message: String, row, col| crate::model::ReadingIssue {
        code: "outside_columns".into(),
        message,
        row,
        col,
        related_cols: Vec::new(),
    };
    let Some(layout) = &binding.layout else {
        return Err(error("请确认读取区域".into(), None, None));
    };
    if !layout.strict_columns {
        return Ok(());
    }
    let mut workbook: Xlsx<_> = open_workbook(&binding.path)
        .map_err(|e: calamine::XlsxError| error(e.to_string(), None, None))?;
    let sheet = binding
        .sheet
        .clone()
        .or_else(|| workbook.sheet_names().first().cloned())
        .ok_or_else(|| error("没有工作表".into(), None, None))?;
    let mut reader = workbook
        .worksheet_cells_reader(&sheet)
        .map_err(|e| error(e.to_string(), None, None))?;
    while let Some(cell) = reader
        .next_cell()
        .map_err(|e| error(e.to_string(), None, None))?
    {
        let (r, c) = cell.get_position();
        let r = r as usize + 1;
        let c = c + 1;
        if r < layout.data_start_row {
            continue;
        }
        if layout.data_end_row.is_some_and(|end| r > end) {
            break;
        }
        if c < layout.first_col || c > layout.last_col {
            let value: calamine::Data = cell.get_value().clone().into();
            if !value.to_string().trim().is_empty() {
                return Err(error(
                    "所选明细列范围外还有值，请扩大范围或明确确认排除".into(),
                    Some(r),
                    Some(c),
                ));
            }
        }
    }
    Ok(())
}

pub fn inspect_normalized_source(binding: &SourceBinding) -> Result<SourcePreview, String> {
    let report = crate::source_structure::inspect(binding)?;
    let active = report.active.ok_or("请先确认表格结构")?;
    if let Some(expected) = &binding.reading_baseline {
        let differences = crate::source_structure::compare_baseline(expected, &active.baseline);
        if !differences.is_empty() {
            return Err(differences
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("；"));
        }
    }
    if !active.ready {
        return Err(active.issues.join("；"));
    }
    Ok(active.preview)
}

pub fn source_sheets(path: &str) -> Result<Vec<String>, String> {
    let source = Path::new(path);
    match source_kind(source)? {
        SourceKind::DelimitedText => Ok(Vec::new()),
        SourceKind::Xlsx => {
            let workbook: Xlsx<_> =
                open_workbook::<Xlsx<_>, _>(source).map_err(|e| e.to_string())?;
            Ok(workbook.sheet_names())
        }
    }
}
fn create_source_table(
    conn: &Connection,
    table_name: &str,
    headers: &[String],
) -> Result<(), String> {
    validate_headers(&headers)?;
    let defs = headers
        .iter()
        .map(|h| format!("{} VARCHAR", sql_name(h)))
        .collect::<Vec<_>>()
        .join(", ");
    execute(conn, &format!("CREATE TABLE {table_name} (__company VARCHAR, __file VARCHAR, __sheet VARCHAR, __row BIGINT, __record BIGINT, __role VARCHAR, {defs})"))
}

fn append_source_row(
    appender: &mut duckdb::Appender<'_>,
    width: usize,
    binding: &SourceBinding,
    company: &str,
    role: &str,
    sheet: &str,
    row_no: usize,
    record_no: usize,
    row: Vec<String>,
) -> Result<(), String> {
    if row.len() != width {
        return Err(format!("原始行 {row_no} 的列数与表头不一致"));
    }
    let mut values = vec![
        company.to_string(),
        binding.path.clone(),
        sheet.to_string(),
        row_no.to_string(),
        record_no.to_string(),
        role.to_string(),
    ];
    values.extend(row);
    let row_id = row_no as i64;
    let record_id = record_no as i64;
    let mut sql_values: Vec<&dyn duckdb::ToSql> = Vec::with_capacity(values.len());
    for (index, item) in values.iter().enumerate() {
        if index == 3 {
            sql_values.push(&row_id);
        } else if index == 4 {
            sql_values.push(&record_id);
        } else {
            sql_values.push(item);
        }
    }
    appender
        .append_row(sql_values.as_slice())
        .map_err(|e| e.to_string())
}

fn import_source(
    conn: &Connection,
    table_name: &str,
    binding: &SourceBinding,
    company: &str,
    role: &str,
) -> Result<(), String> {
    let path = Path::new(&binding.path);
    if source_kind(path)? == SourceKind::DelimitedText {
        let (mut reader, tracker) = open_text_reader(binding, true)?;
        let (options, _) = text_options(binding)?;
        let mut records = reader.records();
        for _ in 1..options.header_record {
            let skipped = records
                .next()
                .ok_or("找不到指定的表头记录")?
                .map_err(text_csv_error)?;
            text_row_number(&skipped, &tracker)?;
        }
        let header = records
            .next()
            .ok_or("找不到指定的表头记录")?
            .map_err(text_csv_error)?;
        text_row_number(&header, &tracker)?;
        let headers = header.iter().map(str::to_string).collect::<Vec<_>>();
        create_source_table(conn, table_name, &headers)?;
        let mut appender = conn
            .appender(table_name.trim_matches('"'))
            .map_err(|e| e.to_string())?;
        for (index, record) in records.enumerate() {
            let record = record.map_err(text_csv_error)?;
            let row_no = text_row_number(&record, &tracker)?;
            append_source_row(
                &mut appender,
                headers.len(),
                binding,
                company,
                role,
                "",
                row_no,
                index + 1,
                record.iter().map(str::to_string).collect(),
            )?;
        }
        appender.flush().map_err(|e| e.to_string())?;
    } else if source_kind(path)? == SourceKind::Xlsx {
        if let Some(layout) = &binding.layout {
            validate_layout(layout)?;
            let headers = grid_headers(layout);
            create_source_table(conn, table_name, &headers)?;
            let mut appender = conn
                .appender(table_name.trim_matches('"'))
                .map_err(|e| e.to_string())?;
            let mut record_no = 0usize;
            let (_, merges, _) = scan_xlsx_grid(binding, |sheet, row_no, row| {
                record_no += 1;
                append_source_row(
                    &mut appender,
                    headers.len(),
                    binding,
                    company,
                    role,
                    sheet,
                    row_no,
                    record_no,
                    row,
                )?;
                Ok(true)
            })?;
            appender.flush().map_err(|e| e.to_string())?;
            let merge_table = sql_name(&format!("{}__merges", table_name.trim_matches('"')));
            execute(conn,&format!("CREATE TABLE {merge_table} (row_start BIGINT,row_end BIGINT,col_start BIGINT,col_end BIGINT)"))?;
            let mut merge_appender = conn
                .appender(merge_table.trim_matches('"'))
                .map_err(|e| e.to_string())?;
            for region in merges {
                let row_start = region.start.0 as i64 + 1;
                let row_end = region.end.0 as i64 + 1;
                let col_start = region.start.1 as i64 + 1;
                let col_end = region.end.1 as i64 + 1;
                let values: [&dyn duckdb::ToSql; 4] = [&row_start, &row_end, &col_start, &col_end];
                merge_appender
                    .append_row(values)
                    .map_err(|e| e.to_string())?;
            }
            merge_appender.flush().map_err(|e| e.to_string())?;
            return Ok(());
        }
        let mut appender = None;
        let mut width = 0usize;
        let mut record_no = 0usize;
        scan_xlsx(binding, |sheet, is_header, row_no, row| {
            if is_header {
                width = row.len();
                create_source_table(conn, table_name, &row)?;
                appender = Some(
                    conn.appender(table_name.trim_matches('"'))
                        .map_err(|e| e.to_string())?,
                );
            } else {
                record_no += 1;
                append_source_row(
                    appender.as_mut().ok_or("Excel 缺少表头")?,
                    width,
                    binding,
                    company,
                    role,
                    sheet,
                    row_no,
                    record_no,
                    row,
                )?;
            }
            Ok(true)
        })?;
        appender
            .as_mut()
            .ok_or("Excel 缺少表头")?
            .flush()
            .map_err(|e| e.to_string())?;
    } else {
        unreachable!()
    }
    Ok(())
}
fn keys(v: &Value) -> Result<Vec<String>, String> {
    let result = required_strings(v, "keys")?;
    for k in &result {
        identifier(k)?;
    }
    if result.is_empty() {
        return Err("匹配键不能为空".into());
    }
    Ok(result)
}
fn issue_sources(config: &Value) -> Result<Vec<String>, String> {
    match config.get("issue_sources") {
        Some(value) => value
            .as_array()
            .ok_or("issue_sources 必须是数组")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "异常来源必须是节点 ID".to_string())
            })
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn process_structure(
    conn: &Connection,
    node: &crate::model::Node,
    binding: &SourceBinding,
    source_id: &str,
    input: &str,
    out: &str,
) -> Result<(), String> {
    let layout = binding
        .layout
        .as_ref()
        .ok_or("公司资料缺少结构整理配置 layout")?;
    validate_layout(layout)?;
    if Path::new(&binding.path)
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case("xlsx"))
    {
        return Err("复杂结构整理目前仅支持 XLSX".into());
    }
    let merge_table = sql_name(&format!("{}__merges", n(source_id)?));
    let mut merge_stmt = conn
        .prepare(&format!(
            "SELECT row_start,row_end,col_start,col_end FROM {merge_table}"
        ))
        .map_err(|e| e.to_string())?;
    let merges = merge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut header_rows = BTreeMap::new();
    for physical_row in &layout.header_rows {
        let select = (layout.first_col..=layout.last_col)
            .map(|col| sql_name(&format!("col_{col}")))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT {select} FROM {input} WHERE __row={physical_row}");
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let row = rows
            .next()
            .map_err(|e| e.to_string())?
            .ok_or(format!("表头原始行 {physical_row} 不存在"))?;
        let mut cells = Vec::new();
        for index in 0..(layout.last_col - layout.first_col + 1) as usize {
            cells.push(
                row.get::<_, Option<String>>(index)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default(),
            );
        }
        header_rows.insert(*physical_row, cells);
    }
    let merged_bounds = merges
        .iter()
        .map(|&(r1, r2, c1, c2)| (r1 as usize, r2 as usize, c1 as u32, c2 as u32))
        .collect::<Vec<_>>();
    let (described, issues) =
        crate::source_structure::fields(layout, &header_rows, &merged_bounds)?;
    if !issues.is_empty() {
        return Err(issues.join("；"));
    }
    let headers = described
        .into_iter()
        .map(|field| field.name)
        .collect::<Vec<_>>();
    validate_headers(&headers)?;
    let mut unique = headers
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for col in &layout.fill_merged_data_cols {
        let flag = format!(
            "{}__filled_from_merge",
            headers[(col - layout.first_col) as usize]
        );
        if !unique.insert(flag) {
            return Err("补值标记列与现有表头冲突".into());
        }
    }
    let fill_table = sql_name(&format!("{}__merge_fill", n(&node.id)?));
    execute(
        conn,
        &format!(
            "CREATE TABLE {fill_table} (row_start BIGINT,row_end BIGINT,col BIGINT,anchor VARCHAR)"
        ),
    )?;
    let mut fill_appender = conn
        .appender(fill_table.trim_matches('"'))
        .map_err(|e| e.to_string())?;
    for &(row_start, row_end, col_start, col_end) in &merges {
        if row_end < layout.data_start_row as i64
            || layout
                .data_end_row
                .is_some_and(|end| row_start > end as i64)
        {
            continue;
        }
        if !layout
            .fill_merged_data_cols
            .iter()
            .any(|col| (*col as i64) >= col_start && (*col as i64) <= col_end)
        {
            continue;
        }
        if col_start != col_end {
            return Err(format!(
                "数据区第 {row_start} 至 {row_end} 行存在横向合并，不能按纵向补值"
            ));
        }
        if row_start < layout.data_start_row as i64
            || layout.data_end_row.is_some_and(|end| row_end > end as i64)
        {
            return Err(format!(
                "数据区合并区域第 {row_start} 至 {row_end} 行跨越数据边界"
            ));
        }
        let anchor: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT {} FROM {input} WHERE __row={row_start}",
                    sql_name(&format!("col_{col_start}"))
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let anchor = anchor.unwrap_or_default();
        if anchor.trim().is_empty() {
            return Err(format!("合并区域第 {row_start} 行的原始锚点为空，不能补值"));
        }
        let values: [&dyn duckdb::ToSql; 4] = [&row_start, &row_end, &col_start, &anchor];
        fill_appender
            .append_row(values)
            .map_err(|e| e.to_string())?;
    }
    fill_appender.flush().map_err(|e| e.to_string())?;
    let mut select = vec![
        "t.__company".to_string(),
        "t.__file".into(),
        "t.__sheet".into(),
        "t.__row".into(),
        "row_number() OVER (ORDER BY t.__row) AS __record".into(),
        "t.__role".into(),
    ];
    for (offset, header) in headers.iter().enumerate() {
        let col = layout.first_col + offset as u32;
        let raw = format!("t.{}", sql_name(&format!("col_{col}")));
        if layout.fill_merged_data_cols.contains(&col) {
            let fill=format!("(SELECT m.anchor FROM {fill_table} m WHERE m.col={col} AND t.__row>m.row_start AND t.__row<=m.row_end LIMIT 1)");
            select.push(format!(
                "COALESCE(NULLIF({raw},''),{fill},{raw}) AS {}",
                sql_name(header)
            ));
            select.push(format!(
                "({raw}='' AND {fill} IS NOT NULL) AS {}",
                sql_name(&format!("{header}__filled_from_merge"))
            ));
        } else {
            select.push(format!("{raw} AS {}", sql_name(header)));
        }
    }
    let end = layout
        .data_end_row
        .map_or(String::new(), |row| format!(" AND t.__row<={row}"));
    execute(
        conn,
        &format!(
            "CREATE TABLE {out} AS SELECT {} FROM {input} t WHERE t.__row>={}{} ORDER BY t.__row",
            select.join(","),
            layout.data_start_row,
            end
        ),
    )?;
    Ok(())
}

pub(crate) fn input_ids<'a>(wf: &'a Workflow, node: &'a crate::model::Node) -> Result<Vec<&'a str>, String> {
    let mut input_ids = incoming(wf, &node.id);
    if node.kind == "join" && node.version == 2 {
        return Ok(crate::join_append::ordered_inputs(wf, node, &["left", "right"])?
            .into_iter().map(|edge| edge.source.as_str()).collect());
    } else if node.kind == "join" {
        input_ids = vec![
            required_str(&node.config, "left_id")?,
            required_str(&node.config, "right_id")?,
        ];
    } else if node.kind == "union" && node.version == 2 {
        let rule = crate::join_append::AppendRule::read(node)?;
        let ports = rule.ports.iter().map(String::as_str).collect::<Vec<_>>();
        return Ok(crate::join_append::ordered_inputs(wf, node, &ports)?
            .into_iter().map(|edge| edge.source.as_str()).collect());
    } else if node.kind == "union" {
        // Saved canvas ports determine which input is called left or right,
        // regardless of the order in which edges appear in the JSON file.
        let edges: Vec<_> = wf.edges.iter().filter(|e| e.target == node.id).collect();
        if edges.len() == 2 && edges.iter().all(|edge| edge.target_port.is_some()) {
            return Ok(crate::join_append::ordered_inputs(wf, node, &["left", "right"])?
                .into_iter().map(|edge| edge.source.as_str()).collect());
        }
        if let Some(left) = edges
            .iter()
            .find(|e| e.target_port.as_deref() == Some("left"))
        {
            let right = edges
                .iter()
                .find(|e| e.id != left.id)
                .ok_or("追加缺少右侧输入")?;
            input_ids = vec![left.source.as_str(), right.source.as_str()];
        } else if let Some(right) = edges
            .iter()
            .find(|e| e.target_port.as_deref() == Some("right"))
        {
            let left = edges
                .iter()
                .find(|e| e.id != right.id)
                .ok_or("追加缺少左侧输入")?;
            input_ids = vec![left.source.as_str(), right.source.as_str()];
        }
    }
    Ok(input_ids)
}

fn process_node(
    conn: &Connection,
    wf: &Workflow,
    company: &CompanyConfig,
    node: &crate::model::Node,
) -> Result<(), String> {
    let out = table(&node.id)?;
    let input_ids = input_ids(wf, node)?;
    let input = if input_ids.is_empty() {
        String::new()
    } else {
        table(input_ids[0])?
    };
    let cfg = &node.config;
    if ["decimal","text_extract","text_clean","filter","derive","sort","duplicates","deduplicate"].contains(&node.kind.as_str()) || (node.kind=="aggregate" && node.version==2) {
        let fields=column_types(conn,&n(input_ids[0])?)?.into_iter().map(|(name,data_type)|crate::model::FieldDescriptor{name,data_type,nullable:true,origin:input_ids[0].into(),confirmed:true}).collect::<Vec<_>>();
        crate::field_contract::check(node,&fields)?;
    }
    match node.kind.as_str() {
        "pivot" => {
            crate::pivot::execute(conn,node,&n(input_ids[0])?,&n(&node.id)?)?;
        }
        "sort" | "duplicates" | "deduplicate" | "sequence" => {
            let fields = descriptors(conn, &n(input_ids[0])?, input_ids[0])?;
            crate::table_ops::execute(conn, node, &input, &out, &fields)?;
        }
        "aggregate" if node.version == 2 => {
            let fields = descriptors(conn, &n(input_ids[0])?, input_ids[0])?;
            crate::table_ops::execute(conn, node, &input, &out, &fields)?;
        }
        "derive" => {
            let fields = descriptors(conn, &n(input_ids[0])?, input_ids[0])?;
            let formula = crate::formula::compile_for_table(required_str(cfg,"expression")?, &fields, conn, &input)?;
            let target = required_str(cfg,"target")?;
            let error_name = format!("{target}__error");
            execute(conn, &format!("CREATE TABLE {out} AS SELECT *,({}) AS {},({}) AS {} FROM {input}",formula.sql,sql_name(target),formula.error,sql_name(&error_name)))?;
            execute(conn, &format!("CREATE TABLE {} AS SELECT * FROM {out} WHERE {} IS NOT NULL",sql_name(&format!("{}__exceptions",n(&node.id)?)),sql_name(&error_name)))?;
        }
        "source" => {
            let key = required_str(cfg, "source_key")?;
            let binding = &company.sources[key];
            if cfg.get("normalize_structure").and_then(Value::as_bool) == Some(true) {
                let raw_id = format!("{}__raw", node.id);
                let raw = table(&raw_id)?;
                // Keep the original physical grid in the run database for review.
                inspect_normalized_source(binding)?;
                import_source(conn, &raw, binding, &company.company, key)?;
                process_structure(conn, node, binding, &raw_id, &raw, &out)?;
            } else {
                import_source(conn, &out, binding, &company.company, key)?;
            }
        }
        "map" => {
            let input_columns = columns(conn, &n(input_ids[0])?)?;
            let mut select:Vec<String> = input_columns.iter().filter(|s|(s.starts_with("__")&&s.as_str()!="__company")||s.as_str()=="company").map(|s|sql_name(s)).collect();
            if !input_columns.iter().any(|s|s=="company") && input_columns.iter().any(|s|s=="__company") {
                select.push("__company AS company".into());
            }
            if let Some(mappings) = cfg.get("mappings") {
                let mappings = mappings.as_array().ok_or("mappings 必须是数组")?;
                if mappings.is_empty() {
                    return Err("请至少保留一个字段".into());
                }
                let available = columns(conn, &n(input_ids[0])?)?;
                let mut targets = std::collections::BTreeSet::new();
                for mapping in mappings {
                    let source = required_str(mapping, "source")?;
                    let target = required_str(mapping, "target")?;
                    if !available.iter().any(|column| column == source) {
                        return Err(format!("上一步缺少字段：{source}"));
                    }
                    if target.trim().is_empty()
                        || target.chars().count() > 100
                        || !targets.insert(target)
                        || target.starts_with("__")
                        || target == "company"
                    {
                        return Err(format!("输出字段重名或使用保留名称：{target}"));
                    }
                    select.push(format!("{} AS {}", sql_name(source), sql_name(target)));
                }
            } else {
                let key = required_str(cfg, "source_key")?;
                let binding = company
                    .sources
                    .get(key)
                    .ok_or(format!("缺少 {key} 的字段配置"))?;
                for field in required_strings(cfg, "fields")? {
                    identifier(&field)?;
                    let original = binding
                        .fields
                        .get(&field)
                        .ok_or(format!("{} 的字段映射缺少 {field}", company.company))?;
                    select.push(format!("{} AS {}", sql_name(original), sql_name(&field)));
                }
            }
            execute(
                conn,
                &format!(
                    "CREATE TABLE {out} AS SELECT {} FROM {input}",
                    select.join(", ")
                ),
            )?;
        }
        "structure" => {
            let key = required_str(cfg, "source_key")?;
            let binding = company
                .sources
                .get(key)
                .ok_or(format!("缺少 {key} 的资料绑定"))?;
            process_structure(conn, node, binding, input_ids[0], &input, &out)?;
        }
        "text_clean" => {
            let mode=required_str(cfg,"mode")?;
            let mut expressions=Vec::new();
            for f in config_array(cfg,"fields")? {let source=sql_name(required_str(f,"source")?);let target=sql_name(required_str(f,"target")?);let expr=if mode=="trim"{format!("trim(CAST({source} AS VARCHAR), ' ' || chr(9) || chr(10) || chr(13))")}else{format!("replace(CAST({source} AS VARCHAR), {}, {})",lit(required_str(cfg,"find")?),lit(cfg.get("replacement").and_then(Value::as_str).ok_or("缺少替换内容")?))};expressions.push(format!("{expr} AS {target}"));}
            execute(conn,&format!("CREATE TABLE {out} AS SELECT *, {} FROM {input}",expressions.join(",")))?;
        }
        "decimal" if node.version == 2 => {
            let mut expressions = Vec::new();
            let mut pending = Vec::new();
            let source_columns = columns(conn, &n(input_ids[0])?)?
                .into_iter().filter(|name| name != "__error")
                .map(|name| sql_name(&name)).collect::<Vec<_>>();
            expressions.extend(source_columns);
            for item in config_array(cfg, "fields")? {
                let source = sql_name(required_str(item, "source")?);
                let target = required_str(item, "target")?;
                let text = amount_text(item,&source);
                let empty = format!("({text} IS NULL OR {text} = '')");
                let precision = format!("regexp_matches({text}, '[.][0-9]{{5,}}|[eE][+-]?[0-9]+')");
                let numeric = format!("regexp_full_match({text}, '[+-]?([0-9]+([.][0-9]{{0,4}})?|[.][0-9]{{1,4}})')");
                let cast = format!("TRY_CAST({text} AS DECIMAL(28,4))");
                let zero = item.get("empty_as_zero").and_then(Value::as_bool).unwrap_or(false);
                let blank_value = if zero { "CAST(0 AS DECIMAL(28,4))" } else { "NULL" };
                let blank_status = if zero { "空白补零" } else { "空白" };
                let status = sql_name(&format!("{target}__amount_status"));
                expressions.push(format!("CASE WHEN {empty} THEN {blank_value} WHEN {precision} OR NOT {numeric} THEN NULL ELSE {cast} END AS {}", sql_name(target)));
                expressions.push(format!("CASE WHEN {empty} THEN '{blank_status}' WHEN {precision} THEN '精度或科学计数法' WHEN NOT {numeric} THEN '格式无法转换' WHEN {cast} IS NULL THEN '超出金额范围' ELSE '有效' END AS {status}"));
                pending.push(format!("{status} NOT IN ('有效', '空白补零')"));
            }
            execute(conn, &format!("CREATE TABLE {out} AS SELECT {} FROM {input}", expressions.join(", ")))?;
            let exceptions = sql_name(&format!("{}__exceptions", n(&node.id)?));
            execute(conn, &format!("CREATE TABLE {exceptions} AS SELECT * FROM {out} WHERE {}", pending.join(" OR ")))?;
        }
        "decimal" => {
            let fields = config_array(cfg, "fields")?;
            let mut columns = Vec::new();
            let mut errors = Vec::new();
            for item in fields {
                let source = required_str(item, "source")?;
                let target = required_str(item, "target")?;


                let source_name = sql_name(source);
                // An implicit DECIMAL cast rounds excess fractional digits. Keep the
                // original text in an exception until a precision rule is explicit.
                let excess_precision = format!(
                    "regexp_matches(TRIM(CAST({source_name} AS VARCHAR)), '[.][0-9]{{5,}}|[eE][+-]?[0-9]+')"
                );
                columns.push(format!(
                    "CASE WHEN {excess_precision} THEN NULL ELSE TRY_CAST({source_name} AS DECIMAL(28,4)) END AS {}",
                    sql_name(target)
                ));
                errors.push(format!("CASE WHEN {source_name} IS NULL OR TRIM(CAST({source_name} AS VARCHAR)) = '' THEN {} WHEN {excess_precision} THEN {} WHEN TRY_CAST({source_name} AS DECIMAL(28,4)) IS NULL THEN {} ELSE NULL END", lit(&format!("{source}:金额为空")), lit(&format!("{source}:金额精度超过4位或使用科学计数法")), lit(&format!("{source}:金额无法转换"))));
            }
            if fields.is_empty() {
                return Err("金额转换节点缺少字段".into());
            }
            let base_columns=column_types(conn,&n(input_ids[0])?)?.into_keys().filter(|c|c!="__error").map(|c|sql_name(&c)).collect::<Vec<_>>().join(",");
            let classified = format!(
                "(SELECT {base_columns}, {}, concat_ws(';', {}) AS __error FROM {input})",
                columns.join(","),
                errors.join(",")
            );
            execute(
                conn,
                &format!(
                    "CREATE TABLE {} AS SELECT * FROM {classified} WHERE __error <> ''",
                    sql_name(&format!("{}__exceptions", n(&node.id)?))
                ),
            )?;
            execute(
                conn,
                &format!("CREATE TABLE {out} AS SELECT * FROM {classified} WHERE __error = ''"),
            )?;
        }
        "text_extract" => {
            let source = required_str(cfg, "source")?;
            let target = required_str(cfg, "target")?;
            let pattern = required_str(cfg, "pattern")?;


            if pattern.len() > 1000 {
                return Err("提取规则超过 1000 字节，请缩短后重试".into());
            }
            let group = cfg
                .get("group")
                .map_or(Some(1), |v| v.as_u64())
                .filter(|v| *v <= 99)
                .ok_or("提取分组须为 0 至 99 的整数")?;
            let matched = format!("{target}__matched");
            crate::field_contract::business_name(&matched)?;
            let input_columns = column_types(conn, &n(input_ids[0])?)?;
            if !input_columns.contains_key(source) {
                return Err(format!("提取源字段 {source} 不存在"));
            }
            if input_columns.contains_key(target) || input_columns.contains_key(&matched) {
                return Err(format!("提取目标字段 {target} 已存在；请选择新的目标字段"));
            }
            let extracted = format!(
                "NULLIF(TRIM(regexp_extract(CAST({} AS VARCHAR), {}, {group})), '')",
                sql_name(source),
                lit(pattern)
            );
            execute(conn, &format!(
                "CREATE TABLE {out} AS SELECT *, {extracted} AS {}, ({extracted} IS NOT NULL) AS {} FROM {input}",
                sql_name(target), sql_name(&matched)
            ))?;
            execute(
                conn,
                &format!(
                    "CREATE TABLE {} AS SELECT * FROM {out} WHERE {} IS NOT TRUE",
                    sql_name(&format!("{}__exceptions", n(&node.id)?)),
                    sql_name(&matched)
                ),
            )?;
        }
        "mark" => {
            let field = required_str(cfg, "field")?;
            let target = required_str(cfg, "target")?;
            identifier(field)?;
            identifier(target)?;
            let values = required_strings(cfg, "values")?;
            if values.is_empty() {
                return Err("标记值不能为空".into());
            }
            let vals = values.iter().map(|s| lit(s)).collect::<Vec<_>>().join(",");
            execute(
                conn,
                &format!(
                    "CREATE TABLE {out} AS SELECT *, ({} IN ({vals})) AS {} FROM {input}",
                    sql_name(field),
                    sql_name(target)
                ),
            )?;
        }
        "filter" => {
            if let Some(expected)=cfg.get("selected_fingerprints"){
                let expected=expected.as_object().ok_or("来源记录文件指纹格式无效")?;
                for (key,hash) in expected{
                    let source=company.sources.get(key).ok_or("选中记录的来源绑定已变化，请重新选择")?;
                    if hash.as_str()!=Some(hash_file(Path::new(&source.path))?.as_str()){
                        return Err("选中记录的来源文件已变化，请重新选择记录".into());
                    }
                }
            }
            if let Some(conditions)=cfg.get("conditions").and_then(Value::as_array){
                for condition in conditions.iter().filter(|condition|matches!(condition.get("operator").and_then(Value::as_str),Some("include_records"|"exclude_records"))){
                    let expected=condition.get("records").and_then(Value::as_array).ok_or("来源记录条件格式无效")?.len() as i64;
                    let mut positive=condition.clone();positive["operator"]=serde_json::json!("include_records");
                    let predicate=crate::table_ops::predicate(&serde_json::json!({"condition":positive}),&descriptors(conn,&n(input_ids[0])?,input_ids[0])?)?;
                    let actual=query_i64(conn,&format!("SELECT COUNT(*) FROM {input} WHERE {predicate}"))?;
                    if actual!=expected{return Err("选中来源记录已缺失、重复或换了绑定，请重新选择记录".into());}
                }
            }
            let condition = if cfg.get("conditions").is_some()
                || cfg.get("condition").and_then(|condition|condition.get("operator")).and_then(Value::as_str).is_some_and(|op|op.starts_with("date_"))
            {
                crate::table_ops::predicate(cfg,&descriptors(conn,&n(input_ids[0])?,input_ids[0])?)?
            } else if let Some(predicate) = cfg.get("condition") {
                let field = required_str(predicate, "field")?;
                let available = columns(conn, &n(input_ids[0])?)?;
                if !available.iter().any(|column| column == field) {
                    return Err(format!("上一步缺少筛选字段：{field}"));
                }
                let column = sql_name(field);
                match required_str(predicate, "operator")? {
                    op if op.starts_with("numeric_") => {let cmp=match op{"numeric_gt"=>">","numeric_gte"=>">=","numeric_lt"=>"<","numeric_lte"=>"<=","numeric_eq"=>"=","numeric_neq"=>"<>",_=>return Err("未知数值比较".into())};format!("{column} {cmp} CAST({} AS DECIMAL(28,4))",lit(required_str(predicate,"value")?))},
                    "equals" => format!(
                        "CAST({column} AS VARCHAR) = {}",
                        lit(required_str(predicate, "value")?)
                    ),
                    "not_equals" => format!(
                        "CAST({column} AS VARCHAR) <> {}",
                        lit(required_str(predicate, "value")?)
                    ),
                    "contains" => format!(
                        "contains(CAST({column} AS VARCHAR), {})",
                        lit(required_str(predicate, "value")?)
                    ),
                    "is_empty" => {
                        format!("{column} IS NULL OR TRIM(CAST({column} AS VARCHAR)) = ''")
                    }
                    "is_not_empty" => {
                        format!("{column} IS NOT NULL AND TRIM(CAST({column} AS VARCHAR)) <> ''")
                    }
                    other => return Err(format!("不支持的筛选条件：{other}")),
                }
            } else {
                let field = required_str(cfg, "field")?;
                identifier(field)?;
                format!("{} IS TRUE", sql_name(field))
            };
            execute(
                conn,
                &format!(
                    "CREATE TABLE {} AS SELECT * FROM {input} WHERE ({condition}) IS NOT TRUE",
                    sql_name(&format!("{}__excluded", n(&node.id)?)),
                ),
            )?;
            execute(
                conn,
                &format!("CREATE TABLE {out} AS SELECT * FROM {input} WHERE ({condition}) IS TRUE"),
            )?;
        }
        "union" if node.version == 2 => {
            let tables = input_ids.iter().map(|id| n(id)).collect::<Result<Vec<_>, _>>()?;
            crate::join_append::execute_append(conn, node, &tables, &n(&node.id)?)?;
        }
        "union" => {
            let right = table(input_ids[1])?;
            let left_schema = column_types(conn, &n(input_ids[0])?)?;
            let right_schema = column_types(conn, &n(input_ids[1])?)?;
            let left_set: std::collections::BTreeSet<_> = left_schema.keys().collect();
            let right_set: std::collections::BTreeSet<_> = right_schema.keys().collect();
            if left_set != right_set {
                return Err(format!(
                    "同类追加的字段不一致；左侧 {left_set:?}，右侧 {right_set:?}。请先映射字段"
                ));
            }
            for (column, left_type) in &left_schema {
                let right_type = &right_schema[column];
                if left_type != right_type {
                    return Err(format!(
                        "同类追加的字段 {column} 类型不一致：左侧 {left_type}，右侧 {right_type}。请先统一字段类型，避免金额或日期被自动转换"
                    ));
                }
            }
            execute(conn, &format!("CREATE TABLE {out} AS SELECT * FROM {input} UNION ALL BY NAME SELECT * FROM {right}"))?;
        }
        "aggregate" => {
            let group = keys(cfg)?;
            let amount = required_str(cfg, "amount")?;
            let target = required_str(cfg, "target")?;
            identifier(amount)?;
            identifier(target)?;
            let null_amount = query_i64(
                conn,
                &format!(
                    "SELECT COUNT(*) FROM {input} WHERE {} IS NULL",
                    sql_name(amount)
                ),
            )?;
            if null_amount > 0 {
                return Err(format!("汇总金额有 {null_amount} 行为空；不能静默忽略"));
            }
            let select_keys = group
                .iter()
                .map(|k| sql_name(k))
                .collect::<Vec<_>>()
                .join(",");
            execute(conn, &format!("CREATE TABLE {out} AS SELECT {select_keys}, SUM({}) AS {}, COUNT(*) AS __members FROM {input} GROUP BY {select_keys}", sql_name(amount), sql_name(target)))?;
        }
        "join" if node.version == 2 => {
            crate::join_append::execute_join(conn, node, &n(input_ids[0])?, &n(input_ids[1])?, &n(&node.id)?)?;
        }
        "join" => {
            let right = table(input_ids[1])?;
            let group = keys(cfg)?;
            let left_amount = required_str(cfg, "left_amount")?;
            let right_amount = required_str(cfg, "right_amount")?;
            identifier(left_amount)?;
            identifier(right_amount)?;
            let checks = group
                .iter()
                .map(|k| format!("l.{} = r.{}", sql_name(k), sql_name(k)))
                .collect::<Vec<_>>()
                .join(" AND ");
            for side in [&input, &right] {
                let missing = query_i64(
                    conn,
                    &format!(
                        "SELECT COUNT(*) FROM {side} WHERE {}",
                        group
                            .iter()
                            .map(|k| format!(
                                "{} IS NULL OR TRIM(CAST({} AS VARCHAR)) = ''",
                                sql_name(k),
                                sql_name(k)
                            ))
                            .collect::<Vec<_>>()
                            .join(" OR ")
                    ),
                )?;
                if missing > 0 {
                    return Err(format!("关联前发现 {missing} 行匹配键为空，不能静默关联"));
                }
                let duplicate = query_i64(conn, &format!("SELECT COUNT(*) FROM (SELECT 1 FROM {side} GROUP BY {} HAVING COUNT(*) > 1)", group.iter().map(|k| sql_name(k)).collect::<Vec<_>>().join(",")))?;
                if duplicate > 0 {
                    return Err(format!(
                        "关联前发现 {duplicate} 个重复键组；请先明确汇总或处理重复键"
                    ));
                }
            }
            let selects = group
                .iter()
                .map(|k| format!("COALESCE(l.{0},r.{0}) AS {0}", sql_name(k)))
                .collect::<Vec<_>>()
                .join(",");
            let first_key = sql_name(&group[0]);
            let issue_ids = issue_sources(cfg)?;
            let (issue_join, issue_expr) = if !issue_ids.is_empty() {
                let issue_union = issue_ids
                    .iter()
                    .map(|id| {
                        Ok(format!(
                            "SELECT {} FROM {}",
                            group
                                .iter()
                                .map(|k| sql_name(k))
                                .collect::<Vec<_>>()
                                .join(","),
                            sql_name(&format!("{}__exceptions", n(id)?))
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .join(" UNION ALL ");
                let group_keys = group
                    .iter()
                    .map(|k| sql_name(k))
                    .collect::<Vec<_>>()
                    .join(",");
                let on = group
                    .iter()
                    .map(|k| format!("x.{0} = COALESCE(l.{0},r.{0})", sql_name(k)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                (format!("LEFT JOIN (SELECT {group_keys}, COUNT(*) AS issue_count FROM ({issue_union}) issue_rows GROUP BY {group_keys}) x ON {on}"), "COALESCE(x.issue_count,0)".to_string())
            } else {
                (String::new(), "0".to_string())
            };
            execute(conn, &format!("CREATE TABLE {out} AS SELECT {selects}, l.{} AS left_amount, r.{} AS right_amount, COALESCE(l.{0},0)-COALESCE(r.{1},0) AS difference, {issue_expr} AS issue_count, CASE WHEN l.{first_key} IS NULL THEN '仅右侧' WHEN r.{first_key} IS NULL THEN '仅左侧' WHEN l.{0}=r.{1} AND {issue_expr}>0 THEN '数值相符但待复核' WHEN l.{0}=r.{1} THEN '数值相符' ELSE '金额不符' END AS match_status FROM {input} l FULL OUTER JOIN {right} r ON {checks} {issue_join}", sql_name(left_amount), sql_name(right_amount)))?;
        }
        "inspect" | "export" => {
            execute(
                conn,
                &format!("CREATE TABLE {out} AS SELECT * FROM {input}"),
            )?;
        }
        _ => return Err(format!("不支持的节点：{}", node.kind)),
    }
    Ok(())
}
fn review(conn: &Connection, node: &crate::model::Node) -> Result<NodeReview, String> {
    let name = n(&node.id)?;
    let tbl = sql_name(&name);
    let rows = query_i64(conn, &format!("SELECT COUNT(*) FROM {tbl}"))?;
    let columns = columns(conn, &name)?;
    let types = column_types(conn, &name)?;
    let amount = ["net", "amount", "difference"].iter().find(|f| {
        columns.contains(&f.to_string())
            && types.get(**f).is_some_and(|kind| {
                kind.starts_with("DECIMAL")
                    || matches!(
                        kind.as_str(),
                        "TINYINT"
                            | "SMALLINT"
                            | "INTEGER"
                            | "BIGINT"
                            | "HUGEINT"
                            | "UTINYINT"
                            | "USMALLINT"
                            | "UINTEGER"
                            | "UBIGINT"
                            | "UHUGEINT"
                            | "FLOAT"
                            | "DOUBLE"
                    )
            })
    });
    let amount_total = match amount {
        Some(field) => conn
            .query_row(
                &format!(
                    "SELECT CAST(SUM({}) AS VARCHAR) FROM {tbl}",
                    sql_name(field)
                ),
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|e| e.to_string())?,
        None => None,
    };
    let exceptions = if node.kind == "decimal" || node.kind == "text_extract" || node.kind == "derive" {
        query_i64(
            conn,
            &format!(
                "SELECT COUNT(*) FROM {}",
                sql_name(&format!("{}__exceptions", name))
            ),
        )?
    } else {
        0
    };
    let excluded = if node.kind == "filter" || node.kind == "deduplicate" {
        query_i64(
            conn,
            &format!(
                "SELECT COUNT(*) FROM {}",
                sql_name(&format!("{}__excluded", name))
            ),
        )?
    } else {
        0
    };
    let (matched, left_only, right_only, unresolved) = if node.kind == "join" {
        (
            Some(query_i64(conn,&format!("SELECT COUNT(*) FILTER (WHERE match_status NOT IN ('仅左侧','仅右侧')) FROM {tbl}"))?),
            Some(query_i64(conn,&format!("SELECT COUNT(*) FILTER (WHERE match_status = '仅左侧') FROM {tbl}"))?),
            Some(query_i64(conn,&format!("SELECT COUNT(*) FILTER (WHERE match_status = '仅右侧') FROM {tbl}"))?),
            if node.version == 1 {
                Some(query_i64(conn,&format!("SELECT COUNT(*) FILTER (WHERE issue_count > 0) FROM {tbl}"))?)
            } else {
                None
            },
        )
    } else {
        (None, None, None, None)
    };
    Ok(NodeReview {
        node_id: node.id.clone(),
        label: node.label.clone(),
        rows,
        amount_total,
        exceptions,
        excluded,
        table: name,
        matched,
        left_only,
        right_only,
        unresolved,
    })
}
pub(crate) fn columns(conn: &Connection, table_name: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {} LIMIT 0", sql_name(table_name)))
        .map_err(|e| e.to_string())?;
    // duckdb-rs initializes the Arrow schema only after the statement is queried.
    let _ = stmt.query([]).map_err(|e| e.to_string())?;
    Ok(stmt.column_names())
}
pub(crate) fn column_types(conn: &Connection, table_name: &str) -> Result<BTreeMap<String, String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", lit(table_name)))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|e| e.to_string())
}
pub fn preview(db_path: &str, table_name: &str, limit: usize) -> Result<Preview, String> {
    identifier(table_name)?;
    let _guard = result_query_guard()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    preview_query(&conn,table_name,"TRUE",limit)
}

pub fn preview_page(db_path: &str, table_name: &str, offset: usize, limit: usize) -> Result<Preview, String> {
    identifier(table_name)?;
    let _guard = result_query_guard()?;
    let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
    preview_query_at(&conn, table_name, "TRUE", limit.min(500), offset, true)
}
pub(crate) fn descriptors(conn:&Connection,table_name:&str,origin:&str)->Result<Vec<crate::model::FieldDescriptor>,String>{
    let types=column_types(conn,table_name)?;
    Ok(columns(conn,table_name)?.into_iter().map(|name|crate::model::FieldDescriptor{data_type:types[&name].clone(),name,nullable:true,origin:origin.into(),confirmed:true}).collect())
}
pub(crate) fn result_order(conn:&Connection,table_name:&str,cols:&[String])->String{
    let mut order=Vec::new();
    if cols.iter().any(|column|column=="__order"){order.push(sql_name("__order"));}
    if cols.iter().any(|column|column.eq_ignore_ascii_case("rowid"))
        || conn.prepare(&format!("SELECT rowid FROM {} LIMIT 0",sql_name(table_name))).is_err()
    {
        order.extend(cols.iter().filter(|name|name.as_str()!="__order").map(|name|sql_name(name)));
    }else{order.push("rowid".into());}
    order.join(", ")
}
pub(crate) fn preview_query(conn:&Connection,table_name:&str,predicate:&str,limit:usize)->Result<Preview,String>{
    preview_query_at(conn, table_name, predicate, limit.min(200), 0, false)
}

fn preview_query_at(conn: &Connection, table_name: &str, predicate: &str, limit: usize, offset: usize, stable: bool) -> Result<Preview, String> {
    let cols = columns(&conn, table_name)?;
    let selects = cols
        .iter()
        .map(|c| format!("CAST({} AS VARCHAR)", sql_name(c)))
        .collect::<Vec<_>>()
        .join(",");
    let count = query_i64(
        &conn,
        &format!("SELECT COUNT(*) FROM {} WHERE {predicate}", sql_name(table_name)),
    )?;
    let order=if stable {format!("ORDER BY {}",result_order(conn,table_name,&cols))}
        else if cols.iter().any(|name|name=="__order") {"ORDER BY __order".into()} else {String::new()};
    let sql = format!("SELECT {selects} FROM {} WHERE {predicate} {order} LIMIT ? OFFSET ?", sql_name(table_name));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map(duckdb::params![limit as i64, i64::try_from(offset).unwrap_or(i64::MAX)], |row| {
            (0..cols.len())
                .map(|i| {
                    row.get::<_, Option<String>>(i)
                        .map(|v| v.unwrap_or_default())
                })
                .collect::<duckdb::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(row.map_err(|e| e.to_string())?);
    }
    Ok(Preview {
        columns: cols,
        rows,
        total_rows: count,
    })
}
fn preview_with_filter(
    conn: &Connection,
    table_name: &str,
    group: &BTreeMap<String, String>,
) -> Result<Preview, String> {
    let cols = columns(conn, table_name)?;
    for key in group.keys() {
        identifier(key)?;
        if !cols.contains(key) {
            return Err(format!("来源表缺少追溯键 {key}"));
        }
    }
    let filter = group
        .iter()
        .map(|(k, v)| format!("{} = {}", sql_name(k), lit(v)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let tbl = sql_name(table_name);
    let count = query_i64(conn, &format!("SELECT COUNT(*) FROM {tbl} WHERE {filter}"))?;
    let select = cols
        .iter()
        .map(|c| format!("CAST({} AS VARCHAR)", sql_name(c)))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT {select} FROM {tbl} WHERE {filter} ORDER BY __file, __row LIMIT 200");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map([], |row| {
            (0..cols.len())
                .map(|i| {
                    row.get::<_, Option<String>>(i)
                        .map(|v| v.unwrap_or_default())
                })
                .collect::<duckdb::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for item in mapped {
        rows.push(item.map_err(|e| e.to_string())?);
    }
    Ok(Preview {
        columns: cols,
        rows,
        total_rows: count,
    })
}
pub fn trace_join(
    db_path: &str,
    wf: &Workflow,
    join_id: &str,
    group: &BTreeMap<String, String>,
) -> Result<Vec<TracePart>, String> {
    let _guard = result_query_guard()?;
    let join = wf
        .nodes
        .iter()
        .find(|n| n.id == join_id && n.kind == "join")
        .ok_or("请选择关联节点")?;
    let group_keys = keys(&join.config)?;
    if group_keys.iter().any(|k| !group.contains_key(k)) || group.len() != group_keys.len() {
        return Err("追溯键须与关联键完全一致".into());
    }
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let inputs = vec![
        required_str(&join.config, "left_id")?,
        required_str(&join.config, "right_id")?,
    ];
    let mut parts = Vec::new();
    for (index, aggregate_id) in inputs.iter().enumerate() {
        let aggregate = wf
            .nodes
            .iter()
            .find(|n| n.id == *aggregate_id)
            .ok_or("关联输入节点缺失")?;
        if aggregate.kind != "aggregate" {
            return Err("当前来源反查需要关联两侧先按键汇总".into());
        }
        let detail_id = incoming(wf, aggregate_id)
            .first()
            .copied()
            .ok_or("汇总来源缺失")?;
        parts.push(TracePart {
            label: if index == 0 {
                "左侧明细".into()
            } else {
                "右侧明细".into()
            },
            preview: preview_with_filter(&conn, &n(detail_id)?, group)?,
        });
    }
    for issue_id in issue_sources(&join.config)? {
        parts.push(TracePart {
            label: format!("{} 同组异常", issue_id),
            preview: preview_with_filter(&conn, &format!("{}__exceptions", n(&issue_id)?), group)?,
        });
    }
    Ok(parts)
}
fn write_csv(conn: &Connection, table_name: &str, path: &Path) -> Result<(), String> {
    let cols = columns(conn, table_name)?;
    let mut writer = csv::Writer::from_path(path).map_err(|e| e.to_string())?;
    writer.write_record(&cols).map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT {} FROM {} {}",
        cols.iter()
            .map(|c| format!("CAST({} AS VARCHAR)", sql_name(c)))
            .collect::<Vec<_>>()
            .join(","),
        sql_name(table_name),
        if cols.iter().any(|s|s=="__order"){"ORDER BY __order"}else{""}
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map([], |row| {
            (0..cols.len())
                .map(|i| {
                    row.get::<_, Option<String>>(i)
                        .map(|v| v.unwrap_or_default())
                })
                .collect::<duckdb::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    for row in mapped {
        writer
            .write_record(row.map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())
}
pub fn run(wf: &Workflow, company: &CompanyConfig, output_root: &str) -> Result<RunResult, String> {
    run_internal(wf, company, output_root, true)
}

fn verify_input_fingerprints(
    company: &CompanyConfig,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, fingerprint) in expected {
        let binding = company.sources.get(key).ok_or_else(|| format!("缺少输入资料 {key}"))?;
        let actual = hash_file(Path::new(&binding.path))
            .map_err(|error| format!("无法复核输入资料 {key}：{error}"))?;
        if &actual != fingerprint {
            return Err(format!("输入资料 {key} 在计算期间发生变化，请重新运行。"));
        }
    }
    Ok(())
}

/// Execute the same node rules without writing any export/exception CSV files.
pub fn process(
    wf: &Workflow,
    company: &CompanyConfig,
    work_root: &str,
) -> Result<RunResult, String> {
    run_internal(wf, company, work_root, false)
}

fn run_internal(
    wf: &Workflow,
    company: &CompanyConfig,
    output_root: &str,
    export_files: bool,
) -> Result<RunResult, String> {
    let order = ordered_nodes(wf, company)?;
    let root = Path::new(output_root);
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let label: String = company
        .company
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(48)
        .collect();
    let label = if label.is_empty() { "company" } else { &label };
    let dir: PathBuf = root.join(format!("{}_{}", label, stamp));
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("run.duckdb");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    let temp_dir = dir.join("duckdb-temp");
    fs::create_dir(&temp_dir).map_err(|e| e.to_string())?;
    execute(
        &conn,
        &format!("SET temp_directory = {}", lit(&temp_dir.to_string_lossy())),
    )?;
    let mut fingerprints = BTreeMap::new();
    for node in &order {
        if node.kind == "source" {
            let key = required_str(&node.config, "source_key")?;
            if !fingerprints.contains_key(key) {
                let binding = &company.sources[key];
                fingerprints.insert(key.to_string(), hash_file(Path::new(&binding.path))?);
            }
        }
    }
    let mut reviews = Vec::new();
    for node in &order {
        process_node(&conn, wf, company, node)
            .map_err(|e| format!("节点 {}（{}）：{e}", node.id, node.label))?;
        reviews.push(review(&conn, node)?);
    }
    verify_input_fingerprints(company, &fingerprints)?;
    for (node, item) in order.iter().zip(&reviews) {
        if export_files && node.kind == "export" {
            let file_name = node
                .config
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let file_name = if file_name.is_empty() {
                format!("{}.csv", node.id)
            } else {
                let name = Path::new(file_name);
                if name.file_name().and_then(|part| part.to_str()) != Some(file_name)
                    || !file_name.to_ascii_lowercase().ends_with(".csv")
                {
                    return Err(format!(
                        "节点 {} 的输出文件名须为不含路径的 CSV 文件名",
                        node.label
                    ));
                }
                file_name.to_string()
            };
            write_csv(&conn, &item.table, &dir.join(file_name))?;
        }
        if export_files && item.exceptions > 0 {
            write_csv(
                &conn,
                &format!("{}__exceptions", item.table),
                &dir.join(format!("{}_exceptions.csv", node.id)),
            )?;
        }
        if export_files && item.excluded > 0 {
            write_csv(
                &conn,
                &format!("{}__excluded", item.table),
                &dir.join(format!("{}_excluded.csv", node.id)),
            )?;
        }
    }
    let result = RunResult {
        run_dir: dir.to_string_lossy().to_string(),
        database: db_path.to_string_lossy().to_string(),
        reviews,
        input_fingerprints: fingerprints,
    };
    fs::write(
        dir.join("workflow.json"),
        serde_json::to_vec_pretty(wf).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        dir.join("company.json"),
        serde_json::to_vec_pretty(company).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Same-directory rename replaces the completed file; failure never deletes the old target.
pub(crate) fn atomic_replace(
    target: &Path,
    writer: impl FnOnce(&Path) -> Result<(), String>,
    backup: bool,
) -> Result<(), String> {
    atomic_replace_checked(target,writer,backup,|temp,target|fs::rename(temp,target).map_err(|error|format!("替换失败，原文件保持不变：{error}")))
}

pub(crate) fn atomic_replace_checked(
    target: &Path,
    writer: impl FnOnce(&Path) -> Result<(), String>,
    backup: bool,
    publish: impl FnOnce(&Path,&Path)->Result<(),String>,
) -> Result<(), String> {
    let parent = target.parent().ok_or("保存路径无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temp = parent.join(format!(".audit-{}-{stamp}.tmp", std::process::id()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    drop(file);
    let result = (|| {
        writer(&temp)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&temp)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        if backup && target.exists() {
            fs::copy(target, target.with_extension("json.bak"))
                .map_err(|e| format!("备份失败，原文件保持不变：{e}"))?;
        }
        publish(&temp,target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
pub fn export_csv_file(database: &str, table_name: &str, path: &Path) -> Result<i64, String> {
    let _guard = result_query_guard()?;
    identifier(table_name)?;
    if !path.is_absolute()
        || path
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.eq_ignore_ascii_case("csv"))
    {
        return Err("请选择本地CSV文件保存位置".into());
    }
    let conn = Connection::open(database).map_err(|e| e.to_string())?;
    let count = query_i64(
        &conn,
        &format!("SELECT COUNT(*) FROM {}", sql_name(table_name)),
    )?;
    atomic_replace(path, |temp| write_csv(&conn, table_name, temp), false)?;
    Ok(count)
}

#[cfg(test)]
mod column_action_tests {
 use super::*;
 #[test]
 fn text_windows_keep_complete_records_and_tail_physical_positions(){
     let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/step1-foundation/window-records.csv");
     let mut text="编号,摘要\n".to_owned();
     for index in 1..=201{
         text.push_str(&format!("{index},{}\n",if index==101{"\"跨\n行\""}else{"普通"}));
     }
     fs::write(&path,text).unwrap();
     let binding:SourceBinding=serde_json::from_value(serde_json::json!({"path":path,"fields":{}})).unwrap();
     let window=inspect_raw_slice(&binding,100,1,false).unwrap();
     assert_eq!(window.rows.len(),20);
     assert_eq!(window.physical_rows[0],100);
     assert!(window.rows.iter().any(|row|row[1]=="跨\n行"));
     let tail=inspect_raw_slice(&binding,1,1,true).unwrap();
     assert_eq!(tail.rows.len(),20);
     assert_eq!(tail.physical_rows[0],184);
     assert_eq!(tail.physical_rows[19],203);
     assert_eq!(tail.rows[19][0],"201");
 }
 #[test]
 fn amount_formats_are_opt_in_and_keep_malformed_grouping_unresolved(){
     let conn=Connection::open_in_memory().unwrap();
     let options=serde_json::json!({"trim_special_space":true,"thousands":true,"parentheses_negative":true,"currency_prefix":"USD"});
     for (source,expected) in [
         ("\u{a0}USD 1,234.50\u{3000}","1234.50"),
         ("(USD 1,234.50)","-1234.50"),
         ("( USD 1,234.50 )","-1234.50"),
         ("USD 1,23","1,23"),
         ("EUR 100","EUR 100"),
         ("1 000","1 000"),
     ] {
         let normalized:String=conn.query_row(&format!("SELECT {}",amount_text(&options,&lit(source))),[],|row|row.get(0)).unwrap();
         assert_eq!(normalized,expected);
     }
     let strict:String=conn.query_row(&format!("SELECT {}",amount_text(&serde_json::json!({}),&lit("1,234"))),[],|row|row.get(0)).unwrap();
     assert_eq!(strict,"1,234");
 }
 #[test]
 fn result_pages_preserve_order_and_include_the_row_after_the_first_block() {
     let conn = Connection::open_in_memory().unwrap();
     execute(&conn, "CREATE TABLE n_paged AS SELECT i AS __order, 0 AS rowid, i AS source_row FROM range(1,202) t(i) ORDER BY i DESC").unwrap();
     let first = preview_query_at(&conn, "n_paged", "TRUE", 200, 0, true).unwrap();
     let second = preview_query_at(&conn, "n_paged", "TRUE", 200, 200, true).unwrap();
     assert_eq!(first.total_rows, 201);
     assert_eq!(first.rows.iter().map(|row| row[2].parse::<usize>().unwrap()).collect::<Vec<_>>(), (1..=200).collect::<Vec<_>>());
     assert_eq!(second.rows.len(), 1);
     execute(&conn,"CREATE VIEW n_paged_view AS SELECT * FROM n_paged WHERE source_row > 200").unwrap();
     let view=preview_query_at(&conn,"n_paged_view","TRUE",200,0,true).unwrap();
     assert_eq!(view.total_rows,1);
     assert_eq!(second.rows[0][2], "201");
 }
 #[test]
 fn cell_amount_policy_keeps_usable_values_and_calculates_each_row_independently() {
     let conn = Connection::open_in_memory().unwrap();
     execute(&conn, "CREATE TABLE n_seed AS SELECT * FROM (VALUES (1,' 100 ','待确认'), (2,'200',' '), (3,'1.23456','30'), (4,'0','0')) t(__row,借方,贷方)").unwrap();
     let company = CompanyConfig { schema_version: 1, company: "测试".into(), sources: BTreeMap::new() };
     let mut workflow = Workflow { schema_version: 1, name: "金额按格".into(), nodes: vec![], edges: vec![] };
     for (id, source, kind, config) in [
         ("money", "seed", "decimal", serde_json::json!({"fields":[{"source":"借方","target":"借方金额"},{"source":"贷方","target":"贷方金额","empty_as_zero":true}]})),
         ("diff", "money", "derive", serde_json::json!({"expression":"[借方金额]-[贷方金额]","target":"差额"})),
     ] {
         let node = crate::model::Node { id: id.into(), kind: kind.into(), version: if kind == "decimal" { 2 } else { 1 }, label: id.into(), position: crate::model::Point { x: 0.0, y: 0.0 }, config };
         workflow.edges.push(crate::model::Edge { id: id.into(), source: source.into(), target: id.into(), source_port: None, target_port: None });
         process_node(&conn, &workflow, &company, &node).unwrap();
         workflow.nodes.push(node);
     }
     assert_eq!(query_i64(&conn, "SELECT COUNT(*) FROM n_money").unwrap(), 4);
     assert_eq!(query_i64(&conn, "SELECT CAST(SUM(借方金额) AS BIGINT) FROM n_money").unwrap(), 300);
     assert_eq!(query_i64(&conn, "SELECT CAST(SUM(贷方金额) AS BIGINT) FROM n_money").unwrap(), 30);
     assert_eq!(query_i64(&conn, "SELECT COUNT(*) FROM n_money__exceptions").unwrap(), 2);
     assert_eq!(query_i64(&conn, "SELECT COUNT(*) FROM n_money WHERE __row=2 AND 贷方金额=0 AND 贷方金额__amount_status='空白补零'").unwrap(), 1);
     assert_eq!(query_i64(&conn, "SELECT COUNT(*) FROM n_diff WHERE (__row IN (1,3) AND 差额 IS NULL) OR (__row=2 AND 差额=200) OR (__row=4 AND 差额=0)").unwrap(), 4);
 }
 #[test]
 fn six_actions_preserve_values_and_partition_current_errors(){
  let conn=Connection::open_in_memory().unwrap();
  execute(&conn,"CREATE TABLE n_seed AS SELECT * FROM (VALUES (1,' 2 ','0','科目:1001'),(2,'10','0','科目:1002'),(3,'3','错误','未匹配'),(4,'1.12345','0','科目:1004'),(5,'1e2','0','科目:1005')) t(__row,金额一,金额二,说明)").unwrap();
  let specs=vec![
   ("trim","text_clean",serde_json::json!({"fields":[{"source":"金额一","target":"清理"}],"mode":"trim"})),
   ("replace","text_clean",serde_json::json!({"fields":[{"source":"说明","target":"替换"}],"mode":"replace","find":":","replacement":"-"})),
   ("extract","text_extract",serde_json::json!({"source":"替换","target":"编码","pattern":"科目-([0-9]+)","group":1})),
   ("first","decimal",serde_json::json!({"fields":[{"source":"清理","target":"数额一"}]})),
   ("second","decimal",serde_json::json!({"fields":[{"source":"金额二","target":"数额二"}]})),
   ("calc","derive",serde_json::json!({"expression":"[数额一]-[数额二]","target":"差额"})),
   ("filter","filter",serde_json::json!({"condition":{"field":"差额","operator":"numeric_gt","value":"2"}}))];
  let company=CompanyConfig{schema_version:1,company:"测试".into(),sources:BTreeMap::new()};
  let mut wf=Workflow{schema_version:1,name:"列处理".into(),nodes:vec![],edges:vec![]};let mut previous="seed";
  for (id,kind,config) in specs {let node=crate::model::Node{id:id.into(),kind:kind.into(),version:1,label:id.into(),position:crate::model::Point{x:0.0,y:0.0},config};wf.edges.push(crate::model::Edge{id:id.into(),source:previous.into(),target:id.into(),source_port:None,target_port:None});process_node(&conn,&wf,&company,&node).unwrap();wf.nodes.push(node);previous=id;}
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_first__exceptions").unwrap(),2);
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_second__exceptions").unwrap(),1);
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_extract__exceptions").unwrap(),1);
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_extract").unwrap(),5);
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_filter WHERE 差额=10 AND 编码='1002' AND 金额一='10'").unwrap(),1);
  assert_eq!(query_i64(&conn,"SELECT COUNT(*) FROM n_filter__excluded WHERE 差额=2 AND 金额一=' 2 '").unwrap(),1);
  for config in [serde_json::json!({"fields":[{"source":"缺列","target":"新列"}]}),serde_json::json!({"fields":[{"source":"金额一","target":"金额二"}]}),serde_json::json!({"fields":[{"source":"金额一","target":"X"},{"source":"金额二","target":"x"}]})] {let node=crate::model::Node{id:"bad".into(),kind:"decimal".into(),version:1,label:"bad".into(),position:crate::model::Point{x:0.0,y:0.0},config};wf.edges.push(crate::model::Edge{id:"bad".into(),source:"seed".into(),target:"bad".into(),source_port:None,target_port:None});assert!(process_node(&conn,&wf,&company,&node).is_err());}
 }
}
