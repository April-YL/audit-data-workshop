//! Read-only queries against a completed batch. Never execute the workflow here.
use crate::{
    engine, field_contract,
    model::*,
    workflow::{identifier, sql_name},
};
use duckdb::Connection;
use std::{collections::BTreeSet, fs, path::Path};
use sha2::{Digest,Sha256};

#[derive(Clone,serde::Deserialize,serde::Serialize)]
pub(crate) struct ExportScope {
    pub columns:Vec<String>,
    pub condition:Option<serde_json::Value>,
    pub records:Option<Vec<u64>>,
}

#[derive(serde::Serialize)]
pub(crate) struct ExportPlan {
    pub token:String,
    pub rows:i64,
    pub columns:Vec<String>,
    pub order:String,
}

pub(crate) fn export_query(
    conn:&Connection,result:&RunResult,node:&str,category:&str,scope:&ExportScope,
)->Result<(String,ExportPlan),String>{
    let table=resolve(result,node,category)?;
    let names=engine::columns(conn,&table)?;
    if scope.columns.is_empty() || scope.columns.iter().collect::<BTreeSet<_>>().len()!=scope.columns.len()
        || scope.columns.iter().any(|name|!names.contains(name))
    {return Err("请明确选择不重复的导出列".into());}
    if names.contains(&"__audit_export_ordinal".into()){return Err("结果含导出内部保留列名，请先更名".into());}
    let fields=engine::descriptors(conn,&table,node)?;
    let predicate=match &scope.condition {
        Some(condition)=>crate::table_ops::predicate(condition,&fields)?,
        None=>"TRUE".into(),
    };
    let records=match &scope.records {
        None=>"TRUE".into(),
        Some(records)=>{
            if records.is_empty()||records.len()>100000||records.contains(&0) {return Err("请明确选择1至100000条有效记录".into());}
            format!("__audit_export_ordinal IN ({})",records.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
        }
    };
    let order=engine::result_order(conn,&table,&names);
    let query=format!(
        "SELECT {} FROM (SELECT *,ROW_NUMBER() OVER (ORDER BY {order}) AS __audit_export_ordinal FROM {})
         WHERE ({predicate}) AND ({records}) ORDER BY __audit_export_ordinal",
        scope.columns.iter().map(|name|sql_name(name)).collect::<Vec<_>>().join(","),sql_name(&table),
    );
    let rows:i64=conn.query_row(&format!("SELECT COUNT(*) FROM ({query})"),[],|row|row.get(0)).map_err(|error|error.to_string())?;
    let identity=serde_json::to_vec(&(&result.run_dir,node,category,scope,rows,&names)).map_err(|error|error.to_string())?;
    Ok((query,ExportPlan{token:format!("{:x}",Sha256::digest(identity)),rows,columns:scope.columns.clone(),order:"当前结果顺序，条件筛选后保留顺序".into()}))
}

pub(crate) fn export_scoped(
    registry:&crate::result_registry::ResultRegistry,batch:&str,node:&str,category:&str,
    scope:&ExportScope,token:&str,path:&Path,
)->Result<ExportReceipt,String>{
    let result=registry.resolve(batch)?;
    crate::ensure_export_path(&result,path)?;
    if !path.extension().is_some_and(|extension|extension.eq_ignore_ascii_case("csv")){return Err("请选择CSV文件".into());}
    let _guard=engine::result_query_guard()?;
    let conn=Connection::open(&result.database).map_err(|error|error.to_string())?;
    let (query,plan)=export_query(&conn,&result,node,category,scope)?;
    if plan.token!=token{return Err("导出范围已变化，请重新预检".into());}
    engine::atomic_replace_checked(path,|temp|{
        conn.execute_batch(&format!("COPY ({query}) TO {} (HEADER, DELIMITER ',', QUOTE '\"', ESCAPE '\"')",engine::lit(&temp.to_string_lossy())))
            .map_err(|error|error.to_string())
    },false,|temp,target|registry.with_current(batch,||fs::rename(temp,target).map_err(|error|error.to_string())))?;
    Ok(ExportReceipt{path:path.to_string_lossy().into(),rows:plan.rows,node_id:node.into(),category:category.into()})
}

pub(crate) fn selected_record_filter(result:&RunResult,node:&str,records:&[u64],exclude:bool)->Result<serde_json::Value,String>{
    if records.is_empty()||records.len()>1000||records.contains(&0)||records.iter().collect::<BTreeSet<_>>().len()!=records.len(){
        return Err("请选择1至1000条不同记录，较大范围请使用条件筛选".into());
    }
    let _guard=engine::result_query_guard()?;
    let table=resolve(result,node,"normal")?;
    let conn=Connection::open(&result.database).map_err(|error|error.to_string())?;
    let columns=engine::columns(&conn,&table)?;
    let keys=columns.iter().filter(|name|{
        name.as_str()=="__append_input"||["__file","__sheet","__record","__row"].contains(&name.as_str())
        || ["__left__file","__left__sheet","__left__record","__left__row","__right__file","__right__sheet","__right__record","__right__row"].contains(&name.as_str())
    }).cloned().collect::<Vec<_>>();
    if keys.is_empty(){return Err("当前结果没有单行来源，请用条件筛选或回查组成明细".into());}
    let order=engine::result_order(&conn,&table,&columns);
    let sql=format!("SELECT {} FROM (SELECT *,ROW_NUMBER() OVER(ORDER BY {order}) AS __audit_record_position FROM {}) WHERE __audit_record_position IN ({})",
        keys.iter().map(|name|format!("CAST({} AS VARCHAR)",sql_name(name))).collect::<Vec<_>>().join(","),
        sql_name(&table),records.iter().map(u64::to_string).collect::<Vec<_>>().join(","));
    let mut statement=conn.prepare(&sql).map_err(|error|error.to_string())?;
    let values=statement.query_map([],|row|(0..keys.len()).map(|index|row.get::<_,Option<String>>(index)).collect::<duckdb::Result<Vec<_>>>())
        .map_err(|error|error.to_string())?.collect::<duckdb::Result<Vec<_>>>().map_err(|error|error.to_string())?;
    if values.len()!=records.len(){return Err("部分选中记录已不存在，请重新选择".into());}
    let condition=serde_json::json!({"operator":"include_records","keys":keys,"records":values});
    let config=serde_json::json!({"match":"all","conditions":[condition]});
    let predicate=crate::table_ops::predicate(&config,&engine::descriptors(&conn,&table,node)?)?;
    let count:i64=conn.query_row(&format!("SELECT COUNT(*) FROM {} WHERE {predicate}",sql_name(&table)),[],|row|row.get(0)).map_err(|error|error.to_string())?;
    if count!=records.len() as i64{return Err("所选来源对应多行，无法唯一保留或排除，请使用明确条件".into());}
    let mut config=config;
    if exclude{config["conditions"][0]["operator"]=serde_json::json!("exclude_records");}
    config["selected_fingerprints"]=serde_json::to_value(&result.input_fingerprints).map_err(|error|error.to_string())?;
    Ok(config)
}

pub(crate) fn resolve(result: &RunResult, node_id: &str, category: &str) -> Result<String, String> {
    identifier(node_id)?;
    let item = result
        .reviews
        .iter()
        .find(|r| r.node_id == node_id)
        .ok_or("该步骤没有有效批次")?;
    match category {
        "normal" => Ok(item.table.clone()),
        "excluded" if item.excluded > 0 => Ok(format!("{}__excluded", item.table)),
        "exceptions" if item.exceptions > 0 => Ok(format!("{}__exceptions", item.table)),
        "matched" | "left_only" | "right_only" => {
            let workflow: Workflow = serde_json::from_slice(
                &fs::read(Path::new(&result.run_dir).join("workflow.json")).map_err(|error|error.to_string())?
            ).map_err(|error|error.to_string())?;
            if !workflow.nodes.iter().any(|node|node.id==node_id&&node.kind=="join"&&node.version==2) {
                return Err("该步骤没有所选关联类别".into());
            }
            Ok(format!("{}__{category}",item.table))
        }
        _ => Err("该步骤没有所选类别的结果".into()),
    }
}

pub fn distinct_values(result: &RunResult, node: &str, field: &str, search: &str, offset: usize,filter_config:Option<serde_json::Value>) -> Result<Vec<Option<String>>, String> {
    let _guard = engine::result_query_guard()?;
    let table = resolve(result, node, "normal")?;
    let conn = Connection::open(&result.database).map_err(|error| error.to_string())?;
    if field.starts_with("__") || !engine::column_types(&conn, &table)?.contains_key(field) {
        return Err("结果中没有该筛选字段".into());
    }
    let col = sql_name(field);
    let mut filter=filter_config.unwrap_or_else(||serde_json::json!({"match":"all","conditions":[]}));
    let conditions=filter.get_mut("conditions").and_then(serde_json::Value::as_array_mut).ok_or("值列表条件格式无效")?;
    conditions.retain(|condition|condition.get("field").and_then(serde_json::Value::as_str)!=Some(field));
    let predicate=if conditions.is_empty(){"TRUE".into()}else{
        crate::table_ops::predicate(&filter,&engine::descriptors(&conn,&table,node)?)?
    };
    let table = sql_name(&table);
    let sql = format!("SELECT DISTINCT CAST({col} AS VARCHAR) AS value FROM {table} WHERE ({predicate}) AND (? = '' OR contains(CAST({col} AS VARCHAR), ?)) ORDER BY value NULLS FIRST LIMIT 201 OFFSET ?");
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement.query_map(duckdb::params![search, search, i64::try_from(offset).unwrap_or(i64::MAX)], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<Option<String>>, _>>().map_err(|error| error.to_string())
}
pub fn summarize(
    result: &RunResult,
    node: &str,
    category: &str,
    fields: &[String],
) -> Result<ResultSummary, String> {
    let _guard = engine::result_query_guard()?;
    if fields.is_empty()
        || fields.len() > 64
        || fields.iter().collect::<BTreeSet<_>>().len() != fields.len()
    {
        return Err("请选择1至64个不重复的数值字段".into());
    }
    let table = resolve(result, node, category)?;
    let conn = Connection::open(&result.database).map_err(|e| e.to_string())?;
    let types = engine::column_types(&conn, &table)?;
    let mut select = vec!["COUNT(*)".to_string()];
    if types.contains_key("__pivot_total") {
        return Err("透视结果已包含独立计算的总计，请查看总计行，避免再次相加造成重复".into());
    }
    let mut amount_status = Vec::new();
    for f in fields {
        if f.starts_with("__") || !types.get(f).is_some_and(|t| field_contract::numeric(t)) {
            return Err(format!("请选择实际数值字段，文本金额须先转换：{f}"));
        }
        let c = sql_name(f);
        select.push(format!("COUNT({c})"));
        select.push(format!("CAST(SUM({c}) AS VARCHAR)"));
        let status_name = format!("{f}__amount_status");
        let has_status = types.get(&status_name).is_some_and(|kind| kind == "VARCHAR");
        amount_status.push(has_status);
        if has_status {
            let status = sql_name(&status_name);
            select.push(format!("COUNT(*) FILTER (WHERE {status} = '空白')"));
            select.push(format!("COUNT(*) FILTER (WHERE {status} IN ('精度或科学计数法','格式无法转换','超出金额范围'))"));
            select.push(format!("COUNT(*) FILTER (WHERE {status} = '空白补零')"));
        }
    }
    let sql = format!("SELECT {} FROM {}", select.join(","), sql_name(&table));
    conn.query_row(&sql, [], |r| {
        let rows: i64 = r.get(0)?;
        let mut summaries = Vec::new();
        let mut index = 1;
        for (name, has_status) in fields.iter().zip(&amount_status) {
            let non_null_count = r.get::<_, i64>(index)?;
            let sum = r.get(index + 1)?;
            index += 2;
            let amount_counts = if *has_status {
                let counts = AmountCounts { blank: r.get(index)?, failed: r.get(index + 1)?, filled_zero: r.get(index + 2)? };
                index += 3;
                Some(counts)
            } else { None };
            summaries.push(FieldSummary {
                name: name.clone(),
                data_type: types[name].clone(),
                non_null_count,
                null_count: rows - non_null_count,
                sum,
                amount_counts,
            });
        }
        Ok(ResultSummary {
            batch: result.run_dir.clone(),
            node_id: node.into(),
            category: category.into(),
            rows,
            fields: summaries,
        })
    })
    .map_err(|e| e.to_string())
}

pub fn inspect_group(
    result: &RunResult,
    node: &str,
    group_values: &std::collections::BTreeMap<String, Option<String>>,
    limit: usize,
) -> Result<Preview, String> {
    let _guard = engine::result_query_guard()?;
    resolve(result, node, "normal")?;
    // The batch's saved rules, not the current editor draft, define membership.
    let bytes =
        fs::read(Path::new(&result.run_dir).join("workflow.json")).map_err(|e| e.to_string())?;
    let wf: Workflow = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let aggregate = wf
        .nodes
        .iter()
        .find(|n| n.id == node && n.kind == "aggregate")
        .ok_or("请选择汇总步骤")?;
    let keys = crate::table_ops::aggregate_keys(&aggregate.config)?;
    if keys.iter().collect::<BTreeSet<_>>() != group_values.keys().collect::<BTreeSet<_>>() {
        return Err("分组字段与本批次规则不一致".into());
    }
    let parent = wf
        .edges
        .iter()
        .find(|e| e.target == node)
        .ok_or("汇总缺上游")?;
    let table = resolve(result, &parent.source, "normal")?;
    let conn = Connection::open(&result.database).map_err(|e| e.to_string())?;
    let types = engine::column_types(&conn, &table)?;
    let mut conditions = Vec::new();
    for key in keys {
        let ty = types.get(&key).ok_or("分组字段不存在")?;
        // Type names originate exclusively from DuckDB metadata.
        conditions.push(match &group_values[&key] {
            None => format!("{} IS NULL", sql_name(&key)),
            Some(v) => format!(
                "{} IS NOT DISTINCT FROM CAST({} AS {ty})",
                sql_name(&key),
                engine::lit(v)
            ),
        });
    }
    engine::preview_query(&conn, &table, &conditions.join(" AND "), limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_includes_rows_outside_preview_and_preserves_empty_sum() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/b-batch-acceptance/summary");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("query.duckdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE OR REPLACE TABLE n_test AS SELECT CAST(CASE WHEN i=200 THEN 1000 ELSE 1 END AS DECIMAL(28,4)) AS 金额 FROM range(201) t(i)").unwrap();
        drop(conn);
        let result = RunResult {
            run_dir: root.to_string_lossy().into(),
            database: path.to_string_lossy().into(),
            reviews: vec![NodeReview {
                node_id: "test".into(),
                label: "test".into(),
                rows: 201,
                amount_total: None,
                exceptions: 0,
                excluded: 0,
                table: "n_test".into(),
                matched: None,
                left_only: None,
                right_only: None,
                unresolved: None,
            }],
            input_fingerprints: Default::default(),
        };
        let s = summarize(&result, "test", "normal", &["金额".into()]).unwrap();
        assert_eq!(s.rows, 201);
        assert_eq!(s.fields[0].sum.as_deref(), Some("1200.0000"));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("DELETE FROM n_test").unwrap();
        }
        assert_eq!(
            summarize(&result, "test", "normal", &["金额".into()])
                .unwrap()
                .fields[0]
                .sum,
            None
        );
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("INSERT INTO n_test VALUES(NULL)")
                .unwrap();
        }
        let s = summarize(&result, "test", "normal", &["金额".into()]).unwrap();
        assert_eq!(s.fields[0].null_count, 1);
        assert_eq!(s.fields[0].sum, None);
        assert!(summarize(&result, "test", "normal", &["不存在".into()]).is_err());
        assert!(summarize(&result, "test", "excluded", &["金额".into()]).is_err());
    }
}
