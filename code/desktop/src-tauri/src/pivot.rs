use crate::{
    engine, field_contract,
    model::{FieldDescriptor, Node},
    workflow::sql_name,
};
use duckdb::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Deserialize)]
pub(crate) struct Rule {
    pub row_by: Vec<String>,
    pub column_by: Vec<String>,
    metrics: Vec<Metric>,
}

#[derive(Deserialize)]
struct Metric {
    op: String,
    #[serde(default)]
    field: String,
    target: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Column {
    pub name: String,
    pub values: Vec<Option<String>>,
    pub total: bool,
}

pub(crate) fn members(
    result: &crate::model::RunResult,
    node_id: &str,
    group_values: &std::collections::BTreeMap<String, Option<String>>,
    column: &str,
    total_row: bool,
    limit: usize,
) -> Result<crate::model::Preview, String> {
    let _guard = engine::result_query_guard()?;
    let workflow: crate::model::Workflow = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(&result.run_dir).join("workflow.json"))
            .map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())?;
    let node = workflow
        .nodes
        .iter()
        .find(|node| node.id == node_id && node.kind == "pivot")
        .ok_or("请选择本次运行的透视步骤")?;
    let parent = engine::input_ids(&workflow, node)?
        .first()
        .copied()
        .ok_or("透视缺少输入")?;
    let input = crate::result_queries::resolve(result, parent, "normal")?;
    let output = crate::result_queries::resolve(result, node_id, "normal")?;
    let conn = Connection::open(&result.database).map_err(|err| err.to_string())?;
    let fields = engine::descriptors(&conn, &input, parent)?;
    let rule = read(node, &fields)?;
    if rule.row_by.iter().collect::<BTreeSet<_>>() != group_values.keys().collect::<BTreeSet<_>>() {
        return Err("透视成员查询的行维度与本次运行不一致".into());
    }
    let specification: String = conn
        .query_row(
            &format!(
                "SELECT specification FROM {} WHERE name = ?",
                sql_name(&format!("{output}__pivot_columns"))
            ),
            [column],
            |row| row.get(0),
        )
        .map_err(|_| "所选列不是本次透视统计列".to_string())?;
    let column: Column = serde_json::from_str(&specification).map_err(|err| err.to_string())?;
    let types = engine::column_types(&conn, &input)?;
    let condition = |name: &str, value: &Option<String>| match value {
        None => format!("{} IS NULL", sql_name(name)),
        Some(value) => format!(
            "{} IS NOT DISTINCT FROM CAST({} AS {})",
            sql_name(name),
            engine::lit(value),
            types[name]
        ),
    };
    let mut conditions = Vec::new();
    if !total_row {
        for name in &rule.row_by {
            conditions.push(condition(name, &group_values[name]));
        }
    }
    if !column.total {
        for (name, value) in rule.column_by.iter().zip(&column.values) {
            conditions.push(condition(name, value));
        }
    }
    let predicate = if conditions.is_empty() {
        "TRUE".into()
    } else {
        conditions.join(" AND ")
    };
    engine::preview_query(&conn, &input, &predicate, limit)
}

pub(crate) fn read(node: &Node, fields: &[FieldDescriptor]) -> Result<Rule, String> {
    let rule: Rule = serde_json::from_value(node.config.clone())
        .map_err(|err| format!("透视规则不完整：{err}"))?;
    if rule.row_by.is_empty() || rule.column_by.is_empty() || rule.metrics.is_empty() {
        return Err("请选择行维度、列维度和统计项".into());
    }
    let mut dimensions = BTreeSet::new();
    if rule.row_by.iter().any(|name| name == "汇总层级") {
        return Err("行维度与透视结果的汇总层级列重名，请先更名".into());
    }
    for name in rule.row_by.iter().chain(&rule.column_by) {
        if !fields.iter().any(|field| field.name == *name) || !dimensions.insert(name) {
            return Err(format!("透视维度不存在或重复：{name}"));
        }
    }
    let mut names = BTreeSet::new();
    for metric in &rule.metrics {
        field_contract::business_name(&metric.target)?;
        if !names.insert(metric.target.to_lowercase()) {
            return Err("透视统计名称重复".into());
        }
        if !["sum", "rows", "count", "distinct"].contains(&metric.op.as_str()) {
            return Err("不支持的透视统计方式".into());
        }
        if metric.op != "rows" {
            let field = fields
                .iter()
                .find(|field| field.name == metric.field)
                .ok_or("透视统计字段不存在")?;
            if metric.op == "sum" && !field_contract::numeric(&field.data_type) {
                return Err("求和需要数值字段，请先转换金额".into());
            }
        }
    }
    Ok(rule)
}

fn expression(metric: &Metric, predicate: &str) -> String {
    let field = sql_name(&metric.field);
    let aggregate = match metric.op.as_str() {
        "rows" => "COUNT(*)".into(),
        "count" => format!("COUNT({field})"),
        "distinct" => format!("COUNT(DISTINCT {field})"),
        _ => format!("SUM({field})"),
    };
    if predicate == "TRUE" {
        return aggregate;
    }
    format!("CASE WHEN COUNT(*) FILTER (WHERE {predicate}) = 0 THEN NULL ELSE {aggregate} FILTER (WHERE {predicate}) END")
}

pub(crate) fn execute(
    conn: &Connection,
    node: &Node,
    input: &str,
    output: &str,
) -> Result<(), String> {
    let fields = engine::descriptors(conn, input, &node.id)?;
    let rule = read(node, &fields)?;
    let types = engine::column_types(conn, input)?;
    let dimensions = rule
        .column_by
        .iter()
        .map(|name| sql_name(name))
        .collect::<Vec<_>>();
    let query = format!(
        "SELECT DISTINCT {} FROM {} ORDER BY {} LIMIT 201",
        dimensions
            .iter()
            .map(|name| format!("CAST({name} AS VARCHAR)"))
            .collect::<Vec<_>>()
            .join(","),
        sql_name(input),
        (1..=dimensions.len())
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut statement = conn.prepare(&query).map_err(|err| err.to_string())?;
    let values = statement
        .query_map([], |row| {
            (0..dimensions.len())
                .map(|index| row.get::<_, Option<String>>(index))
                .collect::<duckdb::Result<Vec<_>>>()
        })
        .map_err(|err| err.to_string())?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(|err| err.to_string())?;
    if values.len().saturating_mul(rule.metrics.len()) > 200 {
        return Err("透视动态列超过200列，请减少列维度或统计项".into());
    }
    let rows = rule
        .row_by
        .iter()
        .map(|name| sql_name(name))
        .collect::<Vec<_>>();
    let mut selects = rows.clone();
    selects.push(format!("GROUPING({}) <> 0 AS __pivot_total", rows[0]));
    selects.push(format!(
        "CASE WHEN GROUPING({}) <> 0 THEN '总计' ELSE '分组' END AS \"汇总层级\"",
        rows[0]
    ));
    for (index, name) in rows.iter().enumerate() {
        selects.push(format!("{name} IS NULL AS __group_null_{index}"));
    }
    selects.push("COUNT(*) AS __members".into());
    let mut columns = Vec::new();
    let mut used = rule
        .row_by
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<BTreeSet<_>>();
    for (tuple, total) in values
        .iter()
        .map(|value| (value.clone(), false))
        .chain(std::iter::once((vec![], true)))
    {
        let predicate = if total {
            "TRUE".into()
        } else {
            rule.column_by
                .iter()
                .zip(&tuple)
                .map(|(name, value)| match value {
                    None => format!("{} IS NULL", sql_name(name)),
                    Some(value) => format!(
                        "{} IS NOT DISTINCT FROM CAST({} AS {})",
                        sql_name(name),
                        engine::lit(value),
                        types[name]
                    ),
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        for metric in &rule.metrics {
            let identity = serde_json::to_string(&(&rule.column_by, &tuple, &metric.target, total))
                .map_err(|err| err.to_string())?;
            let hash = format!("{:x}", Sha256::digest(identity.as_bytes()));
            let label = if total {
                "行合计".into()
            } else {
                tuple
                    .iter()
                    .map(|value| value.as_deref().unwrap_or("NULL"))
                    .collect::<Vec<_>>()
                    .join("/")
            };
            let label: String = label.chars().take(24).collect();
            let target: String = metric.target.chars().take(24).collect();
            let name = format!("{target}［{label}］_{}", &hash[..16]);
            if !used.insert(name.to_lowercase()) {
                return Err("透视输出列名冲突，请修改统计名称".into());
            }
            selects.push(format!(
                "{} AS {}",
                expression(metric, &predicate),
                sql_name(&name)
            ));
            columns.push(Column {
                name,
                values: tuple.clone(),
                total,
            });
        }
    }
    let groups = rows.join(",");
    conn.execute_batch(&format!(
        "CREATE TABLE {} AS SELECT *, ROW_NUMBER() OVER (ORDER BY __pivot_total, {groups}) AS __order
         FROM (SELECT {} FROM {} GROUP BY GROUPING SETS (({groups}), ()))",
        sql_name(output),selects.join(","),sql_name(input),
    )).map_err(|err|format!("透视执行失败：{err}"))?;
    conn.execute_batch(&format!(
        "CREATE TABLE {} (name VARCHAR, specification VARCHAR)",
        sql_name(&format!("{output}__pivot_columns"))
    ))
    .map_err(|err| err.to_string())?;
    for column in columns {
        conn.execute(
            &format!(
                "INSERT INTO {} VALUES (?,?)",
                sql_name(&format!("{output}__pivot_columns"))
            ),
            [
                &column.name,
                &serde_json::to_string(&column).map_err(|err| err.to_string())?,
            ],
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn distinct_totals_are_recomputed_and_dynamic_limit_is_enforced() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE input AS SELECT * FROM (VALUES ('A','01','x',10),('A','02','x',20),('B','01','y',NULL)) t(company,month,person,amount)").unwrap();
        let mut node:Node=serde_json::from_value(json!({"id":"pivot","kind":"pivot","label":"透视","position":{"x":0,"y":0},"config":{"row_by":["company"],"column_by":["month"],"metrics":[{"op":"distinct","field":"person","target":"人数"}]}})).unwrap();
        execute(&conn, &node, "input", "result").unwrap();
        let mut stmt = conn
            .prepare("SELECT specification FROM result__pivot_columns")
            .unwrap();
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|item| serde_json::from_str::<Column>(&item.unwrap()).unwrap())
            .collect::<Vec<_>>();
        let total = &columns.iter().find(|column| column.total).unwrap().name;
        let count: i64 = conn
            .query_row(
                &format!("SELECT {} FROM result WHERE __pivot_total", sql_name(total)),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        let count: i64 = conn
            .query_row(
                &format!("SELECT {} FROM result WHERE company='A'", sql_name(total)),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        conn.execute_batch("CREATE TABLE empty_input AS SELECT * FROM input WHERE FALSE")
            .unwrap();
        execute(&conn, &node, "empty_input", "empty_pivot").unwrap();
        let empty_count: i64 = conn
            .query_row(
                &format!(
                    "SELECT {} FROM empty_pivot WHERE __pivot_total",
                    sql_name(total)
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(empty_count, 0);
        conn.execute_batch("CREATE TABLE wide AS SELECT 'A' AS company, CAST(i AS VARCHAR) AS month, 'x' AS person FROM range(201) t(i)").unwrap();
        assert!(execute(&conn, &node, "wide", "blocked")
            .unwrap_err()
            .contains("200"));
        node.config["metrics"] = json!([{"op":"sum","field":"amount","target":"金额"}]);
        execute(&conn, &node, "input", "sums").unwrap();
    }
}
