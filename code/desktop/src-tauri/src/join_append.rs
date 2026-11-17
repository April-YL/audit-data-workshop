use crate::model::{Edge, Node, Workflow};
use crate::workflow::sql_name;

#[derive(serde::Deserialize)]
pub(crate) struct AppendRule {
    pub ports: Vec<String>,
    fields: Vec<AppendField>,
    mappings: Vec<Vec<Option<String>>>,
    excluded: Vec<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct AppendField {
    name: String,
    data_type: String,
}

impl AppendRule {
    pub(crate) fn read(node: &Node) -> Result<Self, String> {
        let rule: Self = serde_json::from_value(node.config.clone())
            .map_err(|err| format!("追加规则不完整：{err}"))?;
        let unique: std::collections::BTreeSet<_> = rule.ports.iter().collect();
        if rule.ports.len() < 2
            || unique.len() != rule.ports.len()
            || rule.ports.iter().any(|port| port.trim().is_empty())
        {
            return Err("追加须至少配置两个不同的输入端口".into());
        }
        if rule.fields.is_empty()
            || rule.mappings.len() != rule.ports.len()
            || rule.excluded.len() != rule.ports.len()
            || rule
                .mappings
                .iter()
                .any(|mapping| mapping.len() != rule.fields.len())
        {
            return Err("请逐份配置追加字段、补空列和排除列".into());
        }
        Ok(rule)
    }

    pub(crate) fn output_fields(
        &self,
        node: &Node,
        inputs: &[&[crate::model::FieldDescriptor]],
    ) -> Result<Vec<crate::model::FieldDescriptor>, String> {
        let mut names = std::collections::BTreeSet::new();
        for field in &self.fields {
            crate::field_contract::business_name(&field.name)?;
            if !names.insert(field.name.to_lowercase()) {
                return Err(format!("追加输出列重复：{}", field.name));
            }
        }
        if inputs.len() != self.ports.len() {
            return Err("追加输入与配置的端口数量不一致".into());
        }
        for (index, input) in inputs.iter().enumerate() {
            let port = &self.ports[index];
            let mapping = &self.mappings[index];
            let excluded = &self.excluded[index];
            let mut handled = std::collections::BTreeSet::new();
            for (target, source) in self.fields.iter().zip(mapping) {
                let Some(source) = source else {
                    continue;
                };
                let field = input
                    .iter()
                    .find(|field| field.name == *source)
                    .ok_or_else(|| format!("端口 {port} 缺少映射列 {source}"))?;
                if field.data_type != target.data_type {
                    return Err(format!(
                        "端口 {port} 的列 {source} 类型为 {}，目标 {} 需要 {}",
                        field.data_type, target.name, target.data_type
                    ));
                }
                handled.insert(source.as_str());
            }
            for name in excluded {
                if !input.iter().any(|field| field.name == *name) || !handled.insert(name.as_str())
                {
                    return Err(format!("端口 {port} 的排除列 {name} 无效或已映射"));
                }
            }
            for field in *input {
                if !field.name.starts_with("__")
                    && field.name != "company"
                    && !handled.contains(field.name.as_str())
                {
                    return Err(format!("端口 {port} 的列 {} 尚未映射或排除", field.name));
                }
            }
        }
        let mut output = self
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                if !self.mappings.iter().any(|mapping| mapping[index].is_some()) {
                    return Err(format!(
                        "目标列 {} 至少需要一份实际输入，以确认列类型",
                        field.name
                    ));
                }
                Ok(crate::model::FieldDescriptor {
                    name: field.name.clone(),
                    data_type: field.data_type.clone(),
                    nullable: true,
                    origin: node.id.clone(),
                    confirmed: true,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        for (name, data_type) in [
            ("__company", "VARCHAR"),
            ("__file", "VARCHAR"),
            ("__sheet", "VARCHAR"),
            ("__row", "BIGINT"),
            ("__record", "BIGINT"),
            ("__append_input", "INTEGER"),
            ("__order", "BIGINT"),
        ] {
            output.push(crate::model::FieldDescriptor {
                name: name.into(),
                data_type: data_type.into(),
                nullable: true,
                origin: node.id.clone(),
                confirmed: true,
            });
        }
        Ok(output)
    }
}

pub(crate) fn execute_append(
    conn: &duckdb::Connection,
    node: &Node,
    tables: &[String],
    output: &str,
) -> Result<(), String> {
    let rule = AppendRule::read(node)?;
    let descriptors = tables
        .iter()
        .map(|name| crate::engine::descriptors(conn, name, name))
        .collect::<Result<Vec<_>, _>>()?;
    let inputs = descriptors.iter().map(Vec::as_slice).collect::<Vec<_>>();
    rule.output_fields(node, &inputs)?;
    let mut queries = Vec::new();
    for (index, (table, fields)) in tables.iter().zip(inputs).enumerate() {
        let table = sql_name(table);
        let order = crate::table_ops::stable_order(conn, &table, fields)?;
        let mut columns = rule
            .fields
            .iter()
            .zip(&rule.mappings[index])
            .map(|(target, source)| {
                let expression = match source {
                    Some(source) => sql_name(source),
                    None => format!("CAST(NULL AS {})", target.data_type),
                };
                format!("{expression} AS {}", sql_name(&target.name))
            })
            .collect::<Vec<_>>();
        for (name, kind) in [
            ("__company", "VARCHAR"),
            ("__file", "VARCHAR"),
            ("__sheet", "VARCHAR"),
            ("__row", "BIGINT"),
            ("__record", "BIGINT"),
        ] {
            if fields.iter().any(|field| field.name == name) {
                columns.push(name.into());
            } else {
                columns.push(format!("CAST(NULL AS {kind}) AS {name}"));
            }
        }
        columns.push(format!("{index} AS __append_input"));
        columns.push(format!(
            "ROW_NUMBER() OVER (ORDER BY {order}) AS __append_position"
        ));
        queries.push(format!("SELECT {} FROM {table}", columns.join(", ")));
    }
    conn.execute_batch(&format!(
        "CREATE TABLE {} AS SELECT * EXCLUDE (__append_position),
         ROW_NUMBER() OVER (ORDER BY __append_input, __append_position) AS __order
         FROM ({})",
        sql_name(output),
        queries.join(" UNION ALL "),
    ))
    .map_err(|err| format!("追加执行失败：{err}"))
}

#[derive(serde::Deserialize)]
pub(crate) struct JoinKey {
    pub left: String,
    pub right: String,
}

#[derive(serde::Deserialize)]
struct JoinColumn {
    side: String,
    source: String,
    target: String,
}

#[derive(serde::Deserialize)]
struct JoinRule {
    mode: String,
    keys: Vec<JoinKey>,
    columns: Vec<JoinColumn>,
    #[serde(default)]
    match_empty: bool,
    #[serde(default)]
    allow_expand: bool,
}

pub(crate) const EXPANSION_LIMIT: i64 = 1_000_000;

pub(crate) fn join_fields(
    node: &Node,
    left: &[crate::model::FieldDescriptor],
    right: &[crate::model::FieldDescriptor],
) -> Result<Vec<crate::model::FieldDescriptor>, String> {
    let rule: JoinRule = serde_json::from_value(node.config.clone())
        .map_err(|err| format!("关联规则不完整：{err}"))?;
    validate_fields(node, &rule, left, right)
}

fn validate_fields(
    node: &Node,
    rule: &JoinRule,
    left: &[crate::model::FieldDescriptor],
    right: &[crate::model::FieldDescriptor],
) -> Result<Vec<crate::model::FieldDescriptor>, String> {
    if rule.keys.is_empty() || rule.columns.is_empty() {
        return Err("请至少选择一组匹配键和一列输出".into());
    }
    for key in &rule.keys {
        let l = left
            .iter()
            .find(|field| field.name == key.left)
            .ok_or_else(|| format!("左侧缺少匹配键 {}", key.left))?;
        let r = right
            .iter()
            .find(|field| field.name == key.right)
            .ok_or_else(|| format!("右侧缺少匹配键 {}", key.right))?;
        if l.data_type != r.data_type {
            return Err(format!("匹配键类型不一致：{} 与 {}", key.left, key.right));
        }
    }
    let mut names = std::collections::BTreeSet::from(["match_status".to_owned()]);
    let mut output = Vec::new();
    for column in &rule.columns {
        let fields = match column.side.as_str() {
            "left" => left,
            "right" => right,
            _ => return Err("带回列须选择左侧或右侧".into()),
        };
        crate::field_contract::business_name(&column.target)?;
        if !names.insert(column.target.to_lowercase()) {
            return Err(format!("关联输出列重复：{}", column.target));
        }
        let mut field = fields
            .iter()
            .find(|field| field.name == column.source)
            .cloned()
            .ok_or_else(|| format!("关联输入缺少列 {}", column.source))?;
        field.name = column.target.clone();
        field.nullable = true;
        output.push(field);
    }
    output.push(crate::model::FieldDescriptor {
        name: "match_status".into(),
        data_type: "VARCHAR".into(),
        nullable: false,
        origin: node.id.clone(),
        confirmed: true,
    });
    for (side, fields) in [("left", left), ("right", right)] {
        for name in ["__file", "__sheet", "__row", "__record"] {
            if let Some(field) = fields.iter().find(|field| field.name == name) {
                let mut field = field.clone();
                field.name = format!("__{side}{name}");
                field.nullable = true;
                output.push(field);
            }
        }
    }
    Ok(output)
}

pub(crate) fn execute_join(
    conn: &duckdb::Connection,
    node: &Node,
    left: &str,
    right: &str,
    output: &str,
) -> Result<(), String> {
    let rule: JoinRule = serde_json::from_value(node.config.clone())
        .map_err(|err| format!("关联规则不完整：{err}"))?;
    let join = match rule.mode.as_str() {
        "inner" => "INNER",
        "left" => "LEFT",
        "right" => "RIGHT",
        "full" => "FULL OUTER",
        _ => return Err("请选择有效的连接方式".into()),
    };
    let left_fields = crate::engine::descriptors(conn, left, left)?;
    let right_fields = crate::engine::descriptors(conn, right, right)?;
    validate_fields(node, &rule, &left_fields, &right_fields)?;
    let (counts, predicate) = preflight(conn, left, right, &rule.keys, rule.match_empty)?;
    let expands = counts.expands_left || counts.expands_right;
    let expected = match rule.mode.as_str() {
        "inner" => Some(counts.matched),
        "left" => counts.matched.checked_add(counts.left_only),
        "right" => counts.matched.checked_add(counts.right_only),
        _ => counts
            .matched
            .checked_add(counts.left_only)
            .and_then(|value| value.checked_add(counts.right_only)),
    }
    .ok_or("预计关联行数超出支持范围")?;
    if expands && (!rule.allow_expand || expected > EXPANSION_LIMIT) {
        return Err(format!(
            "存在重复键匹配，预计匹配 {} 行、仅左 {} 行、仅右 {} 行，本方式输出 {expected} 行。须明确允许展开，展开上限为 {EXPANSION_LIMIT} 行，超限请先汇总。",
            counts.matched,counts.left_only,counts.right_only,
        ));
    }
    let mut projections = Vec::new();
    for column in rule.columns {
        let side = if column.side == "left" { "l" } else { "r" };
        projections.push(format!(
            "{side}.{} AS {}",
            sql_name(&column.source),
            sql_name(&column.target)
        ));
    }
    projections.push("CASE WHEN l.__audit_join_present IS NULL THEN '仅右侧' WHEN r.__audit_join_present IS NULL THEN '仅左侧' ELSE '匹配' END AS match_status".into());
    for (alias, fields, side) in [("l", &left_fields, "left"), ("r", &right_fields, "right")] {
        for field in ["__file", "__sheet", "__row", "__record"] {
            if fields.iter().any(|item| item.name == field) {
                projections.push(format!("{alias}.{field} AS __{side}{field}"));
            }
        }
    }
    conn.execute_batch(&format!(
        "CREATE TABLE {} AS SELECT {}
         FROM (SELECT *, TRUE AS __audit_join_present FROM {}) l
         {join} JOIN (SELECT *, TRUE AS __audit_join_present FROM {}) r ON {predicate}",
        sql_name(output),
        projections.join(", "),
        sql_name(left),
        sql_name(right),
    ))
    .map_err(|err| format!("关联执行失败：{err}"))?;
    for (category, status) in [
        ("matched", "匹配"),
        ("left_only", "仅左侧"),
        ("right_only", "仅右侧"),
    ] {
        conn.execute_batch(&format!(
            "CREATE VIEW {} AS SELECT * FROM {} WHERE match_status = '{}'",
            sql_name(&format!("{output}__{category}")),
            sql_name(output),
            status,
        ))
        .map_err(|err| format!("关联复核视图创建失败：{err}"))?;
    }
    Ok(())
}

pub(crate) fn preflight(
    conn: &duckdb::Connection,
    left_table: &str,
    right_table: &str,
    keys: &[JoinKey],
    match_empty: bool,
) -> Result<(JoinCounts, String), String> {
    if keys.is_empty() {
        return Err("至少选择一组匹配键".into());
    }
    let left_fields = crate::engine::column_types(conn, left_table)?;
    let right_fields = crate::engine::column_types(conn, right_table)?;
    let mut checks = Vec::new();
    for key in keys {
        let left_type = left_fields
            .get(&key.left)
            .ok_or_else(|| format!("左侧缺少匹配键 {}", key.left))?;
        let right_type = right_fields
            .get(&key.right)
            .ok_or_else(|| format!("右侧缺少匹配键 {}", key.right))?;
        if left_type != right_type {
            return Err(format!(
                "匹配键类型不一致：{}（{left_type}）与 {}（{right_type}）",
                key.left, key.right
            ));
        }
        let left = sql_name(&key.left);
        let right = sql_name(&key.right);
        checks.push(format!("l.{left} = r.{right}"));
        if !match_empty && left_type == "VARCHAR" {
            checks.push(format!("l.{left} <> ''"));
        }
    }
    let grouped = |table: &str, fields: Vec<&str>| {
        let fields = fields
            .into_iter()
            .map(sql_name)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT {fields}, COUNT(*) AS __audit_join_count FROM {} GROUP BY {fields}",
            sql_name(table)
        )
    };
    let left = grouped(
        left_table,
        keys.iter().map(|key| key.left.as_str()).collect(),
    );
    let right = grouped(
        right_table,
        keys.iter().map(|key| key.right.as_str()).collect(),
    );
    let predicate = checks.join(" AND ");
    let counts = count_pairs(conn, &left, &right, &predicate)?;
    Ok((counts, predicate))
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct JoinCounts {
    pub matched: i64,
    pub left_only: i64,
    pub right_only: i64,
    pub expands_left: bool,
    pub expands_right: bool,
}

pub(crate) fn preflight_node(
    conn: &duckdb::Connection,
    node: &Node,
    left: &str,
    right: &str,
) -> Result<JoinCounts, String> {
    let rule: JoinRule =
        serde_json::from_value(node.config.clone()).map_err(|error| error.to_string())?;
    let left_fields = crate::engine::descriptors(conn, left, left)?;
    let right_fields = crate::engine::descriptors(conn, right, right)?;
    validate_fields(node, &rule, &left_fields, &right_fields)?;
    preflight(conn, left, right, &rule.keys, rule.match_empty).map(|(counts, _)| counts)
}

/// Count from grouped keys before materializing pairs. Expressions are built
/// from quoted column names by the caller and include the configured null rules.
pub(crate) fn count_pairs(
    conn: &duckdb::Connection,
    left_keys: &str,
    right_keys: &str,
    predicate: &str,
) -> Result<JoinCounts, String> {
    conn.query_row(
        &format!(
            "WITH l AS ({left_keys}), r AS ({right_keys})
             SELECT
                 COALESCE(SUM(l.__audit_join_count::HUGEINT * r.__audit_join_count), 0)::BIGINT,
                 COALESCE(SUM(l.__audit_join_count) FILTER (WHERE r.__audit_join_count IS NULL), 0)::BIGINT,
                 COALESCE(SUM(r.__audit_join_count) FILTER (WHERE l.__audit_join_count IS NULL), 0)::BIGINT,
                 COALESCE(BOOL_OR(r.__audit_join_count > 1 AND l.__audit_join_count IS NOT NULL), false),
                 COALESCE(BOOL_OR(l.__audit_join_count > 1 AND r.__audit_join_count IS NOT NULL), false)
             FROM l FULL OUTER JOIN r ON {predicate}"
        ),
        [],
        |row| {
            Ok(JoinCounts {
                matched: row.get(0)?,
                left_only: row.get(1)?,
                right_only: row.get(2)?,
                expands_left: row.get(3)?,
                expands_right: row.get(4)?,
            })
        },
    )
    .map_err(|err| format!("无法预检关联行数：{err}"))
}

/// Resolve saved multi-table inputs by configured port order, independent of edge order.
pub(crate) fn ordered_inputs<'a>(
    workflow: &'a Workflow,
    node: &Node,
    ports: &[&str],
) -> Result<Vec<&'a Edge>, String> {
    let incoming: Vec<_> = workflow
        .edges
        .iter()
        .filter(|edge| edge.target == node.id)
        .collect();
    if incoming.len() != ports.len() {
        return Err(format!(
            "{} 需要 {} 条输入连线，实际 {} 条",
            node.label,
            ports.len(),
            incoming.len()
        ));
    }
    ports
        .iter()
        .map(|port| {
            let mut matches = incoming
                .iter()
                .copied()
                .filter(|edge| edge.target_port.as_deref() == Some(*port));
            let edge = matches
                .next()
                .ok_or_else(|| format!("{} 缺少 {port} 端口的输入", node.label))?;
            if matches.next().is_some() {
                return Err(format!("{} 的 {port} 端口重复连接", node.label));
            }
            Ok(edge)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_expansion_checks_one_million_row_boundary_and_amount_multiplicity() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a AS SELECT '001' AS code, i AS amount, i AS __row FROM range(1000) t(i);
             CREATE TABLE b AS SELECT '001' AS code, i AS amount, i AS __row FROM range(1000) t(i);"
        ).unwrap();
        let node:Node=serde_json::from_value(json!({
            "id":"join","kind":"join","version":2,"label":"关联","position":{"x":0,"y":0},
            "config":{"mode":"inner","allow_expand":true,"keys":[{"left":"code","right":"code"}],
                "columns":[{"side":"left","source":"amount","target":"左金额"},{"side":"right","source":"amount","target":"右金额"}]}
        })).unwrap();
        let started = std::time::Instant::now();
        execute_join(&conn, &node, "a", "b", "expanded").unwrap();
        let result: (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*),SUM(左金额)::BIGINT,SUM(右金额)::BIGINT FROM expanded",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(result, (1_000_000, 499_500_000, 499_500_000));
        conn.execute_batch("INSERT INTO a VALUES ('001',1000,1000)")
            .unwrap();
        assert!(execute_join(&conn, &node, "a", "b", "blocked_expansion")
            .unwrap_err()
            .contains("上限"));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/workbench-rebuild-acceptance/reports");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("join-expansion-boundary.json"),serde_json::to_vec_pretty(&json!({
            "kind":"synthetic-in-memory-boundary","input_left":1000,"input_right":1000,"output":result.0,
            "sum_left":result.1,"sum_right":result.2,"limit":EXPANSION_LIMIT,"elapsed_ms":started.elapsed().as_millis(),
            "above_limit_rejected":true,"scope":"single machine, two numeric fields, in-memory DuckDB; no general performance claim"
        })).unwrap()).unwrap();
    }

    #[test]
    fn appends_three_mapped_tables_in_port_order() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a AS SELECT * FROM (VALUES ('001', 1), ('002', 2)) t(code, __order);
             CREATE TABLE b AS SELECT * FROM (VALUES ('003', 10, 1), ('004', 20, 2), ('005', 30, 3)) t(account, amount, __order);
             CREATE TABLE c AS SELECT '006' AS id, 1 AS __order;",
        ).unwrap();
        let node: Node = serde_json::from_value(json!({
            "id":"union","kind":"union","version":2,"label":"追加","position":{"x":0,"y":0},
            "config":{"ports":["a","b","c"],
                      "fields":[{"name":"编号","data_type":"VARCHAR"},{"name":"金额","data_type":"INTEGER"}],
                      "mappings":[["code",null],["account","amount"],["id",null]],
                      "excluded":[[],[],[]]}
        })).unwrap();
        execute_append(
            &conn,
            &node,
            &["a".into(), "b".into(), "c".into()],
            "combined",
        )
        .unwrap();
        let inputs = ["a", "b", "c"]
            .iter()
            .map(|name| crate::engine::descriptors(&conn, name, name).unwrap())
            .collect::<Vec<_>>();
        let expected = AppendRule::read(&node)
            .unwrap()
            .output_fields(&node, &inputs.iter().map(Vec::as_slice).collect::<Vec<_>>())
            .unwrap()
            .into_iter()
            .map(|field| (field.name, field.data_type))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            expected,
            crate::engine::column_types(&conn, "combined").unwrap()
        );
        let outcome: (i64, i64, i64, String) = conn
            .query_row(
                "SELECT COUNT(*), SUM(金额)::BIGINT, COUNT(*) FILTER (WHERE 金额 IS NULL),
                    STRING_AGG(编号, ',' ORDER BY __order) FROM combined",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(outcome, (6, 60, 3, "001,002,003,004,005,006".into()));
    }

    #[test]
    fn executes_all_join_modes_and_preserves_each_side() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a AS SELECT * FROM (VALUES ('001',2),('002',3),(NULL,4)) t(code,__row);
             CREATE TABLE b AS SELECT * FROM (VALUES ('001',5),('003',6)) t(account,__row);",
        )
        .unwrap();
        let mut node: Node = serde_json::from_value(json!({
            "id":"join","kind":"join","version":2,"label":"关联","position":{"x":0,"y":0},
            "config":{"keys":[{"left":"code","right":"account"}],
                      "columns":[{"side":"left","source":"code","target":"左编码"},
                                 {"side":"right","source":"account","target":"右编码"}]}
        }))
        .unwrap();
        for (mode, expected) in [("inner", 1), ("left", 3), ("right", 2), ("full", 4)] {
            node.config["mode"] = json!(mode);
            execute_join(&conn, &node, "a", "b", mode).unwrap();
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {}", sql_name(mode)),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, expected);
        }
        let origins: (i32, i32) = conn
            .query_row(
                "SELECT __left__row, __right__row FROM \"full\" WHERE match_status = '匹配'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(origins, (2, 5));
        let null_status: String = conn
            .query_row(
                "SELECT match_status FROM \"full\" WHERE __left__row = 4",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(null_status, "仅左侧");
        conn.execute_batch("INSERT INTO b VALUES ('001',7)")
            .unwrap();
        assert!(execute_join(&conn, &node, "a", "b", "blocked")
            .unwrap_err()
            .contains("重复键"));
    }

    #[test]
    fn predicts_pairs_without_materializing_duplicate_rows() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let left = "SELECT k, COUNT(*) AS __audit_join_count FROM (VALUES ('001'), ('002'), ('002'), (NULL), ('')) t(k) GROUP BY k";
        let right = "SELECT k, COUNT(*) AS __audit_join_count FROM (VALUES ('001'), ('002'), ('002'), ('003'), (NULL), ('')) t(k) GROUP BY k";
        assert_eq!(
            count_pairs(&conn, left, right, "l.k = r.k").unwrap(),
            JoinCounts {
                matched: 6,
                left_only: 1,
                right_only: 2,
                expands_left: true,
                expands_right: true,
            }
        );
        assert_eq!(
            count_pairs(&conn, left, right, "l.k = r.k AND l.k <> ''").unwrap(),
            JoinCounts {
                matched: 5,
                left_only: 2,
                right_only: 3,
                expands_left: true,
                expands_right: true,
            }
        );
    }

    #[test]
    fn preflight_uses_distinct_column_names_and_rejects_type_coercion() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a AS SELECT '001' AS code;
             CREATE TABLE b AS SELECT '001' AS account;
             CREATE TABLE c AS SELECT 1 AS account;",
        )
        .unwrap();
        let keys = [JoinKey {
            left: "code".into(),
            right: "account".into(),
        }];
        assert_eq!(
            preflight(&conn, "a", "b", &keys, false).unwrap().0.matched,
            1
        );
        assert!(preflight(&conn, "a", "c", &keys, false).is_err());
    }

    #[test]
    fn resolves_by_port_and_rejects_duplicate_or_missing_ports() {
        let mut workflow: Workflow = serde_json::from_value(json!({
            "schema_version": 1, "name": "端口测试",
            "nodes": [{"id":"join","kind":"join","version":1,"label":"关联","position":{"x":0,"y":0},"config":{}}],
            "edges": [
                {"id":"r","source":"second","target":"join","target_port":"right"},
                {"id":"l","source":"first","target":"join","target_port":"left"}
            ]
        })).unwrap();
        let ids: Vec<_> = ordered_inputs(&workflow, &workflow.nodes[0], &["left", "right"])
            .unwrap()
            .into_iter()
            .map(|edge| edge.source.as_str())
            .collect();
        assert_eq!(ids, ["first", "second"]);
        workflow.edges[0].target_port = Some("left".into());
        assert!(ordered_inputs(&workflow, &workflow.nodes[0], &["left", "right"]).is_err());
        workflow.edges[0].target_port = None;
        assert!(ordered_inputs(&workflow, &workflow.nodes[0], &["left", "right"]).is_err());
    }
}
