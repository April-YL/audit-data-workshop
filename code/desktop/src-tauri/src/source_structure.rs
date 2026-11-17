//! Bounded structural suggestions. A candidate is never an input binding until applied.
use crate::model::{ReadingBaseline, ReadingCheck, ReadingIssue};
use crate::model::{
    SourceBinding, SourcePreview, StructureCandidate, StructureField, StructureReport, TableLayout,
};
use calamine::{open_workbook, Xlsx};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Merge = (usize, usize, u32, u32);

pub(crate) fn fields(
    layout: &TableLayout,
    rows: &BTreeMap<usize, Vec<String>>,
    merges: &[Merge],
) -> Result<(Vec<StructureField>, Vec<String>), String> {
    crate::engine::validate_layout(layout)?;
    if layout
        .field_names
        .keys()
        .any(|c| *c < layout.first_col || *c > layout.last_col)
    {
        return Err("字段更名位置不在所选列范围内".into());
    }
    let mut expanded = rows.clone();
    let mut issues = Vec::new();
    for &(r1, r2, c1, c2) in merges {
        if r1 < layout.data_start_row && r2 >= layout.data_start_row {
            return Err(format!(
                "合并区域第 {r1} 至 {r2} 行跨越表头与明细，请调整数据区域"
            ));
        }
        if !layout.header_rows.iter().any(|r| *r >= r1 && *r <= r2) {
            continue;
        }
        if c2 < layout.first_col || c1 > layout.last_col {
            continue;
        }
        if c1 < layout.first_col || c2 > layout.last_col || !layout.header_rows.contains(&r1) {
            issues.push(format!(
                "表头合并区域 {r1}:{r2} 行、{c1}:{c2} 列被所选区域截断"
            ));
            continue;
        }
        let anchor = rows
            .get(&r1)
            .and_then(|row| row.get((c1 - layout.first_col) as usize))
            .cloned()
            .unwrap_or_default();
        for r in &layout.header_rows {
            if *r < r1 || *r > r2 {
                continue;
            }
            if let Some(row) = expanded.get_mut(r) {
                for col in c1..=c2 {
                    if let Some(cell) = row.get_mut((col - layout.first_col) as usize) {
                        if cell.trim().is_empty() {
                            *cell = anchor.clone();
                        }
                    }
                }
            }
        }
    }
    let mut used = BTreeSet::new();
    let mut result = Vec::new();
    for col in layout.first_col..=layout.last_col {
        let mut parts: Vec<String> = Vec::new();
        for r in &layout.header_rows {
            let value = expanded
                .get(r)
                .and_then(|row| row.get((col - layout.first_col) as usize))
                .ok_or_else(|| format!("表头原始行 {r} 不在预览范围内"))?
                .trim();
            if !value.is_empty() && parts.last().is_none_or(|last| last != value) {
                parts.push(value.to_string());
            }
        }
        let name = layout
            .field_names
            .get(&col)
            .cloned()
            .unwrap_or_else(|| parts.join(" / "));
        if name.trim().is_empty() {
            issues.push(format!("第 {col} 列缺少字段名，请更名或调整列范围"));
        } else if name.starts_with("__")
            || name.chars().count() > 100
            || name.trim() != name
            || name.contains(['\r', '\n', '\0'])
        {
            issues.push(format!(
                "第 {col} 列字段名无效（保留前缀、首尾空格、控制字符或超过100字）"
            ));
        } else if !used.insert(name.to_lowercase()) {
            issues.push(format!("第 {col} 列字段名重复：{name}；请明确区分"));
        }
        result.push(StructureField {
            physical_col: col,
            parts,
            name,
        });
    }
    Ok((result, issues))
}

fn numeric(value: &str) -> bool {
    value.trim().parse::<f64>().is_ok()
}

fn candidate(
    raw: &crate::model::RawSourcePreview,
    layout: TableLayout,
    merges: &[Merge],
) -> Result<StructureCandidate, String> {
    crate::engine::validate_layout(&layout)?;
    if !layout.fill_merged_data_cols.is_empty() {
        return Err("本次输入内整理不支持明细补值；请保留旧结构整理节点使用原规则".into());
    }
    let rows: BTreeMap<usize, Vec<String>> = raw
        .physical_rows
        .iter()
        .zip(&raw.rows)
        .map(|(r, row)| {
            (
                *r,
                (layout.first_col..=layout.last_col)
                    .map(|c| row.get(c as usize - 1).cloned().unwrap_or_default())
                    .collect(),
            )
        })
        .collect();
    let (fields, mut issues) = match fields(&layout, &rows, merges) {
        Ok(result) => result,
        Err(message) => (Vec::new(), vec![message]),
    };
    let body: Vec<_> = rows
        .iter()
        .filter(|(r, _)| {
            **r >= layout.data_start_row && layout.data_end_row.is_none_or(|end| **r <= end)
        })
        .take(20)
        .collect();
    if body.is_empty() {
        issues.push("所选明细范围没有可预览记录，请检查明细起止行".into());
    }
    let mut data_rows = Vec::new();
    let mut physical_rows = Vec::new();
    for (r, row) in body {
        physical_rows.push(*r);
        data_rows.push(row.clone());
    }
    // No automatic rejection of totals or repeated headers. Those require a policy.
    if layout.strict_columns
        && raw.physical_rows.iter().zip(&raw.rows).any(|(r, row)| {
            *r >= layout.data_start_row
                && layout.data_end_row.is_none_or(|end| *r <= end)
                && row.iter().enumerate().any(|(i, v)| {
                    !v.trim().is_empty()
                        && (i + 1 < layout.first_col as usize || i + 1 > layout.last_col as usize)
                })
        })
    {
        issues.push("明细在所选列范围外还有值；请扩大范围，或明确确认排除范围外列".into());
    }
    let preview = SourcePreview {
        headers: fields.iter().map(|f| f.name.clone()).collect(),
        rows: data_rows,
        physical_rows,
        truncated: raw.truncated,
        sheet: raw.sheet.clone(),
    };
    let mut diagnostics = Vec::new();
    for &(r1, r2, c1, c2) in merges {
        let crossing = r1 < layout.data_start_row && r2 >= layout.data_start_row;
        let clipped = layout.header_rows.iter().any(|r| *r >= r1 && *r <= r2)
            && c2 >= layout.first_col
            && c1 <= layout.last_col
            && (c1 < layout.first_col || c2 > layout.last_col || !layout.header_rows.contains(&r1));
        if crossing || clipped {
            diagnostics.push(issue(
                "merge_boundary",
                "合并区域跨越或被读取边界截断，请调整区域",
                Some(r1),
                Some(c1),
            ));
        }
    }
    for field in &fields {
        let duplicates: Vec<u32> = fields
            .iter()
            .filter(|f| f.name.to_lowercase() == field.name.to_lowercase())
            .map(|f| f.physical_col)
            .collect();
        let problem = if field.name.trim().is_empty() {
            Some(("unnamed", "此列没有字段名，请明确命名或调整读取范围"))
        } else if duplicates.len() > 1 {
            Some(("duplicate", "字段名重复，请结合原始表头明确区分"))
        } else if field.name.starts_with("__")
            || field.name.trim() != field.name
            || field.name.chars().count() > 100
            || field.name.contains(['\r', '\n', '\0'])
        {
            Some(("invalid_name", "字段名无效，请修改"))
        } else {
            None
        };
        if let Some((code, message)) = problem {
            diagnostics.push(ReadingIssue {
                code: code.into(),
                message: message.into(),
                row: layout.header_rows.last().copied(),
                col: Some(field.physical_col),
                related_cols: duplicates,
            });
        }
    }
    if layout.strict_columns {
        if let Some((row, col)) = raw
            .physical_rows
            .iter()
            .zip(&raw.rows)
            .filter(|(r, _)| {
                **r >= layout.data_start_row && layout.data_end_row.is_none_or(|e| **r <= e)
            })
            .find_map(|(r, values)| {
                values
                    .iter()
                    .enumerate()
                    .find(|(i, v)| {
                        !v.trim().is_empty()
                            && (i + 1 < layout.first_col as usize
                                || i + 1 > layout.last_col as usize)
                    })
                    .map(|(i, _)| (*r, i as u32 + 1))
            })
        {
            diagnostics.push(issue(
                "outside_columns",
                "所选列范围外还有值，请核实是否扩大读取范围",
                Some(row),
                Some(col),
            ));
        }
    }
    if diagnostics.is_empty() && !issues.is_empty() {
        diagnostics.extend(issues.iter().map(|m| {
            issue(
                "layout",
                m,
                layout.header_rows.first().copied(),
                Some(layout.first_col),
            )
        }));
    }
    let baseline = ReadingBaseline {
        version: 1,
        sheet: raw.sheet.clone(),
        layout: layout.clone(),
        header_cells: layout
            .header_rows
            .iter()
            .map(|r| rows.get(r).cloned().unwrap_or_default())
            .collect(),
        header_merges: merges
            .iter()
            .filter(|(a, b, c, d)| {
                layout.header_rows.iter().any(|r| r >= a && r <= b)
                    && *d >= layout.first_col
                    && *c <= layout.last_col
            })
            .copied()
            .collect(),
    };
    Ok(StructureCandidate {
        layout,
        fields,
        ready: issues.is_empty(),
        issues,
        score: 0,
        diagnostics,
        baseline,
        preview,
    })
}

fn issue(code: &str, message: &str, row: Option<usize>, col: Option<u32>) -> ReadingIssue {
    ReadingIssue {
        code: code.into(),
        message: message.into(),
        row,
        col,
        related_cols: Vec::new(),
    }
}

/// Compare raw header evidence, never the user's renamed output labels alone.
pub fn compare_baseline(expected: &ReadingBaseline, actual: &ReadingBaseline) -> Vec<ReadingIssue> {
    let mut problems = Vec::new();
    if expected.version != 1 {
        problems.push(issue(
            "version",
            "读取依据版本不支持，请重新确认",
            None,
            None,
        ));
    }
    if expected.sheet != actual.sheet {
        problems.push(issue("sheet", "工作表已变化，请重新确认", None, None));
    }
    if expected.layout != actual.layout {
        problems.push(issue(
            "layout_changed",
            "读取规则已变化，请确认新的读取依据",
            None,
            None,
        ));
    }
    for (i, r) in expected.layout.header_rows.iter().enumerate() {
        for col in expected.layout.first_col..=expected.layout.last_col {
            let offset = (col - expected.layout.first_col) as usize;
            let before = expected.header_cells.get(i).and_then(|v| v.get(offset));
            let after = actual.header_cells.get(i).and_then(|v| v.get(offset));
            if before != after {
                problems.push(issue(
                    "header_changed",
                    &format!(
                        "原始表头变化：原为「{}」，现为「{}」",
                        before.map(String::as_str).unwrap_or(""),
                        after.map(String::as_str).unwrap_or("")
                    ),
                    Some(*r),
                    Some(col),
                ));
            }
        }
    }
    if expected.header_merges != actual.header_merges {
        problems.push(issue(
            "merges_changed",
            "表头合并关系已变化，请核实字段归属",
            expected.layout.header_rows.first().copied(),
            Some(expected.layout.first_col),
        ));
    }
    problems
}

pub fn check_reading(binding: &SourceBinding, full: bool) -> ReadingCheck {
    let report = match inspect(binding) {
        Ok(r) => r,
        Err(e) => {
            return ReadingCheck {
                compatible: false,
                baseline: None,
                diagnostics: vec![issue("read_failed", &e, None, None)],
                report: None,
                full_checked: false,
            }
        }
    };
    let active = report.active.as_ref();
    let baseline = active.map(|c| c.baseline.clone());
    let mut diagnostics = active
        .map(|c| c.diagnostics.clone())
        .unwrap_or_else(|| vec![issue("unconfirmed", "请先确认读取区域", None, None)]);
    if let (Some(old), Some(now)) = (&binding.reading_baseline, &baseline) {
        diagnostics.extend(compare_baseline(old, now));
    }
    if full && diagnostics.is_empty() {
        if let Err(e) = crate::engine::check_reading_extent(binding) {
            diagnostics.push(e);
        }
    }
    let full_checked = full && diagnostics.is_empty();
    ReadingCheck {
        compatible: diagnostics.is_empty(),
        baseline,
        diagnostics,
        report: Some(report),
        full_checked,
    }
}

pub fn inspect(binding: &SourceBinding) -> Result<StructureReport, String> {
    if !binding.path.to_ascii_lowercase().ends_with(".xlsx") {
        return Err("表格结构建议目前仅支持 XLSX".into());
    }
    let cap = binding
        .layout
        .as_ref()
        .map_or(40, |l| l.data_start_row.saturating_add(20).max(40));
    if cap > 200 {
        return Err("本批支持在前180行内确认明细起点；更靠后的结构请使用原结构整理流程".into());
    }
    let raw = crate::engine::inspect_raw_source(binding, cap)?;
    let mut workbook: Xlsx<_> =
        open_workbook(&binding.path).map_err(|e: calamine::XlsxError| e.to_string())?;
    let merges: Vec<Merge> = workbook
        .worksheet_merge_cells(&raw.sheet)
        .ok_or("工作表不存在")?
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| {
            (
                m.start.0 as usize + 1,
                m.end.0 as usize + 1,
                m.start.1 + 1,
                m.end.1 + 1,
            )
        })
        .collect();
    let mut candidates = Vec::new();
    let available = raw.rows.len().min(20);
    for end in 1..available {
        let row = &raw.rows[end - 1];
        let occupied: Vec<_> = row
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.trim().is_empty())
            .collect();
        if occupied.len() < 2
            || occupied.iter().filter(|(_, s)| !numeric(s)).count() * 4 < occupied.len() * 3
        {
            continue;
        }
        let first = occupied.first().unwrap().0 as u32 + 1;
        let header_last = occupied.last().unwrap().0 as u32 + 1;
        // Include populated body columns: unnamed columns must be resolved, not silently dropped.
        let last = raw
            .rows
            .iter()
            .skip(end)
            .take(3)
            .flat_map(|r| {
                r.iter()
                    .enumerate()
                    .filter(|(_, v)| !v.trim().is_empty())
                    .map(|(i, _)| i as u32 + 1)
            })
            .max()
            .unwrap_or(header_last)
            .max(header_last);
        for start in end.saturating_sub(2).max(1)..=end {
            if (start..=end).any(|r| {
                let values: Vec<_> = raw.rows[r - 1]
                    .iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                values.len() < 2
                    || values.iter().filter(|s| !numeric(s)).count() * 4 < values.len() * 3
            }) {
                continue;
            }
            let layout = TableLayout {
                first_col: first,
                last_col: last,
                header_rows: (start..=end).collect(),
                data_start_row: end + 1,
                data_end_row: None,
                fill_merged_data_cols: Vec::new(),
                field_names: BTreeMap::new(),
                strict_columns: true,
            };
            if let Ok(mut c) = candidate(&raw, layout, &merges) {
                let next = raw.rows.iter().skip(end).take(3);
                let numbers = next
                    .flatten()
                    .filter(|v| !v.trim().is_empty() && numeric(v))
                    .count() as i32;
                let groups = c.fields.iter().filter(|f| f.parts.len() > 1).count() as i32;
                c.score = numbers.min(20) * 3 + groups * 2
                    - c.issues.len() as i32 * 5
                    - (end - start) as i32;
                candidates.push(c);
            }
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.layout.header_rows.len().cmp(&b.layout.header_rows.len()))
    });
    candidates.truncate(3);
    let active = binding
        .layout
        .clone()
        .map(|l| candidate(&raw, l, &merges))
        .transpose()?;
    let mut notices = vec![
        format!(
            "仅依据前 {} 行提出结构候选；不代表全量资料检查通过",
            raw.rows.len()
        ),
        "未自动剔除合计、说明、空行或重复表头；明细普通空白不补值".into(),
    ];
    if let Some(c) = &active {
        if raw.rows.iter().any(|r| {
            r.iter().enumerate().any(|(i, v)| {
                !v.trim().is_empty()
                    && (i + 1 < c.layout.first_col as usize || i + 1 > c.layout.last_col as usize)
            })
        }) {
            notices.push("所选列范围外存在原始值；这些列不会进入处理结果，请核实列范围".into());
        }
        if c.layout.data_end_row.is_some() {
            notices.push("已指定明细结束行，范围外记录不会进入处理；请自行核实排除范围".into());
        }
    }
    Ok(StructureReport {
        raw,
        candidates,
        active,
        notices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuse_checks_raw_header_positions_even_with_identical_output_names() {
        let layout:TableLayout=serde_json::from_value(serde_json::json!({"first_col":1,"last_col":2,"header_rows":[5,6],"data_start_row":7,"fill_merged_data_cols":[]})).unwrap();
        let before = ReadingBaseline {
            version: 1,
            sheet: "表一".into(),
            layout,
            header_cells: vec![
                vec!["余额".into(), "余额".into()],
                vec!["借方".into(), "贷方".into()],
            ],
            header_merges: vec![(5, 5, 1, 2)],
        };
        assert!(compare_baseline(&before, &before).is_empty());
        let mut after = before.clone();
        after.header_cells[1].swap(0, 1);
        let issues = compare_baseline(&before, &after);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].row, Some(6));
        assert_eq!(issues[0].col, Some(1));
        after = before.clone();
        after.header_merges.clear();
        assert_eq!(compare_baseline(&before, &after)[0].code, "merges_changed");
        after = before.clone();
        after.sheet = "其他".into();
        assert_eq!(compare_baseline(&before, &after)[0].code, "sheet");
    }
    #[test]
    fn real_tb_header_roles_require_explicit_names_without_filling_blanks() {
        // Only observed header labels, no customer records or amounts.
        let layout:TableLayout=serde_json::from_value(serde_json::json!({"first_col":1,"last_col":4,"header_rows":[5,6],"data_start_row":7,"fill_merged_data_cols":[]})).unwrap();
        let rows = BTreeMap::from([
            (
                5,
                vec!["".into(), "期初余额".into(), "方向".into(), "方向".into()],
            ),
            (
                6,
                vec![
                    "科目编码".into(),
                    "本币".into(),
                    "方向".into(),
                    "方向".into(),
                ],
            ),
        ]);
        let (f, issues) = fields(&layout, &rows, &[]).unwrap();
        assert_eq!(f[0].name, "科目编码");
        assert_eq!(f[1].name, "期初余额 / 本币");
        assert_eq!(issues.len(), 1);
        let mut corrected = layout;
        corrected.field_names.insert(4, "期末方向".into());
        assert!(fields(&corrected, &rows, &[]).unwrap().1.is_empty());
    }
    #[test]
    fn merged_headers_use_actual_bounds_and_reject_crossing_detail() {
        let layout:TableLayout=serde_json::from_value(serde_json::json!({"first_col":1,"last_col":2,"header_rows":[5,6],"data_start_row":7,"fill_merged_data_cols":[]})).unwrap();
        let rows = BTreeMap::from([
            (5, vec!["余额".into(), "".into()]),
            (6, vec!["借方".into(), "贷方".into()]),
        ]);
        let f = fields(&layout, &rows, &[(5, 5, 1, 2)]).unwrap().0;
        assert_eq!(f[1].name, "余额 / 贷方");
        assert_eq!(fields(&layout, &rows, &[]).unwrap().0[1].name, "贷方");
        assert!(fields(&layout, &rows, &[(6, 7, 1, 1)]).is_err());
        let mut invalid = layout;
        invalid.field_names.insert(3, "范围外".into());
        assert!(fields(&invalid, &rows, &[]).is_err());
    }
}
