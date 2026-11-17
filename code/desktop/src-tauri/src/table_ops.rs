//! Single-input table operations. All identifiers are checked against the input schema.
use crate::{
    engine, field_contract,
    model::{FieldDescriptor, Node},
    workflow::{required_str, required_strings, sql_name},
};
use duckdb::Connection;
use serde_json::Value;
use std::collections::BTreeSet;

pub fn aggregate_keys(cfg: &Value) -> Result<Vec<String>, String> {
    required_strings(
        cfg,
        if cfg.get("group_by").is_some() {
            "group_by"
        } else {
            "keys"
        },
    )
}
fn field<'a>(input: &'a [FieldDescriptor], name: &str) -> Result<&'a FieldDescriptor, String> {
    input
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("上游缺少字段：{name}"))
}
fn keys(cfg: &Value, input: &[FieldDescriptor]) -> Result<Vec<String>, String> {
    let keys = required_strings(cfg, "keys")?;
    if keys.is_empty() || keys.iter().collect::<BTreeSet<_>>().len() != keys.len() {
        return Err("请选择不重复的判断字段".into());
    }
    for k in &keys {
        field(input, k)?;
    }
    Ok(keys)
}
pub fn predicate(cfg: &Value, input: &[FieldDescriptor]) -> Result<String, String> {
    if let Some(items) = cfg.get("conditions") {
        let items = items.as_array().ok_or("conditions须为数组")?;
        if items.is_empty()&&cfg.get("cleared").and_then(Value::as_bool)==Some(true){return Ok("TRUE".into());}
        if items.is_empty() || items.len() > 32 {
            return Err("请选择1至32个条件".into());
        }
        let join = match required_str(cfg, "match")? {
            "all" => " AND ",
            "any" => " OR ",
            _ => return Err("条件组合须为all或any".into()),
        };
        return items
            .iter()
            .map(|c| one_condition(c, input).map(|v| format!("({v})")))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join(join));
    }
    one_condition(cfg.get("condition").ok_or("缺少筛选条件")?, input)
}
fn one_condition(c: &Value, input: &[FieldDescriptor]) -> Result<String, String> {
    if matches!(c.get("operator").and_then(Value::as_str),Some("include_records"|"exclude_records")){
        let names=required_strings(c,"keys")?;
        let values=c.get("records").and_then(Value::as_array).filter(|values|!values.is_empty()&&values.len()<=1000).ok_or("请选择1至1000条来源可区分的记录")?;
        if names.is_empty(){return Err("所选记录缺少来源身份".into());}
        let mut rows=Vec::new();
        for row in values{
            let row=row.as_array().filter(|row|row.len()==names.len()).ok_or("来源记录格式无效")?;
            let cells=row.iter().map(|value| {
                if value.is_null(){Ok("NULL::VARCHAR".to_owned())}
                else{Ok(format!("CAST({} AS VARCHAR)",engine::lit(value.as_str().ok_or("来源身份须为文本或NULL")?)))}
            }).collect::<Result<Vec<_>,String>>()?;
            rows.push(format!("({})",cells.join(",")));
        }
        let mut terms=Vec::new();
        let mut aliases=Vec::new();
        for (index,name) in names.iter().enumerate(){
            let ty=&field(input,name)?.data_type;
            let alias=format!("__key_{index}");
            terms.push(format!("{} IS NOT DISTINCT FROM CAST(__selected_records.{alias} AS {ty})",sql_name(name)));
            aliases.push(alias);
        }
        let predicate=format!("EXISTS (SELECT 1 FROM (VALUES {}) AS __selected_records({}) WHERE {})",rows.join(","),aliases.join(","),terms.join(" AND "));
        return Ok(if c["operator"]=="exclude_records"{format!("NOT {predicate}")}else{predicate});
    }
    let name = required_str(c, "field")?;
    let f = field(input, name)?;
    let col = sql_name(name);
    let op = required_str(c, "operator")?;
    if matches!(op, "include_values" | "exclude_values") {
        let values = c.get("values").and_then(Value::as_array).filter(|v| !v.is_empty()).ok_or("请选择至少一个筛选值")?;
        let terms = values.iter().map(|value| {
            if value.is_null() { return Ok(format!("{col} IS NULL")); }
            let text = value.as_str().ok_or("筛选值必须为文本或空值")?;
            Ok::<_, String>(format!("CAST({col} AS VARCHAR) IS NOT DISTINCT FROM {}", engine::lit(text)))
        }).collect::<Result<Vec<_>, _>>()?;
        let matches = format!("({})", terms.join(" OR "));
        return Ok(if op == "include_values" { matches } else { format!("NOT {matches}") });
    }
    if op == "is_empty" {
        return Ok(format!("{col} IS NULL OR TRIM(CAST({col} AS VARCHAR))=''"));
    }
    if op == "is_not_empty" {
        return Ok(format!(
            "{col} IS NOT NULL AND TRIM(CAST({col} AS VARCHAR))<>''"
        ));
    }
    let value = c.get("value").and_then(Value::as_str).ok_or("缺少比较值")?;
    if op.starts_with("date_"){
        if f.data_type!="DATE"{return Err("日期比较需要DATE字段，请先用明确格式转换".into());}
        let date=chrono::NaiveDate::parse_from_str(value,"%Y-%m-%d").map_err(|_|"日期比较值须为有效的yyyy-mm-dd")?;
        if date.format("%Y-%m-%d").to_string()!=value{return Err("日期比较值须严格使用yyyy-mm-dd".into());}
        let cmp=match op{
            "date_eq"=>"=","date_neq"=>"<>","date_gt"=>">","date_gte"=>">=","date_lt"=>"<","date_lte"=>"<=",
            _=>return Err("未知日期条件".into()),
        };
        return Ok(format!("{col} {cmp} CAST({} AS DATE)",engine::lit(value)));
    }
    if op.starts_with("numeric_") {
        if !field_contract::numeric(&f.data_type) {
            return Err("数值筛选需要先转换金额".into());
        }
        field_contract::decimal_literal(value)?;
        let cmp = match op {
            "numeric_gt" => ">",
            "numeric_gte" => ">=",
            "numeric_lt" => "<",
            "numeric_lte" => "<=",
            "numeric_eq" => "=",
            "numeric_neq" => "<>",
            _ => return Err("未知数值条件".into()),
        };
        return Ok(format!(
            "{col} {cmp} CAST({} AS DECIMAL(28,4))",
            engine::lit(value)
        ));
    }
    match op {
        "equals" => Ok(format!("CAST({col} AS VARCHAR)={}", engine::lit(value))),
        "not_equals" => Ok(format!("CAST({col} AS VARCHAR)<>{}", engine::lit(value))),
        "contains" => Ok(format!(
            "contains(CAST({col} AS VARCHAR),{})",
            engine::lit(value)
        )),
        _ => Err("未知文本条件".into()),
    }
}
pub fn check(node: &Node, input: &[FieldDescriptor]) -> Result<Vec<FieldDescriptor>, String> {
    let cfg = &node.config;
    let mut out = input.to_vec();
    match node.kind.as_str() {
        "sort" => {
            sort_terms(cfg, input)?;
        }
        "sequence" => {
            cfg.get("start").and_then(Value::as_i64).ok_or("编号起点须为64位整数")?;
            add(&mut out,node,required_str(cfg,"target")?,"BIGINT")?;
        }
        "duplicates" | "deduplicate" => {
            keys(cfg, input)?;
            if node.kind == "deduplicate" {
                if !matches!(required_str(cfg, "keep")?, "first" | "last") {
                    return Err("保留方式须为first或last".into());
                }
            } else {
                for (key, ty) in [
                    ("target", "BOOLEAN"),
                    ("count_target", "BIGINT"),
                    ("rank_target", "BIGINT"),
                ] {
                    let name = required_str(cfg, key)?;
                    add(&mut out, node, name, ty)?;
                }
            }
        }
        "aggregate" if node.version == 2 => {
            let group = aggregate_keys(cfg)?;
            if group.is_empty() || group.iter().collect::<BTreeSet<_>>().len() != group.len() {
                return Err("请选择不重复的分组字段".into());
            }
            out = group
                .iter()
                .map(|k| field(input, k).cloned())
                .collect::<Result<Vec<_>, _>>()?;
            let metrics = cfg
                .get("metrics")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 32)
                .ok_or("请选择1至32个统计项")?;
            for m in metrics {
                let op = required_str(m, "op")?;
                let ty = if op == "sum" {
                    let f = field(input, required_str(m, "field")?)?;
                    if !field_contract::numeric(&f.data_type) {
                        return Err("求和需要数值字段".into());
                    }
                    if f.data_type.starts_with("DECIMAL") {
                        "DECIMAL(38,4)"
                    } else if f.data_type == "DOUBLE" || f.data_type == "FLOAT" {
                        "DOUBLE"
                    } else {
                        "HUGEINT"
                    }
                } else {
                    if !matches!(op, "rows" | "count" | "distinct") {
                        return Err("未知统计方式".into());
                    }
                    if op != "rows" {
                        field(input, required_str(m, "field")?)?;
                    }
                    "BIGINT"
                };
                add(&mut out, node, required_str(m, "target")?, ty)?;
                if op == "sum" {
                    add(
                        &mut out,
                        node,
                        &format!("{}__nulls", required_str(m, "target")?),
                        "BIGINT",
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(out)
}
fn add(out: &mut Vec<FieldDescriptor>, node: &Node, name: &str, ty: &str) -> Result<(), String> {
    field_contract::business_name(name)?;
    if out.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
        return Err(format!("目标字段重名：{name}"));
    }
    out.push(FieldDescriptor {
        name: name.into(),
        data_type: ty.into(),
        nullable: true,
        origin: node.id.clone(),
        confirmed: true,
    });
    Ok(())
}
fn sort_terms(cfg: &Value, input: &[FieldDescriptor]) -> Result<Vec<String>, String> {
    let items = cfg
        .get("order")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 32)
        .ok_or("请选择排序字段")?;
    let mut seen = BTreeSet::new();
    items
        .iter()
        .map(|item| {
            let name = required_str(item, "field")?;
            field(input, name)?;
            if !seen.insert(name) {
                return Err("排序字段重复".into());
            }
            let direction = match required_str(item, "direction")? {
                "asc" => "ASC",
                "desc" => "DESC",
                _ => return Err("排序方向无效".into()),
            };
            Ok(format!("{} {direction} NULLS LAST", sql_name(name)))
        })
        .collect()
}
pub fn stable_order(
    conn: &Connection,
    input: &str,
    fields: &[FieldDescriptor],
) -> Result<String, String> {
    if fields.iter().any(|f| f.name == "__order") {
        return Ok("__order".into());
    }
    let names = ["__file", "__sheet", "__record", "__row"];
    if !names.iter().all(|n| fields.iter().any(|f| f.name == *n)) {
        return Err("该输入没有可靠顺序，请先明确排序；旧多输入来源须单独核对".into());
    }
    let order = names.join(",");
    let ambiguous:i64=conn.query_row(&format!("SELECT COUNT(*) FROM (SELECT {order} FROM {input} GROUP BY {order} HAVING COUNT(*)>1)"),[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if ambiguous > 0 {
        return Err("来源顺序存在重复，不能猜测首条或末条".into());
    }
    Ok(order)
}
pub fn execute(
    conn: &Connection,
    node: &Node,
    input: &str,
    out: &str,
    fields: &[FieldDescriptor],
) -> Result<(), String> {
    check(node, fields)?;
    let cfg = &node.config;
    let select = if fields.iter().any(|f| f.name == "__order") {
        "* EXCLUDE (__order)"
    } else {
        "*"
    };
    let sql = match node.kind.as_str() {
        "sort" => {
            let stable = stable_order(conn, input, fields)?;
            let mut order = sort_terms(cfg, fields)?;
            order.push(stable);
            format!("CREATE TABLE {out} AS SELECT {select},ROW_NUMBER() OVER(ORDER BY {}) AS __order FROM {input}",order.join(","))
        }
        "duplicates" | "deduplicate" => {
            let stable = stable_order(conn, input, fields)?;
            let keys = keys(cfg, fields)?
                .iter()
                .map(|s| sql_name(s))
                .collect::<Vec<_>>()
                .join(",");
            if node.kind == "duplicates" {
                format!("CREATE TABLE {out} AS SELECT *,COUNT(*) OVER(PARTITION BY {keys})>1 AS {},COUNT(*) OVER(PARTITION BY {keys}) AS {},ROW_NUMBER() OVER(PARTITION BY {keys} ORDER BY {stable}) AS {} FROM {input}",sql_name(required_str(cfg,"target")?),sql_name(required_str(cfg,"count_target")?),sql_name(required_str(cfg,"rank_target")?))
            } else {
                let keep = required_str(cfg, "keep")?;
                let ordering = stable
                    .split(',')
                    .map(|s| {
                        format!(
                            "{s} {} NULLS LAST",
                            if keep == "last" { "DESC" } else { "ASC" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let rank = format!("ROW_NUMBER() OVER(PARTITION BY {keys} ORDER BY {ordering})");
                let excluded = format!("{}__excluded", out.trim_matches('"'));
                conn.execute_batch(&format!(
                    "CREATE TABLE {} AS SELECT * FROM {input} QUALIFY {rank}>1",
                    sql_name(&excluded)
                ))
                .map_err(|e| e.to_string())?;
                format!("CREATE TABLE {out} AS SELECT * FROM {input} QUALIFY {rank}=1")
            }
        }
        "sequence" => {
            let ordered=stable_order(conn,input,fields)?;
            let start=cfg.get("start").and_then(Value::as_i64).ok_or("编号起点须为整数")?;
            format!("CREATE TABLE {out} AS SELECT *,CAST(ROW_NUMBER() OVER (ORDER BY {ordered})::HUGEINT + ({start})::HUGEINT - 1 AS BIGINT) AS {} FROM {input}",sql_name(required_str(cfg,"target")?))
        }
        "aggregate" => {
            let group = aggregate_keys(cfg)?
                .iter()
                .map(|s| sql_name(s))
                .collect::<Vec<_>>();
            let mut selects = group.clone();
            for (i, g) in group.iter().enumerate() {
                selects.push(format!("{g} IS NULL AS __group_null_{i}"));
            }
            for m in cfg["metrics"].as_array().ok_or("缺少统计项")? {
                let op = required_str(m, "op")?;
                let target = sql_name(required_str(m, "target")?);
                let expr = if op == "rows" {
                    "COUNT(*)".into()
                } else {
                    let f = sql_name(required_str(m, "field")?);
                    match op {
                        "sum" => {
                            let null_name = format!("{}__nulls", required_str(m, "target")?);
                            selects
                                .push(format!("COUNT(*)-COUNT({f}) AS {}", sql_name(&null_name)));
                            format!("SUM({f})")
                        }
                        "count" => format!("COUNT({f})"),
                        "distinct" => format!("COUNT(DISTINCT {f})"),
                        _ => return Err("未知统计项".into()),
                    }
                };
                selects.push(format!("{expr} AS {target}"));
            }
            selects.push("COUNT(*) AS __members".into());
            let groups = group.join(",");
            let ordered = group
                .iter()
                .map(|g| format!("{g} NULLS LAST"))
                .collect::<Vec<_>>()
                .join(",");
            format!("CREATE TABLE {out} AS SELECT *,ROW_NUMBER() OVER(ORDER BY {ordered}) AS __order FROM (SELECT {} FROM {input} GROUP BY {groups})",selects.join(","))
        }
        _ => return Err("不支持的表格操作".into()),
    };
    conn.execute_batch(&sql).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sequence_and_explicit_date_filters_keep_order_and_reject_ambiguity(){
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE input AS SELECT * FROM (VALUES (2,DATE '2024-03-01'),(1,DATE '2024-02-29')) t(__order,日期)").unwrap();
        let fields=engine::descriptors(&conn,"input","input").unwrap();
        let numbered=node("sequence",json!({"target":"编号","start":10}));
        execute(&conn,&numbered,"input","numbered",&fields).unwrap();
        let values:Vec<i64>=conn.prepare("SELECT 编号 FROM numbered ORDER BY __order").unwrap().query_map([],|row|row.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(values,[10,11]);
        let condition=json!({"conditions":[{"field":"日期","operator":"date_gt","value":"2024-02-29"}],"match":"all"});
        let condition=predicate(&condition,&fields).unwrap();
        let count:i64=conn.query_row(&format!("SELECT COUNT(*) FROM input WHERE {condition}"),[],|row|row.get(0)).unwrap();
        assert_eq!(count,1);
        assert!(predicate(&json!({"condition":{"field":"日期","operator":"date_gt","value":"03/01/2024"}}),&fields).is_err());
        assert_eq!(predicate(&json!({"conditions":[],"match":"all","cleared":true}),&fields).unwrap(),"TRUE");
        let selected=(1..=1000).map(|index|json!([index.to_string()])).collect::<Vec<_>>();
        let selected=predicate(&json!({"conditions":[{"operator":"include_records","keys":["__order"],"records":selected}],"match":"all"}),&fields).unwrap();
        let count:i64=conn.query_row(&format!("SELECT COUNT(*) FROM input WHERE {selected}"),[],|row|row.get(0)).unwrap();
        assert_eq!(count,2);
    }
    fn node(kind: &str, cfg: Value) -> Node {
        Node {
            id: "test".into(),
            kind: kind.into(),
            version: if kind == "aggregate" { 2 } else { 1 },
            label: kind.into(),
            position: crate::model::Point { x: 0., y: 0. },
            config: cfg,
        }
    }
    #[test]
    fn row_rules_keep_members_amounts_and_explicit_order() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE input AS SELECT * FROM (VALUES (1,'A','x',10.0000),(2,'A','x',10.0000),(3,'A','y',-3.0000),(4,'B','x',0.0000),(5,'B',NULL,NULL),(6,'B','x',2.0000)) t(__order,公司,分类,金额)").unwrap();
        let fields = engine::column_types(&c, "input")
            .unwrap()
            .into_iter()
            .map(|(name, data_type)| FieldDescriptor {
                name,
                data_type,
                nullable: true,
                origin: "source".into(),
                confirmed: true,
            })
            .collect::<Vec<_>>();
        let dedup = node(
            "deduplicate",
            json!({"keys":["公司","分类","金额"],"keep":"first"}),
        );
        execute(&c, &dedup, "input", "\"n_test\"", &fields).unwrap();
        let (rows, sum): (i64, String) = c
            .query_row(
                "SELECT COUNT(*),CAST(SUM(金额) AS VARCHAR) FROM n_test",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 5);
        assert_eq!(sum, "9.0000");
        assert_eq!(
            c.query_row("SELECT __order FROM n_test__excluded", [], |r| r
                .get::<_, i32>(0))
                .unwrap(),
            2
        );
        let sort = node(
            "sort",
            json!({"order":[{"field":"金额","direction":"desc"}]}),
        );
        execute(&c, &sort, "input", "sorted", &fields).unwrap();
        let order = c
            .prepare("SELECT CAST(金额 AS VARCHAR) FROM sorted ORDER BY __order")
            .unwrap()
            .query_map([], |r| r.get::<_, Option<String>>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(order[0].as_deref(), Some("10.0000"));
        assert_eq!(order.last(), Some(&None));
        let filter=predicate(&json!({"match":"any","conditions":[{"field":"公司","operator":"equals","value":"B"},{"field":"金额","operator":"numeric_lt","value":"0"}]}),&fields).unwrap();
        assert_eq!(
            c.query_row(
                &format!("SELECT COUNT(*) FROM input WHERE ({filter}) IS TRUE"),
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4
        );
        let agg = node(
            "aggregate",
            json!({"group_by":["公司"],"metrics":[{"op":"sum","field":"金额","target":"合计"},{"op":"rows","target":"记录数"},{"op":"distinct","field":"分类","target":"类别数"}]}),
        );
        execute(&c, &agg, "input", "agg", &fields).unwrap();
        let v: String = c
            .query_row(
                "SELECT CAST(合计 AS VARCHAR) FROM agg WHERE 公司='A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "17.0000");
        let last = node(
            "deduplicate",
            json!({"keys":["公司","分类","金额"],"keep":"last"}),
        );
        execute(&c, &last, "input", "last_kept", &fields).unwrap();
        assert_eq!(
            c.query_row("SELECT __order FROM last_kept__excluded", [], |r| r
                .get::<_, i32>(0))
                .unwrap(),
            1
        );
        let marked = node(
            "duplicates",
            json!({"keys":["公司","分类","金额"],"target":"重复","count_target":"条数","rank_target":"序号"}),
        );
        execute(&c, &marked, "input", "marked", &fields).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM marked WHERE 重复 IS TRUE",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        let invalid = node(
            "duplicates",
            json!({"keys":["公司"],"target":"金额","count_target":"条数","rank_target":"序号"}),
        );
        assert!(check(&invalid, &fields).is_err());
        let null_group = node(
            "aggregate",
            json!({"group_by":["分类"],"metrics":[{"op":"sum","field":"金额","target":"合计"},{"op":"distinct","field":"公司","target":"主体数"}]}),
        );
        execute(&c, &null_group, "input", "null_group", &fields).unwrap();
        let amount: Option<String> = c
            .query_row(
                "SELECT CAST(合计 AS VARCHAR) FROM null_group WHERE __group_null_0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(amount, None);
        assert_eq!(
            c.query_row("SELECT 合计__nulls FROM agg WHERE 公司='B'", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }
}
