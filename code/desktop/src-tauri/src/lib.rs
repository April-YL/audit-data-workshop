mod engine;
mod field_contract;
mod result_queries;
mod result_registry;
mod join_append;
mod pivot;
mod formula;
mod table_ops;
pub mod model;
mod source_structure;
mod workflow;

use model::{
    CompanyConfig, Preview, ProjectFile, RunResult, SourceBinding, SourcePreview, TracePart,
    Workflow,
};
use std::collections::{BTreeMap, BTreeSet};
use std::{fs, path::Path};
use tauri::{Emitter, Manager};

fn project_company(project: &ProjectFile) -> Result<CompanyConfig, String> {
    if project.schema_version != 1 {
        return Err(format!("不支持的项目版本：{}", project.schema_version));
    }
    let mut sources = BTreeMap::new();
    for node in &project.workflow.nodes {
        if node.kind != "source" {
            continue;
        }
        let key = workflow::required_str(&node.config, "source_key")?;
        let binding = project
            .bindings
            .get(&node.id)
            .ok_or_else(|| format!("文件输入节点 {} 尚未选择资料", node.label))?;
        if binding.path.trim().is_empty() {
            return Err(format!("文件输入节点 {} 尚未选择资料", node.label));
        }
        if !Path::new(&binding.path).is_file() {
            return Err(format!(
                "文件输入节点 {} 的资料不存在：{}",
                node.label, binding.path
            ));
        }
        if let Some(previous) = sources.insert(key.to_string(), binding.clone()) {
            if previous.path != binding.path || previous.sheet != binding.sheet {
                return Err(format!("资料代号 {key} 被不同输入节点重复使用"));
            }
        }
    }
    Ok(CompanyConfig {
        schema_version: 1,
        company: project
            .company_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "本次项目".into()),
        sources,
    })
}

fn project_output_root(project: &ProjectFile) -> Result<String, String> {
    let outputs: Vec<&model::Node> = project
        .workflow
        .nodes
        .iter()
        .filter(|node| node.kind == "export")
        .collect();
    if outputs.len() != 1 {
        return Err("当前项目须有且仅有一个导出节点".into());
    }
    let root = workflow::required_str(&outputs[0].config, "output_root")?;
    if !Path::new(root).is_absolute() {
        return Err("请在导出节点选择本地输出文件夹".into());
    }
    if let Some(name) = outputs[0]
        .config
        .get("file_name")
        .and_then(serde_json::Value::as_str)
    {
        if name.is_empty()
            || Path::new(name).file_name().and_then(|part| part.to_str()) != Some(name)
            || !name.to_ascii_lowercase().ends_with(".csv")
        {
            return Err("导出节点的结果文件名须为不含路径的 CSV 文件名".into());
        }
    }
    Ok(root.to_string())
}

fn validate_project_inputs(project: &ProjectFile) -> Result<Vec<String>, String> {
    let company = project_company(project)?;
    let order = workflow::ordered_nodes(&project.workflow, &company)?;
    let contracts = infer_contracts(project)?;
    let mut schemas: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &order {
        if contracts.get(&node.id).is_some_and(|fields|fields.iter().any(|field|!field.confirmed)) {
            // Dynamic pivot columns are checked against materialized input metadata
            // during execution, before any exports are published.
            continue;
        }
        if node.kind == "source" {
            let binding = project.bindings.get(&node.id).ok_or("缺少文件绑定")?;
            let sample = (if node
                .config
                .get("normalize_structure")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                engine::inspect_normalized_source(binding)
            } else {
                engine::inspect_source(binding, 1)
            })
            .map_err(|error| format!("节点 {}：{error}", node.label))?;
            let mut columns: BTreeSet<String> = sample.headers.into_iter().collect();
            columns.extend(
                [
                    "__company",
                    "__file",
                    "__sheet",
                    "__row",
                    "__record",
                    "__role",
                ]
                .map(str::to_string),
            );
            schemas.insert(node.id.clone(), columns);
            continue;
        }
        let upstream = project
            .workflow
            .edges
            .iter()
            .find(|edge| edge.target == node.id);
        let mut columns = upstream.and_then(|edge| schemas.get(&edge.source)).cloned();
        if node.kind == "map" {
            if let Some(mappings) = node.config.get("mappings") {
                let mappings = mappings
                    .as_array()
                    .ok_or_else(|| format!("节点 {}：字段映射格式无效", node.label))?;
                if mappings.is_empty() {
                    return Err(format!("节点 {}：请至少保留一个字段", node.label));
                }
                let available = columns
                    .as_ref()
                    .ok_or_else(|| format!("节点 {}：无法取得上一步字段", node.label))?;
                let mut outputs: BTreeSet<String> = [
                    "company", "__file", "__sheet", "__row", "__record", "__role",
                ]
                .into_iter()
                .map(str::to_string)
                .collect();
                for mapping in mappings {
                    let source = workflow::required_str(mapping, "source")?;
                    let target = workflow::required_str(mapping, "target")?;
                    if !available.contains(source) {
                        return Err(format!("节点 {}：上一步缺少字段 {source}", node.label));
                    }
                    if target.chars().count() > 100
                        || target.starts_with("__")
                        || !outputs.insert(target.to_string())
                    {
                        return Err(format!(
                            "节点 {}：输出字段重名或使用保留名称 {target}",
                            node.label
                        ));
                    }
                }
                columns = Some(outputs);
            } else {
                columns = None;
            }
        }
        if node.kind == "filter" {
            if let Some(predicate) = node.config.get("condition") {
                let field = workflow::required_str(predicate, "field")?;
                if let Some(available) = columns.as_ref() {
                    if !available.contains(field) {
                        return Err(format!("节点 {}：上一步缺少筛选字段 {field}", node.label));
                    }
                }
                let operator = workflow::required_str(predicate, "operator")?;
                if ![
                    "numeric_gt", "numeric_gte", "numeric_lt", "numeric_lte", "numeric_eq", "numeric_neq",
                    "date_gt","date_gte","date_lt","date_lte","date_eq","date_neq",
                    "equals",
                    "not_equals",
                    "contains",
                    "is_empty",
                    "is_not_empty",
                ]
                .contains(&operator)
                {
                    return Err(format!("节点 {}：不支持的筛选条件 {operator}", node.label));
                }
                if !["is_empty", "is_not_empty"].contains(&operator) {
                    workflow::required_str(predicate, "value")?;
                }
            }
        }
        if let Some(fields) = contracts.get(&node.id) { columns=Some(fields.iter().map(|f|f.name.clone()).collect()); }
        if let Some(columns) = columns {
            schemas.insert(node.id.clone(), columns);
        }
    }
    Ok(order.into_iter().map(|node| node.label).collect())
}

#[tauri::command]
fn validate_project(project: ProjectFile) -> Result<Vec<String>, String> {
    validate_project_inputs(&project)
}

fn infer_contracts(project: &ProjectFile) -> Result<BTreeMap<String, Vec<model::FieldDescriptor>>, String> {
    let company = project_company(project)?;
    let order = workflow::ordered_nodes(&project.workflow, &company)?;
    let mut schemas = BTreeMap::<String, Vec<model::FieldDescriptor>>::new();
    for node in order {
        let fields = if node.kind == "source" {
            let binding = project.bindings.get(&node.id).ok_or("缺少绑定")?;
            let sample = if node.config.get("normalize_structure").and_then(serde_json::Value::as_bool) == Some(true) {
                engine::inspect_normalized_source(binding)?
            } else {
                engine::inspect_source(binding, 1)?
            };
            let mut fields=sample.headers.into_iter().map(|name| model::FieldDescriptor {
                name, data_type: "VARCHAR".into(), nullable: true, origin: node.id.clone(), confirmed: true,
            }).collect::<Vec<_>>();
            for (name,data_type) in [("__company","VARCHAR"),("__file","VARCHAR"),("__sheet","VARCHAR"),("__row","BIGINT"),("__record","BIGINT"),("__role","VARCHAR")] {
                fields.push(model::FieldDescriptor{name:name.into(),data_type:data_type.into(),nullable:false,origin:node.id.clone(),confirmed:true});
            }
            fields
        } else {
            let input_ids = engine::input_ids(&project.workflow, &node)?;
            let inputs = input_ids.iter().map(|id| schemas.get(*id).map(Vec::as_slice)
                .ok_or_else(|| format!("节点 {} 的上游 {} 字段未知", node.label, id)))
                .collect::<Result<Vec<_>, _>>()?;
            if inputs.iter().any(|input|input.iter().any(|field|!field.confirmed)) {
                vec![model::FieldDescriptor{name:"__dynamic_schema".into(),data_type:"UNKNOWN".into(),nullable:true,origin:node.id.clone(),confirmed:false}]
            } else {
                field_contract::check_inputs(&node, &inputs, &company)
                    .map_err(|error| format!("节点 {}：{error}", node.label))?
            }
        };
        schemas.insert(node.id, fields);
    }
    Ok(schemas)
}

#[tauri::command]
fn infer_outputs(project:ProjectFile,node_id:String)->model::OutputContract{
 let result=project_through(&project,&node_id).and_then(|p|infer_contracts(&p)).and_then(|mut s|s.remove(&node_id).ok_or("节点不存在".into())).and_then(|fields|if fields.iter().any(|field|!field.confirmed){Err("字段随透视资料变化，请先计算对应步骤后查看本次实际字段".into())}else{Ok(fields)});
 let mode=project.workflow.nodes.iter().find(|n|n.id==node_id).map(|n|match n.kind.as_str(){"decimal" if n.version==1=>"partition","decimal"|"text_extract"|"derive"=>"annotation",_=>"none"}).unwrap_or("none");
 match result{Ok(fields)=>model::OutputContract{outputs:BTreeMap::from([("normal".into(),fields.clone()),("exceptions".into(),fields.clone()),("excluded".into(),fields)]),diagnostics:vec![],exception_mode:mode.into()},Err(message)=>model::OutputContract{outputs:BTreeMap::new(),diagnostics:vec![model::NodeDiagnostic{node_id,parameter:"config".into(),code:"invalid_fields".into(),message}],exception_mode:mode.into()}}
}

#[tauri::command]
async fn infer_result_outputs(registry:tauri::State<'_,result_registry::ResultRegistry>,batch:String,node_id:String)->Result<model::OutputContract,String>{
    let result=registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=engine::result_query_guard()?;
        let table=result_queries::resolve(&result,&node_id,"normal")?;
        let conn=duckdb::Connection::open(&result.database).map_err(|error|error.to_string())?;
        let fields=engine::descriptors(&conn,&table,&node_id)?;
        Ok(model::OutputContract{outputs:BTreeMap::from([("normal".into(),fields)]),diagnostics:vec![],exception_mode:"none".into()})
    }).await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn inspect_pivot(
    registry:tauri::State<'_,result_registry::ResultRegistry>,
    batch:String,node_id:String,group_values:BTreeMap<String,Option<String>>,
    column:String,total_row:bool,limit:usize,
)->Result<Preview,String>{
    let result=registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||pivot::members(&result,&node_id,&group_values,&column,total_row,limit))
        .await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn preflight_join(project:ProjectFile,node_id:String)->Result<join_append::JoinCounts,String>{
    tauri::async_runtime::spawn_blocking(move||{
        let mut prefix=project_through(&project,&node_id)?;
        let node=prefix.workflow.nodes.iter().find(|node|node.id==node_id&&node.kind=="join"&&node.version==2).cloned().ok_or("请选择新版关联")?;
        let inputs=engine::input_ids(&prefix.workflow,&node)?.into_iter().map(str::to_owned).collect::<Vec<_>>();
        prefix.workflow.nodes.retain(|node|node.id!=node_id);
        prefix.workflow.edges.retain(|edge|edge.target!=node_id);
        validate_project_inputs(&prefix)?;
        let company=project_company(&prefix)?;
        let root=std::env::current_exe().map_err(|error|error.to_string())?.parent().ok_or("无法确定处理目录")?.join("processing-runs");
        let result=engine::process(&prefix.workflow,&company,&root.to_string_lossy())?;
        let conn=duckdb::Connection::open(&result.database).map_err(|error|error.to_string())?;
        join_append::preflight_node(&conn,&node,&format!("n_{}",inputs[0]),&format!("n_{}",inputs[1]))
    }).await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn selected_record_filter(registry:tauri::State<'_,result_registry::ResultRegistry>,batch:String,node_id:String,records:Vec<u64>,exclude:bool)->Result<serde_json::Value,String>{
    let result=registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||result_queries::selected_record_filter(&result,&node_id,&records,exclude))
        .await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn distinct_result_values(registry: tauri::State<'_, result_registry::ResultRegistry>, batch: String, node_id: String, field: String, search: String, offset: usize,filter_config:Option<serde_json::Value>) -> Result<Vec<Option<String>>, String> {
    let result = registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move || result_queries::distinct_values(&result, &node_id, &field, &search, offset,filter_config))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn summarize_result(registry: tauri::State<'_, result_registry::ResultRegistry>, batch: String, node_id:String,category:String,fields:Vec<String>)->Result<model::ResultSummary,String>{
    let result = registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||result_queries::summarize(&result,&node_id,&category,&fields)).await.map_err(|e|e.to_string())?
}
#[tauri::command]
async fn inspect_group(registry: tauri::State<'_, result_registry::ResultRegistry>, batch: String, node_id:String,group_values:BTreeMap<String,Option<String>>,limit:usize)->Result<Preview,String>{
    let result = registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||result_queries::inspect_group(&result,&node_id,&group_values,limit)).await.map_err(|e|e.to_string())?
}
#[tauri::command]
async fn validate_formula(registry:tauri::State<'_,result_registry::ResultRegistry>,project:ProjectFile,node_id:String,expression:String,batch:Option<String>)->Result<model::FormulaCheck,String>{
    let result=batch.map(|batch|registry.resolve(&batch)).transpose()?;
    tauri::async_runtime::spawn_blocking(move||{
        let fields=if let Some(result)=result{
            let _guard=engine::result_query_guard()?;
            let table=result_queries::resolve(&result,&node_id,"normal")?;
            let conn=duckdb::Connection::open(&result.database).map_err(|error|error.to_string())?;
            engine::descriptors(&conn,&table,&node_id)?
        }else{
            infer_contracts(&project_through(&project,&node_id)?)?.remove(&node_id).ok_or("字段结构未知")?
        };
        let compiled=formula::compile(&expression,&fields)?;
        Ok(model::FormulaCheck{result_type:compiled.data_type().into(),referenced_fields:compiled.referenced_fields,diagnostics:vec![]})
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
async fn preview_formula(registry: tauri::State<'_, result_registry::ResultRegistry>, batch: String, node_id:String,expression:String,limit:usize)->Result<Preview,String>{
    let result = registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=engine::result_query_guard()?;
        let name=result_queries::resolve(&result,&node_id,"normal")?;
        let conn=duckdb::Connection::open(&result.database).map_err(|e|e.to_string())?;
        let fields=engine::descriptors(&conn,&name,&node_id)?;let compiled=formula::compile_for_table(&expression,&fields,&conn,&workflow::sql_name(&name))?;
        let view="__formula_trial";
        conn.execute_batch(&format!("CREATE TEMP VIEW {view} AS SELECT *,({}) AS \"公式试算值\",({}) AS \"公式试算错误\" FROM {}",compiled.sql,compiled.error,workflow::sql_name(&name))).map_err(|e|e.to_string())?;
        engine::preview_query(&conn,view,"TRUE",limit)
    }).await.map_err(|e|e.to_string())?
}

fn validate_project_data(project: &ProjectFile) -> Result<Vec<String>, String> {
    let order = validate_project_inputs(project)?;
    project_output_root(project)?;
    Ok(order)
}

fn project_through(project: &ProjectFile, node_id: &str) -> Result<ProjectFile, String> {
    if !project.workflow.nodes.iter().any(|n| n.id == node_id) {
        return Err("要计算的步骤不存在".into());
    }
    let mut needed = BTreeSet::new();
    let mut pending = vec![node_id.to_string()];
    while let Some(id) = pending.pop() {
        if !needed.insert(id.clone()) {
            continue;
        }
        for edge in project.workflow.edges.iter().filter(|e| e.target == id) {
            pending.push(edge.source.clone());
        }
    }
    let mut prefix = project.clone();
    prefix.workflow.nodes.retain(|n| needed.contains(&n.id));
    prefix
        .workflow
        .edges
        .retain(|e| needed.contains(&e.source) && needed.contains(&e.target));
    prefix.bindings.retain(|id, _| needed.contains(id));
    Ok(prefix)
}

fn process_project_data(
    project: ProjectFile,
    node_id: &str,
    work_root: &str,
) -> Result<RunResult, String> {
    let prefix = project_through(&project, node_id)?;
    validate_project_inputs(&prefix)?;
    let company = project_company(&prefix)?;
    engine::process(&prefix.workflow, &company, work_root)
}

#[tauri::command]
async fn process_project(registry: tauri::State<'_, result_registry::ResultRegistry>, project: ProjectFile, node_id: String, revision: u64) -> Result<RunResult, String> {
    let ticket = registry.begin(revision)?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let work_root = executable
        .parent()
        .ok_or("无法确定本地处理目录")?
        .join("processing-runs");
    let result = tauri::async_runtime::spawn_blocking(move || {
        process_project_data(project, &node_id, &work_root.to_string_lossy())
    })
    .await
    .map_err(|e| e.to_string())??;
    registry.publish(ticket, result)
}

#[cfg(test)]
fn export_node_data(
    result: RunResult,
    node_id: &str,
    category: &str,
    path: &str,
) -> Result<model::ExportReceipt, String> {
    let table = result_queries::resolve(&result,node_id,category)?;
    let target = Path::new(path);
    ensure_export_path(&result,target)?;
    let rows = engine::export_csv_file(&result.database, &table, target)?;
    Ok(model::ExportReceipt {
        path: path.into(), rows, node_id: node_id.into(), category: category.into(),
    })
}

fn ensure_export_path(result:&RunResult,target:&Path)->Result<(),String>{
    let absolute = |p: &Path| -> Result<String, String> {
        let normalized = if p.exists() {
            p.canonicalize().map_err(|e| e.to_string())?
        } else {
            p.parent()
                .ok_or("保存路径无效")?
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(p.file_name().ok_or("文件名无效")?)
        };
        Ok(normalized.to_string_lossy().to_lowercase())
    };
    let identity = absolute(target)?;
    let company: CompanyConfig = serde_json::from_slice(
        &fs::read(Path::new(&result.run_dir).join("company.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    for source in company.sources.values() {
        if absolute(Path::new(&source.path))? == identity {
            return Err("不能覆盖本次处理的原始输入文件，请另选保存位置".into());
        }
    }
    Ok(())
}

#[tauri::command]
async fn preflight_export(
    registry:tauri::State<'_,result_registry::ResultRegistry>,
    batch:String,node_id:String,category:String,scope:result_queries::ExportScope,
)->Result<result_queries::ExportPlan,String>{
    let result=registry.resolve(&batch)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=engine::result_query_guard()?;
        let conn=duckdb::Connection::open(&result.database).map_err(|error|error.to_string())?;
        result_queries::export_query(&conn,&result,&node_id,&category,&scope).map(|(_,plan)|plan)
    }).await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn export_scope(
    registry:tauri::State<'_,result_registry::ResultRegistry>,
    batch:String,node_id:String,category:String,scope:result_queries::ExportScope,token:String,path:String,
)->Result<model::ExportReceipt,String>{
    let registry=registry.inner().clone();
    tauri::async_runtime::spawn_blocking(move||
        result_queries::export_scoped(&registry,&batch,&node_id,&category,&scope,&token,Path::new(&path))
    ).await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn export_node(
    registry: tauri::State<'_, result_registry::ResultRegistry>,
    batch: String,
    node_id: String,
    category: String,
    path: String,
) -> Result<model::ExportReceipt, String> {
    let registry=registry.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result=registry.resolve(&batch)?;
        let plan_and_scope={
            let _guard=engine::result_query_guard()?;
            let table=result_queries::resolve(&result,&node_id,&category)?;
            let conn=duckdb::Connection::open(&result.database).map_err(|error|error.to_string())?;
            let scope=result_queries::ExportScope{columns:engine::columns(&conn,&table)?,condition:None,records:None};
            let (_,plan)=result_queries::export_query(&conn,&result,&node_id,&category,&scope)?;
            (plan,scope)
        };
        result_queries::export_scoped(&registry,&batch,&node_id,&category,&plan_and_scope.1,&plan_and_scope.0.token,Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn confirm_project_close(app: tauri::AppHandle) {
    app.exit(0);
}

fn run_project_data(project: ProjectFile) -> Result<RunResult, String> {
    validate_project_data(&project)?;
    let company = project_company(&project)?;
    let output_root = project_output_root(&project)?;
    engine::run(&project.workflow, &company, &output_root)
}

#[tauri::command]
async fn run_project(registry: tauri::State<'_, result_registry::ResultRegistry>, project: ProjectFile, revision: u64) -> Result<RunResult, String> {
    let ticket = registry.begin(revision)?;
    let result = tauri::async_runtime::spawn_blocking(move || run_project_data(project))
        .await.map_err(|error| error.to_string())??;
    registry.publish(ticket, result)
}

#[tauri::command]
fn invalidate_result(registry: tauri::State<'_, result_registry::ResultRegistry>, revision: u64) -> Result<(), String> {
    registry.invalidate(revision)
}

#[tauri::command]
fn save_project(path: String, project: ProjectFile) -> Result<(), String> {
    save_project_file(&path, &project)
}

fn save_project_file(path: &str, project: &ProjectFile) -> Result<(), String> {
    if project.schema_version != 1 {
        return Err("不支持的项目版本".into());
    }
    let target = Path::new(&path);
    if target.extension().and_then(|part| part.to_str()) != Some("json") {
        return Err("项目须保存为 JSON 文件".into());
    }
    let data = serde_json::to_vec_pretty(&project).map_err(|e| e.to_string())?;
    let parent = target.parent().ok_or("项目路径无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    engine::atomic_replace(
        target,
        |temporary| fs::write(temporary, &data).map_err(|e| e.to_string()),
        true,
    )
}

#[tauri::command]
fn load_project(path: String) -> Result<ProjectFile, String> {
    let project: ProjectFile = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if project.schema_version != 1 {
        return Err(format!("不支持的项目版本：{}", project.schema_version));
    }
    Ok(project)
}

#[tauri::command]
fn import_legacy_project(path: String) -> Result<ProjectFile, String> {
    let (workflow, company) = load_template(path)?;
    let mut bindings = BTreeMap::new();
    for node in &workflow.nodes {
        if node.kind == "source" {
            let key = workflow::required_str(&node.config, "source_key")?;
            if let Some(binding) = company.sources.get(key) {
                bindings.insert(node.id.clone(), binding.clone());
            }
        }
    }
    Ok(ProjectFile {
        schema_version: 1,
        workflow,
        company_name: Some(company.company),
        bindings,
    })
}
#[cfg(debug_assertions)]
use tauri::Manager;

#[tauri::command]
fn validate_workflow(workflow: Workflow, company: CompanyConfig) -> Result<Vec<String>, String> {
    Ok(workflow::ordered_nodes(&workflow, &company)?
        .into_iter()
        .map(|n| n.label)
        .collect())
}

pub fn cli_validate(workflow: Workflow, company: CompanyConfig) -> Result<Vec<String>, String> {
    validate_workflow(workflow, company)
}

#[tauri::command]
fn save_workflow(path: String, workflow: Workflow, company: CompanyConfig) -> Result<(), String> {
    save_template(path, workflow, company)
}

pub fn save_template(
    path: String,
    workflow: Workflow,
    company: CompanyConfig,
) -> Result<(), String> {
    workflow::ordered_nodes(&workflow, &company)?;
    let folder = Path::new(&path);
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let workflow_data = serde_json::to_vec_pretty(&workflow).map_err(|e| e.to_string())?;
    let company_data = serde_json::to_vec_pretty(&company).map_err(|e| e.to_string())?;
    fs::write(folder.join("workflow.json"), workflow_data).map_err(|e| e.to_string())?;
    fs::write(folder.join("company.json"), company_data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_workflow(path: String) -> Result<(Workflow, CompanyConfig), String> {
    load_template(path)
}

pub fn load_template(path: String) -> Result<(Workflow, CompanyConfig), String> {
    let folder = Path::new(&path);
    let wf: Workflow =
        serde_json::from_slice(&fs::read(folder.join("workflow.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let cfg: CompanyConfig =
        serde_json::from_slice(&fs::read(folder.join("company.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    workflow::ordered_nodes(&wf, &cfg)?;
    Ok((wf, cfg))
}

fn validate_template(workflow: &Workflow) -> Result<(), String> {
    let mut sources = BTreeMap::new();
    for node in &workflow.nodes {
        if node.kind == "source" {
            let key = workflow::required_str(&node.config, "source_key")?;
            sources.insert(
                key.to_string(),
                SourceBinding {
                    reading_baseline: None,
                    path: String::new(),
                    sheet: None,
                    fields: BTreeMap::new(),
                    layout: None,
                    text: None,
                },
            );
        }
    }
    let placeholder = CompanyConfig {
        schema_version: 1,
        company: String::new(),
        sources,
    };
    workflow::ordered_nodes(workflow, &placeholder)?;
    Ok(())
}

#[tauri::command]
fn save_workflow_template(path: String, workflow: Workflow) -> Result<(), String> {
    save_workflow_file(path, workflow)
}

pub fn save_workflow_file(path: String, workflow: Workflow) -> Result<(), String> {
    validate_template(&workflow)?;
    if path.trim().is_empty() {
        return Err("请先选择工作流模板文件夹".into());
    }
    let folder = Path::new(&path);
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    fs::write(
        folder.join("workflow.json"),
        serde_json::to_vec_pretty(&workflow).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_workflow_template(path: String) -> Result<Workflow, String> {
    load_workflow_file(path)
}

pub fn load_workflow_file(path: String) -> Result<Workflow, String> {
    let wf: Workflow = serde_json::from_slice(
        &fs::read(Path::new(&path).join("workflow.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    validate_template(&wf)?;
    Ok(wf)
}

#[tauri::command]
fn save_company_config(path: String, company: CompanyConfig) -> Result<(), String> {
    save_company_file(path, company)
}

pub fn save_company_file(path: String, company: CompanyConfig) -> Result<(), String> {
    if company.schema_version != 1 || company.company.trim().is_empty() {
        return Err("公司配置版本或公司名称无效".into());
    }
    if path.trim().is_empty() {
        return Err("请先选择公司配置文件".into());
    }
    let file = Path::new(&path);
    if let Some(parent) = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(
        file,
        serde_json::to_vec_pretty(&company).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_company_config(path: String) -> Result<CompanyConfig, String> {
    load_company_file(path)
}

pub fn load_company_file(path: String) -> Result<CompanyConfig, String> {
    let cfg: CompanyConfig = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if cfg.schema_version != 1 || cfg.company.trim().is_empty() {
        return Err("公司配置版本或公司名称无效".into());
    }
    Ok(cfg)
}

#[tauri::command]
async fn run_workflow(
    workflow: Workflow,
    company: CompanyConfig,
    output_root: String,
) -> Result<RunResult, String> {
    tauri::async_runtime::spawn_blocking(move || engine::run(&workflow, &company, &output_root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn preview_table(registry: tauri::State<'_, result_registry::ResultRegistry>, batch: String, node_id: String, category: String, limit: usize, offset: Option<usize>) -> Result<Preview, String> {
    let result = registry.resolve(&batch)?;
    let table = result_queries::resolve(&result, &node_id, &category)?;
    tauri::async_runtime::spawn_blocking(move || engine::preview_page(&result.database, &table, offset.unwrap_or(0), limit))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn inspect_source_file(
    binding: SourceBinding,
    limit: usize,
) -> Result<SourcePreview, String> {
    tauri::async_runtime::spawn_blocking(move || engine::inspect_source(&binding, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn inspect_raw_source_file(
    binding: SourceBinding,
    limit: usize,
) -> Result<crate::model::RawSourcePreview, String> {
    tauri::async_runtime::spawn_blocking(move || engine::inspect_raw_source(&binding, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn inspect_source_structure(
    binding: SourceBinding,
) -> Result<model::StructureReport, String> {
    tauri::async_runtime::spawn_blocking(move || source_structure::inspect(&binding))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_source_sheets(path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || engine::source_sheets(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn check_source_reading(
    binding: SourceBinding,
    full: bool,
) -> Result<model::ReadingCheck, String> {
    tauri::async_runtime::spawn_blocking(move || source_structure::check_reading(&binding, full))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn inspect_raw_source_window(
    binding: SourceBinding,
    start_row: usize,
    start_col: u32,
    tail:Option<bool>,
) -> Result<model::RawSourcePreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        engine::inspect_raw_slice(&binding, start_row, start_col,tail.unwrap_or(false))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn trace_join(
    database: String,
    workflow: Workflow,
    join_id: String,
    group: BTreeMap<String, String>,
) -> Result<Vec<TracePart>, String> {
    engine::trace_join(&database, &workflow, &join_id, &group)
}

pub fn run_app() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(debug_assertions)]
    let builder = builder.setup(|app| {
        if let Some(window) = app.get_webview_window("main") {
            window.set_title("审计数工坊（测试版）")?;
        }
        Ok(())
    });
    builder
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                webview.state::<result_registry::ResultRegistry>().reset();
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("project-close-requested", ());
            }
        })
        .manage(result_registry::ResultRegistry::default())
        .invoke_handler(tauri::generate_handler![
            invalidate_result,
            distinct_result_values,
            validate_project,
            infer_outputs,
            infer_result_outputs,
            inspect_pivot,
            preflight_join,
            selected_record_filter,
            summarize_result,
            validate_formula,
            preview_formula,
            inspect_group,
            run_project,
            process_project,
            export_node,
            preflight_export,
            export_scope,
            confirm_project_close,
            save_project,
            load_project,
            import_legacy_project,
            list_source_sheets,
            validate_workflow,
            save_workflow,
            load_workflow,
            save_workflow_template,
            load_workflow_template,
            save_company_config,
            load_company_config,
            run_workflow,
            inspect_source_file,
            inspect_raw_source_file,
            inspect_source_structure,
            check_source_reading,
            inspect_raw_source_window,
            preview_table,
            trace_join
        ])
        .run(tauri::generate_context!())
        .expect("无法启动审计数工坊");
}

pub fn run_cli(
    workflow: Workflow,
    company: CompanyConfig,
    output_root: &str,
) -> Result<RunResult, String> {
    engine::run(&workflow, &company, output_root)
}
pub fn cli_preview(db_path: &str, table: &str, limit: usize) -> Result<Preview, String> {
    engine::preview(db_path, table, limit)
}
pub fn cli_inspect_source(binding: &SourceBinding, limit: usize) -> Result<SourcePreview, String> {
    engine::inspect_source(binding, limit)
}
pub fn cli_trace(
    db_path: &str,
    wf: &Workflow,
    join_id: &str,
    group: &BTreeMap<String, String>,
) -> Result<Vec<TracePart>, String> {
    engine::trace_join(db_path, wf, join_id, group)
}

#[cfg(test)]
mod project_tests {
    #[test]
    fn scoped_export_preserves_cross_page_records_and_rejects_stale_publication() {
        let topic=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let data=topic.join("data/step1-foundation/export-records.csv");
        let mut content="编号,组\n".to_owned();
        for index in 1..=201 {content.push_str(&format!("{index:03},{}\n",if index%2==1{"A"}else{"B"}));}
        fs::write(&data,content).unwrap();
        let root=topic.join("outputs/workbench-rebuild-acceptance/export");
        fs::create_dir_all(&root).unwrap();
        let project:ProjectFile=serde_json::from_value(json!({
            "schema_version":1,"workflow":{"schema_version":1,"name":"导出边界","nodes":[
                {"id":"input","kind":"source","label":"输入","position":{"x":0,"y":0},"config":{"source_key":"input"}}
            ],"edges":[]},"bindings":{"input":{"path":data,"fields":{}}}
        })).unwrap();
        let result=process_project_data(project.clone(),"input",&root.to_string_lossy()).unwrap();
        let values=result_queries::distinct_values(&result,"input","编号","",0,Some(json!({"match":"all","conditions":[
            {"field":"编号","operator":"equals","value":"001"},{"field":"组","operator":"equals","value":"A"}
        ]}))).unwrap();
        assert_eq!(values.len(),101);
        assert_eq!(values.last(),Some(&Some("201".into())));
        let record_config=result_queries::selected_record_filter(&result,"input",&[1,201],false).unwrap();
        let mut selected_project=project;
        selected_project.workflow.nodes.push(serde_json::from_value(json!({
            "id":"selected","kind":"filter","label":"选中来源","position":{"x":200,"y":0},"config":record_config
        })).unwrap());
        selected_project.workflow.edges.push(Edge{id:"selected_edge".into(),source:"input".into(),target:"selected".into(),source_port:None,target_port:None});
        let filtered=process_project_data(selected_project.clone(),"selected",&root.to_string_lossy()).unwrap();
        assert_eq!(filtered.reviews.last().unwrap().rows,2);
        let conn=duckdb::Connection::open(&result.database).unwrap();
        let scope=result_queries::ExportScope{columns:vec!["编号".into()],condition:None,records:Some(vec![201,1])};
        let (_,plan)=result_queries::export_query(&conn,&result,"input","normal",&scope).unwrap();
        assert_eq!(plan.rows,2);
        drop(conn);
        let registry=result_registry::ResultRegistry::default();
        let ticket=registry.begin(1).unwrap();
        registry.publish(ticket,result.clone()).unwrap();
        let target=root.join("selected.csv");
        result_queries::export_scoped(&registry,&result.run_dir,"input","normal",&scope,&plan.token,&target).unwrap();
        let mut reader=csv::Reader::from_path(&target).unwrap();
        let rows=reader.records().map(|row|row.unwrap()[0].to_owned()).collect::<Vec<_>>();
        assert_eq!(rows,["001","201"]);
        let before=fs::read(&target).unwrap();
        let error=engine::atomic_replace_checked(&target,|temp|{
            fs::write(temp,"incomplete").unwrap();
            registry.invalidate(2)?;
            Ok(())
        },false,|temp,path|registry.with_current(&result.run_dir,||fs::rename(temp,path).map_err(|error|error.to_string())));
        assert!(error.is_err());
        assert_eq!(fs::read(&target).unwrap(),before);
        assert!(result_queries::export_scoped(&registry,&result.run_dir,"input","normal",&scope,&plan.token,&target).is_err());
        let copied=data.with_file_name("export-records-copy.csv");
        fs::copy(&data,&copied).unwrap();
        let mut rebound=selected_project.clone();
        rebound.bindings.get_mut("input").unwrap().path=copied.to_string_lossy().into();
        let rebound_error=process_project_data(rebound,"selected",&root.to_string_lossy()).unwrap_err();
        assert!(rebound_error.contains("选中来源记录"));
        let original=fs::read(&data).unwrap();
        fs::write(&data,"编号,组\n999,A\n").unwrap();
        let changed=process_project_data(selected_project,"selected",&root.to_string_lossy()).unwrap_err();
        fs::write(&data,original).unwrap();
        assert!(changed.contains("来源文件已变化"));
    }
    #[test]
    fn pivot_runs_through_project_entry_and_traces_total_members() {
        let topic=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let data=topic.join("data/step1-foundation/pivot-members.csv");
        fs::write(&data,"部门,月份,人员\nA,01,x\nA,02,x\nB,01,y\n").unwrap();
        let root=topic.join("outputs/workbench-rebuild-acceptance/pivot");
        fs::create_dir_all(&root).unwrap();
        let project:ProjectFile=serde_json::from_value(json!({
            "schema_version":1,
            "workflow":{"schema_version":1,"name":"透视成员验证","nodes":[
                {"id":"input","kind":"source","label":"输入","position":{"x":0,"y":0},"config":{"source_key":"input"}},
                {"id":"pivot","kind":"pivot","label":"透视","position":{"x":200,"y":0},"config":{
                    "row_by":["部门"],"column_by":["月份"],"metrics":[{"op":"distinct","field":"人员","target":"人数"}]
                }}
            ],"edges":[{"id":"e","source":"input","target":"pivot"}]},
            "bindings":{"input":{"path":data,"fields":{}}}
        })).unwrap();
        let result=process_project_data(project,"pivot",&root.to_string_lossy()).unwrap();
        let conn=duckdb::Connection::open(&result.database).unwrap();
        let specification:String=conn.query_row(
            "SELECT specification FROM n_pivot__pivot_columns WHERE specification LIKE '%\"total\":true%'",
            [],|row|row.get(0),
        ).unwrap();
        let column:pivot::Column=serde_json::from_str(&specification).unwrap();
        drop(conn);
        let group=BTreeMap::from([("部门".to_owned(),None)]);
        let members=pivot::members(&result,"pivot",&group,&column.name,true,200).unwrap();
        assert_eq!(members.total_rows,3);
        let group=BTreeMap::from([("部门".to_owned(),Some("A".into()))]);
        let members=pivot::members(&result,"pivot",&group,&column.name,false,200).unwrap();
        assert_eq!(members.total_rows,2);
    }
    #[test]
    fn b_batch_legacy_golden_engine_still_matches_both_companies() {
        let topic=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut wf:Workflow=serde_json::from_slice(&fs::read(topic.join("tests/golden/workflow.json")).unwrap()).unwrap();
        for n in &mut wf.nodes {if n.kind=="export"{n.config["file_name"]=json!("output.csv");}}
        for (name,count,total) in [("甲",3,"38.0000"),("乙",2,"0.0000")] {
            let company:CompanyConfig=serde_json::from_slice(&fs::read(topic.join(format!("tests/golden/{name}_company.json"))).unwrap()).unwrap();
            let result=engine::run(&wf,&company,&topic.join("outputs/b-batch-acceptance/legacy").to_string_lossy()).unwrap();
            let last=result.reviews.last().unwrap();assert_eq!(last.rows,count);assert_eq!(last.amount_total.as_deref(),Some(total));
            if name=="甲"{assert_eq!(result.reviews.iter().find(|r|r.node_id=="je_decimal").unwrap().exceptions,1);}
        }
    }
    #[test]
    fn b_batch_full_pipeline_summary_formula_order_filter_and_group_trace() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../outputs/b-batch-acceptance/pipeline");fs::create_dir_all(&root).unwrap();
        let data=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/b-batch-acceptance");fs::create_dir_all(&data).unwrap();
        let file=data.join("boundary-201.csv");let mut csv="公司,编码,金额\n".to_string();for i in 0..201{csv.push_str(&format!("{},6602/管理/明细,{}\n",if i%2==0{"A"}else{"B"},if i==200{1000}else{1}));}fs::write(&file,csv).unwrap();
        let configs=[
            ("input","source",json!({"source_key":"input"})),
            ("money","decimal",json!({"fields":[{"source":"金额","target":"金额值"}]})),
            ("formula","derive",json!({"expression":"[金额值]*2","target":"计算结果"})),
            ("position","map",json!({"mappings":[{"source":"计算结果","target":"计算结果"},{"source":"公司","target":"公司"},{"source":"编码","target":"编码"},{"source":"金额值","target":"金额值"}]})),
            ("filtered","filter",json!({"match":"all","conditions":[{"field":"计算结果","operator":"numeric_gt","value":"2"},{"field":"公司","operator":"equals","value":"A"}]})),
            ("sorted","sort",json!({"order":[{"field":"金额值","direction":"desc"}]})),
            ("dups","duplicates",json!({"keys":["公司"],"target":"重复","count_target":"条数","rank_target":"序号"})),
            ("unique","deduplicate",json!({"keys":["公司"],"keep":"first"})),
            ("grouped","aggregate",json!({"group_by":["公司"],"metrics":[{"op":"sum","field":"金额值","target":"合计"},{"op":"rows","target":"记录数"}]})),
            ("groupmap","map",json!({"mappings":[{"source":"合计","target":"汇总金额"},{"source":"公司","target":"公司"}]})),
            ("final","derive",json!({"expression":"[汇总金额]/4","target":"四分之一"}))
        ];
        let mut nodes=configs.iter().map(|(id,k,c)|node(id,k,c.clone())).collect::<Vec<_>>();nodes.iter_mut().find(|n|n.id=="grouped").unwrap().version=2;
        let edges:Vec<Edge>=configs.windows(2).enumerate().map(|(i,p)|Edge{id:format!("e{i}"),source:p[0].0.into(),target:p[1].0.into(),source_port:None,target_port:None}).collect();
        let project:ProjectFile=serde_json::from_value(json!({"schema_version":1,"workflow":{"schema_version":1,"name":"B组合","nodes":nodes,"edges":edges},"bindings":{"input":{"path":file,"fields":{}}}})).unwrap();
        let schemas=infer_contracts(&project).unwrap();assert!(schemas["groupmap"].iter().all(|f|f.name!="编码"));
        let result=process_project_data(project.clone(),"final",&root.to_string_lossy()).unwrap();
        let first=result_queries::summarize(&result,"money","normal",&["金额值".into()]).unwrap();assert_eq!(first.rows,201);assert_eq!(first.fields[0].sum.as_deref(),Some("1200.0000"));
        let kept=result_queries::summarize(&result,"filtered","normal",&["计算结果".into()]).unwrap();assert_eq!(kept.rows,1);assert_eq!(kept.fields[0].sum.as_deref(),Some("2000.0000"));
        assert_eq!(result.reviews.iter().find(|r|r.node_id=="filtered").unwrap().excluded,200);
        let excluded=result_queries::summarize(&result,"filtered","excluded",&["计算结果".into()]).unwrap();assert_eq!(excluded.fields[0].sum.as_deref(),Some("400.0000"));
        let final_sum=result_queries::summarize(&result,"final","normal",&["四分之一".into()]).unwrap();assert_eq!(final_sum.fields[0].sum.as_deref(),Some("250.0000"));
        let members=result_queries::inspect_group(&result,"grouped",&BTreeMap::from([("公司".into(),Some("A".into()))]),100).unwrap();assert_eq!(members.total_rows,1);assert!(members.columns.contains(&"__row".into()));
        let output=root.join("final.csv");export_node_data(result.clone(),"final","normal",&output.to_string_lossy()).unwrap();
        let headers=csv::Reader::from_path(&output).unwrap().headers().unwrap().clone();assert!(headers.iter().position(|c|c=="汇总金额").unwrap()<headers.iter().position(|c|c=="公司").unwrap());
        let saved=root.join("project.json");save_project(saved.to_string_lossy().into(),project.clone()).unwrap();let reopened=load_project(saved.to_string_lossy().into()).unwrap();let again=process_project_data(reopened,"final",&root.to_string_lossy()).unwrap();assert_eq!(result_queries::summarize(&again,"final","normal",&["四分之一".into()]).unwrap().fields[0].sum,final_sum.fields[0].sum);
        fs::write(root.join("verification.json"),serde_json::to_vec_pretty(&json!({"kind":"synthetic-engine","input":201,"input_sum":"1200.0000","filtered":1,"excluded":200,"final_sum":"250.0000","reopen_equal":true})).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "requires approved AUDIT_B_PROJECT and AUDIT_B_ALLOWED_DIR"]
    fn b_batch_small_real_sample_and_independent_values() {
        let source_project:ProjectFile=serde_json::from_slice(&fs::read(std::env::var("AUDIT_B_PROJECT").unwrap()).unwrap()).unwrap();
        let binding=source_project.bindings.values().next().unwrap();let path=Path::new(&binding.path).canonicalize().unwrap();let allowed=Path::new(&std::env::var("AUDIT_B_ALLOWED_DIR").expect("set AUDIT_B_ALLOWED_DIR to an approved local sample directory")).canonicalize().unwrap();assert!(path.starts_with(allowed));assert!(fs::metadata(&path).unwrap().len()<20_000_000);
        let before=fs::read(&path).unwrap();let sample=engine::inspect_normalized_source(binding).unwrap();assert!(!sample.rows.is_empty());
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../outputs/b-batch-acceptance/real");fs::create_dir_all(&root).unwrap();let data=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/b-batch-acceptance");fs::create_dir_all(&data).unwrap();let file=data.join("real-sample.csv");
        let mut writer=csv::Writer::from_path(&file).unwrap();writer.write_record(["来源记录","科目文本","金额原值"]).unwrap();
        let mut expected=Vec::new();
        for (i,row) in sample.rows.iter().take(20).enumerate(){let text=row.first().cloned().unwrap_or_default();let amount=row.get(6).cloned().unwrap_or_default();writer.write_record([sample.physical_rows[i].to_string(),text.clone(),amount.clone()]).unwrap();expected.push((text,amount));}writer.flush().unwrap();drop(writer);
        let nodes=vec![node("input","source",json!({"source_key":"input"})),node("money","decimal",json!({"fields":[{"source":"金额原值","target":"金额"}]})),node("formula","derive",json!({"expression":"[金额]-[金额]","target":"自核差额"})),node("text","derive",json!({"expression":"LEFT([科目文本],2)","target":"文本前两字"}))];let edges=[("input","money"),("money","formula"),("formula","text")].iter().enumerate().map(|(i,(s,t))|Edge{id:format!("e{i}"),source:(*s).into(),target:(*t).into(),source_port:None,target_port:None}).collect::<Vec<_>>();
        let project:ProjectFile=serde_json::from_value(json!({"schema_version":1,"workflow":{"schema_version":1,"name":"实样小量核对","nodes":nodes,"edges":edges},"bindings":{"input":{"path":file,"fields":{}}}})).unwrap();
        let result=process_project_data(project.clone(),"text",&root.to_string_lossy()).unwrap();let p=engine::preview(&result.database,"n_text",200).unwrap();let index=p.columns.iter().position(|c|c=="来源记录").unwrap();let text_index=p.columns.iter().position(|c|c=="文本前两字").unwrap();let diff=p.columns.iter().position(|c|c=="自核差额").unwrap();
        for row in &p.rows{let n=row[index].parse::<usize>().unwrap();let i=sample.physical_rows.iter().position(|r|*r==n).unwrap();assert_eq!(row[text_index],expected[i].0.chars().take(2).collect::<String>());assert_eq!(row[diff],"0.0000");}
        assert!(!p.rows.is_empty());export_node_data(result.clone(),"text","normal",&root.join("result.csv").to_string_lossy()).unwrap();save_project(root.join("project.json").to_string_lossy().into(),project).unwrap();assert_eq!(fs::read(&path).unwrap(),before);
        fs::write(root.join("verification.json"),serde_json::to_vec_pretty(&json!({"kind":"real-small-sample-engine","sample_rows":expected.len(),"normal_rows":p.total_rows,"source_unchanged":true,"text_and_difference_checked":true})).unwrap()).unwrap();
    }
    #[test]
    #[ignore = "requires approved outputs/column-acceptance/real-input.csv"]
    fn approved_column_actions_chinese_and_consecutive_decimal() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../outputs/column-acceptance");
        let fixture=root.join("real-input.csv");
        let mut reader=csv::Reader::from_path(&fixture).unwrap();let headers=reader.headers().unwrap().clone();let left=headers.get(6).unwrap();let right=headers.get(7).unwrap();
        let input=node("input","source",serde_json::json!({"source_key":"input"}));
        let trim=node("trim","text_clean",serde_json::json!({"fields":[{"source":left,"target":"清理金额"}],"mode":"trim"}));
        let first=node("first","decimal",serde_json::json!({"fields":[{"source":"清理金额","target":"借方金额"}]}));
        let second=node("second","decimal",serde_json::json!({"fields":[{"source":right,"target":"贷方金额"}]}));
        let calc=node("calc","derive",serde_json::json!({"expression":"[借方金额]-[贷方金额]","target":"差额"}));
        let names=["input","trim","first","second","calc"];let edges:Vec<model::Edge>=names.windows(2).enumerate().map(|(i,p)|model::Edge{id:format!("e{i}"),source:p[0].into(),target:p[1].into(),source_port:None,target_port:None}).collect();
        let project:ProjectFile=serde_json::from_value(serde_json::json!({"schema_version":1,"workflow":{"schema_version":1,"name":"列转换验收","nodes":[input,trim,first,second,calc],"edges":edges},"bindings":{"input":{"path":fixture,"fields":{}}}})).unwrap();
        let result=process_project_data(project.clone(),"calc",&root.to_string_lossy()).unwrap();
        assert_eq!(result.reviews[2].rows,1286);assert_eq!(result.reviews[2].exceptions,136);
        assert_eq!(result.reviews[3].rows,1222);assert_eq!(result.reviews[3].exceptions,64);
        let db=duckdb::Connection::open(&result.database).unwrap();let total:String=db.query_row("SELECT CAST(SUM(差额) AS VARCHAR) FROM n_calc",[],|r|r.get(0)).unwrap();let expected_path=std::env::var("AUDIT_COLUMN_EXPECTED").map(std::path::PathBuf::from).unwrap_or_else(|_|root.join("desktop-verification.json"));
        let expected:serde_json::Value=serde_json::from_slice(&fs::read(expected_path).expect("provide approved local independent amount verification report")).unwrap();
        let expected_sum=expected.get("sum").and_then(serde_json::Value::as_str).expect("independent report must include decimal sum");
        let equal:bool=db.query_row("SELECT CAST(? AS DECIMAL(38,4)) = CAST(? AS DECIMAL(38,4))",[total.as_str(),expected_sum],|r|r.get(0)).unwrap();assert!(equal,"computed amount differs from local independent expectation");
        let mut cross=project.clone();
        let group=headers.get(2).unwrap();
        let key=headers.get(0).unwrap();
        cross.workflow.nodes.push(node("delta","derive",serde_json::json!({"expression":format!("[差额]-PREV([差额],1,[借方金额],[{group}])"),"target":"组内差额变动"})));
        cross.workflow.edges.push(model::Edge{id:"e4".into(),source:"calc".into(),target:"delta".into(),source_port:None,target_port:None});
        cross.workflow.nodes.push(node("label","derive",serde_json::json!({"expression":format!("[{group}] & \"|\" & [{key}]"),"target":"组合编号"})));
        cross.workflow.edges.push(model::Edge{id:"e5".into(),source:"delta".into(),target:"label".into(),source_port:None,target_port:None});
        let cross_result=process_project_data(cross,"label",&root.to_string_lossy()).unwrap();
        let cross_db=duckdb::Connection::open(&cross_result.database).unwrap();
        let group=workflow::sql_name(group);
        let key=workflow::sql_name(key);
        let scan=format!("SELECT {group}, {key}, CAST(差额*10000 AS BIGINT), CAST(组内差额变动*10000 AS BIGINT), 组内差额变动__error, 组合编号 FROM n_label ORDER BY {group}, 借方金额 ASC NULLS LAST, __file, __sheet, __record, __row");
        let mut stmt=cross_db.prepare(&scan).unwrap();
        let mut rows=stmt.query([]).unwrap();
        let mut previous:Option<(Option<String>,i64)>=None;
        let mut count=0;
        while let Some(row)=rows.next().unwrap(){
            let group:Option<String>=row.get(0).unwrap();
            let key:Option<String>=row.get(1).unwrap();
            let amount:i64=row.get(2).unwrap();
            let actual:Option<i64>=row.get(3).unwrap();
            let error:Option<String>=row.get(4).unwrap();
            let label:Option<String>=row.get(5).unwrap();
            let expected=previous.as_ref().filter(|(g,_)|g==&group).map(|(_,last)|amount-last);
            assert_eq!(actual,expected);
            assert_eq!(error.as_deref(),if expected.is_none(){Some("组内没有目标行")}else{None});
            assert_eq!(label,group.as_ref().zip(key).map(|(a,b)|format!("{a}|{b}")));
            previous=Some((group,amount));
            count+=1;
        }
        assert_eq!(count,1222);
        let mut bad=project;bad.workflow.nodes[3].config=serde_json::json!({"fields":[{"source":right,"target":"借方金额"}]});assert!(process_project_data(bad,"calc",&root.to_string_lossy()).unwrap_err().contains("目标字段已存在"));
    }
    use super::*;

    #[test]
    fn normalized_input_accepts_legacy_mapping_but_rejects_double_structure() {
        let binding = SourceBinding {
            reading_baseline: None,
            path: "placeholder.xlsx".into(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: Some(model::TableLayout {
                first_col: 1,
                last_col: 1,
                header_rows: vec![1],
                data_start_row: 2,
                data_end_row: None,
                fill_merged_data_cols: vec![],
                field_names: BTreeMap::new(),
                strict_columns: false,
            }),
            text: None,
        };
        let company = CompanyConfig {
            schema_version: 1,
            company: "结构校验".into(),
            sources: BTreeMap::from([("input".into(), binding)]),
        };
        let source = node(
            "input",
            "source",
            json!({"source_key":"input","normalize_structure":true}),
        );
        let legacy_map = node(
            "mapping",
            "map",
            json!({"source_key":"input","fields":["amount"]}),
        );
        let edge = Edge {
            id: "input_next".into(),
            source: "input".into(),
            target: "mapping".into(),
            source_port: None,
            target_port: None,
        };
        let mut wf = Workflow {
            schema_version: 1,
            name: "结构接入校验".into(),
            nodes: vec![source, legacy_map],
            edges: vec![edge],
        };
        assert!(workflow::ordered_nodes(&wf, &company).is_ok());
        wf.nodes[0].config = json!({"source_key":"input"});
        assert!(workflow::ordered_nodes(&wf, &company)
            .unwrap_err()
            .contains("请先连接结构整理节点"));
        wf.nodes[0].config = json!({"source_key":"input","normalize_structure":true});
        wf.nodes[1] = node("mapping", "structure", json!({"source_key":"input"}));
        assert!(workflow::ordered_nodes(&wf, &company)
            .unwrap_err()
            .contains("不能重复整理"));
    }

    #[test]
    fn project_replace_failure_keeps_previous_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/poc-acceptance/save-recovery");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("project.json");
        fs::write(&target, b"previous").unwrap();
        let error = engine::atomic_replace(
            &target,
            |temp| {
                fs::write(temp, b"unfinished").unwrap();
                Err("controlled writer failure".into())
            },
            true,
        );
        assert!(error.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous");
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let locked = fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&target)
                .unwrap();
            assert!(engine::atomic_replace(
                &target,
                |temp| fs::write(temp, b"new version").map_err(|e| e.to_string()),
                true
            )
            .is_err());
            assert_eq!(fs::read(&target).unwrap(), b"previous");
            drop(locked);
        }
        engine::atomic_replace(
            &target,
            |temp| fs::write(temp, b"complete").map_err(|e| e.to_string()),
            true,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"complete");
        assert_eq!(
            fs::read(target.with_extension("json.bak")).unwrap(),
            b"previous"
        );
    }

    #[test]
    #[ignore = "requires AUDIT_STRUCTURE_FIXTURE for an approved local workbook"]
    fn poc_process_without_export_and_direct_export_real_rows() {
        let fixture = std::env::var("AUDIT_STRUCTURE_FIXTURE").unwrap();
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../outputs/poc-acceptance/backend");
        fs::create_dir_all(&root).unwrap();
        let layout=serde_json::from_value(serde_json::json!({"first_col":1,"last_col":14,"header_rows":[5,6],"data_start_row":7,"fill_merged_data_cols":[],"field_names":{"5":"原始第5列_方向","11":"原始第11列_方向","13":"原始第13列","14":"原始第14列"},"strict_columns":true})).unwrap();
        let binding = SourceBinding {
            path: fixture.clone(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: Some(layout),
            text: None,
            reading_baseline: None,
        };
        let input = node(
            "input",
            "source",
            serde_json::json!({"source_key":"input","normalize_structure":true}),
        );
        let map = node(
            "map",
            "map",
            serde_json::json!({"mappings":[{"source":"科目编码","target":"复核编码"},{"source":"科目名称","target":"科目名称"}]}),
        );
        let filter = node(
            "filter",
            "filter",
            serde_json::json!({"condition":{"field":"复核编码","operator":"equals","value":"1001"}}),
        );
        let mut project = ProjectFile {
            schema_version: 1,
            company_name: None,
            bindings: BTreeMap::from([("input".into(), binding)]),
            workflow: Workflow {
                schema_version: 1,
                name: "PoC实样验收".into(),
                nodes: vec![input, node("keep", "map", serde_json::json!({"mappings":[{"source":"科目编码","target":"科目编码"},{"source":"科目名称","target":"科目名称"}]})), map, filter],
                edges: vec![
                    model::Edge {
                        id: "a".into(),
                        source: "input".into(),
                        target: "keep".into(),
                        source_port: None,
                        target_port: None,
                    },
                    model::Edge {
                        id: "keep-map".into(),
                        source: "keep".into(),
                        target: "map".into(),
                        source_port: None,
                        target_port: None,
                    },
                    model::Edge {
                        id: "b".into(),
                        source: "map".into(),
                        target: "filter".into(),
                        source_port: None,
                        target_port: None,
                    },
                ],
            },
        };
        project.workflow.nodes.push(node(
            "unrelated",
            "source",
            serde_json::json!({"source_key":"unrelated"}),
        ));
        let before = fs::read(&fixture).unwrap();
        let result =
            process_project_data(project.clone(), "filter", &root.to_string_lossy()).unwrap();
        assert_eq!(result.reviews.len(), 4);
        assert_eq!(result.reviews[0].rows, 1422);
        assert_eq!(result.reviews[3].rows, 2);
        assert_eq!(result.reviews[3].excluded, 1420);
        assert!(!Path::new(&result.run_dir)
            .join("filter_excluded.csv")
            .exists());
        assert!(export_node_data(result.clone(), "filter", "normal", &fixture).is_err());
        let main = root.join("included.csv");
        let excluded = root.join("excluded.csv");
        assert_eq!(
            export_node_data(result.clone(), "filter", "normal", &main.to_string_lossy())
                .unwrap()
                .rows,
            2
        );
        assert_eq!(
            export_node_data(
                result.clone(),
                "filter",
                "excluded",
                &excluded.to_string_lossy()
            )
            .unwrap()
            .rows,
            1420
        );
        let conn = duckdb::Connection::open(&result.database).unwrap();
        let mismatch:i64=conn.query_row("SELECT COUNT(*) FROM n_map m JOIN n_input i USING (__row) WHERE m.复核编码 IS DISTINCT FROM i.科目编码 OR m.科目名称 IS DISTINCT FROM i.科目名称",[],|row|row.get(0)).unwrap();
        assert_eq!(mismatch, 0);
        let complete = root.join("project.json");
        save_project_file(&complete.to_string_lossy(), &project).unwrap();
        let reopened: ProjectFile = serde_json::from_slice(&fs::read(complete).unwrap()).unwrap();
        assert_eq!(reopened.workflow.nodes.len(), 5);
        assert_eq!(fs::read(&fixture).unwrap(), before);
        assert_eq!(csv::Reader::from_path(main).unwrap().records().count(), 2);
        assert_eq!(
            csv::Reader::from_path(excluded).unwrap().records().count(),
            1420
        );
    }

    #[test]
    #[ignore = "requires AUDIT_STRUCTURE_FIXTURE for an approved local workbook"]
    fn approved_real_structure_preview_export_and_reopen() {
        let fixture = std::env::var("AUDIT_STRUCTURE_FIXTURE").expect("set approved fixture path");
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../outputs/structure-acceptance");
        let mut binding = SourceBinding {
            reading_baseline: None,
            path: fixture,
            sheet: None,
            fields: BTreeMap::new(),
            layout: None,
            text: None,
        };
        let report = source_structure::inspect(&binding).unwrap();
        assert!(report.raw.rows.len() <= 40);
        assert!(
            report
                .candidates
                .iter()
                .any(|c| c.layout.header_rows == vec![5, 6]),
            "observed multilevel header must be offered"
        );
        let mut layout: model::TableLayout = serde_json::from_value(serde_json::json!({
            "first_col":1,"last_col":14,"header_rows":[5,6],"data_start_row":7,"fill_merged_data_cols":[]
        })).unwrap();
        binding.layout = Some(layout.clone());
        let unresolved = source_structure::inspect(&binding).unwrap().active.unwrap();
        assert!(
            !unresolved.ready,
            "duplicate directions and unnamed physical column must not pass silently"
        );
        // Neutral, physical-column labels for structural validation, not inferred accounting meaning.
        layout.field_names.insert(5, "原始第5列_方向".into());
        layout.field_names.insert(11, "原始第11列_方向".into());
        layout.field_names.insert(13, "原始第13列".into());
        layout.field_names.insert(14, "原始第14列".into());
        layout.strict_columns = true;
        binding.layout = Some(layout);
        let check = source_structure::check_reading(&binding, true);
        assert!(check.compatible && check.full_checked);
        binding.reading_baseline = check.baseline;
        let window = engine::inspect_raw_window(&binding, 64, 11).unwrap();
        assert_eq!(window.physical_rows[3], 67);
        assert!(
            !window.rows[3][3].is_empty(),
            "late populated column must be visible at its real location"
        );
        let mut narrowed = binding.clone();
        narrowed.reading_baseline = None;
        narrowed.layout.as_mut().unwrap().last_col = 13;
        narrowed.layout.as_mut().unwrap().field_names.remove(&14);
        let local = source_structure::check_reading(&narrowed, false);
        assert!(
            local.compatible,
            "bounded preview cannot claim to see a later column"
        );
        let full = source_structure::check_reading(&narrowed, true);
        assert!(!full.compatible);
        assert!(full
            .diagnostics
            .iter()
            .any(|d| d.row == Some(67) && d.col == Some(14)));
        let mut changed = binding.clone();
        changed.reading_baseline.as_mut().unwrap().header_cells[0][0] = "已变化的读取依据".into();
        assert!(!source_structure::check_reading(&changed, false).compatible);
        assert!(engine::inspect_normalized_source(&changed).is_err());
        if let Ok(other) = std::env::var("AUDIT_REUSE_FIXTURE") {
            let mut replacement = binding.clone();
            replacement.path = other;
            let check = source_structure::check_reading(&replacement, false);
            assert!(
                !check.compatible,
                "different real structure must not silently reuse the original plan"
            );
            assert!(!check.diagnostics.is_empty());
        }
        let preview = engine::inspect_normalized_source(&binding).unwrap();
        assert_eq!(preview.headers.len(), 14);
        assert_eq!(preview.headers[0], "科目编码");
        assert_eq!(preview.physical_rows[0], 7);
        assert!(preview
            .headers
            .iter()
            .any(|s| s.contains("期初余额") && s.contains("本币")));
        let project = ProjectFile {
            schema_version: 1,
            company_name: None,
            bindings: BTreeMap::from([("input".into(), binding)]),
            workflow: Workflow {
                schema_version: 1,
                name: "结构接入实样核查".into(),
                nodes: vec![
                    node(
                        "input",
                        "source",
                        serde_json::json!({"source_key":"input","normalize_structure":true}),
                    ),
                    node(
                        "export",
                        "export",
                        serde_json::json!({"output_root":root.join("runs").to_string_lossy(),"file_name":"result.csv"}),
                    ),
                ],
                edges: vec![model::Edge {
                    id: "input_export".into(),
                    source: "input".into(),
                    target: "export".into(),
                    source_port: Some("main".into()),
                    target_port: None,
                }],
            },
        };
        let path = root.join("project.json");
        save_project(path.to_string_lossy().into_owned(), project.clone()).unwrap();
        let reopened = load_project(path.to_string_lossy().into_owned()).unwrap();
        let result = run_project_data(reopened).unwrap();
        let conn = duckdb::Connection::open(&result.database).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM n_input", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0);
        assert_eq!(result.reviews[0].rows, count);
        for (index, header) in preview.headers.iter().enumerate() {
            let sql=format!("SELECT COUNT(*) FROM n_input o JOIN n_input__raw r ON o.__row=r.__row WHERE o.{} IS DISTINCT FROM r.{}",workflow::sql_name(header),workflow::sql_name(&format!("col_{}",index+1)));
            let diff: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
            assert_eq!(
                diff,
                0,
                "raw values must stay unchanged at physical column {}",
                index + 1
            );
        }
        let raw_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM n_input__raw WHERE __row>=7",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, raw_count);
        drop(conn);
        let stored = engine::preview(&result.database, "n_input", 20).unwrap();
        assert_eq!(stored.total_rows, count);
        let row_col = stored.columns.iter().position(|c| c == "__row").unwrap();
        assert_eq!(stored.rows[0][row_col], "7");
        let exported = csv::Reader::from_path(Path::new(&result.run_dir).join("result.csv"))
            .unwrap()
            .records()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(exported.len() as i64, count);
        fs::write(
            root.join("last-run.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("Real workbook: preview, {} unchanged physical columns, {} rows, export and project reopening verified.",preview.headers.len(),count);
    }
    use crate::model::{Edge, Node, Point, TextOptions};
    use serde_json::json;

    fn node(id: &str, kind: &str, config: serde_json::Value) -> Node {
        Node {
            id: id.into(),
            kind: kind.into(),
            version: 1,
            label: id.into(),
            position: Point { x: 0.0, y: 0.0 },
            config,
        }
    }

    #[test]
    fn text_inputs_keep_raw_preview_separate_from_table_validation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/test-runs")
            .join(format!("text-input-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("synthetic.csv");
        let (encoded, _, _) = encoding_rs::GB18030.encode("科目,金额\n管理费用,100\n财务费用,20\n");
        fs::write(&source, encoded.as_ref()).unwrap();
        let binding = SourceBinding {
            reading_baseline: None,
            path: source.to_string_lossy().to_string(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: None,
            text: Some(TextOptions {
                encoding: "gb18030".into(),
                delimiter: ",".into(),
                header_record: 1,
            }),
        };
        let raw = engine::inspect_raw_source(&binding, 10).unwrap();
        assert_eq!(raw.physical_rows, vec![1, 2, 3]);
        assert_eq!(raw.rows[0], ["科目", "金额"]);
        let table = engine::inspect_source(&binding, 10).unwrap();
        assert_eq!(table.headers, ["科目", "金额"]);
        assert_eq!(table.physical_rows, vec![2, 3]);
        let workflow = Workflow {
            schema_version: 1,
            name: "编码导入对拍".into(),
            nodes: vec![node("input", "source", json!({"source_key":"input"}))],
            edges: vec![],
        };
        let company = CompanyConfig {
            schema_version: 1,
            company: "虚构测试".into(),
            sources: BTreeMap::from([("input".into(), binding.clone())]),
        };
        let imported =
            engine::run(&workflow, &company, root.join("runs").to_str().unwrap()).unwrap();
        let imported_rows = engine::preview(&imported.database, "n_input", 10).unwrap();
        assert_eq!(imported_rows.total_rows, 2);
        assert!(imported_rows
            .rows
            .iter()
            .any(|row| row.iter().any(|value| value == "管理费用")));
        let mut wrong_encoding = binding.clone();
        wrong_encoding.text.as_mut().unwrap().encoding = "utf-8".into();
        assert!(engine::inspect_raw_source(&wrong_encoding, 10)
            .unwrap_err()
            .contains("GB18030"));

        let line_endings = root.join("line-endings.csv");
        fs::write(
            &line_endings,
            "科目,摘要\r\n6602,\"第一行\r\n第二行\"\r\n6603,正常\n",
        )
        .unwrap();
        let line_binding = SourceBinding {
            reading_baseline: None,
            path: line_endings.to_string_lossy().to_string(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: None,
            text: None,
        };
        let raw_lines = engine::inspect_raw_source(&line_binding, 10).unwrap();
        assert_eq!(raw_lines.physical_rows, vec![1, 2, 4]);
        assert_eq!(raw_lines.rows[1][1], "第一行\r\n第二行");
        assert_eq!(
            engine::inspect_source(&line_binding, 10)
                .unwrap()
                .physical_rows,
            vec![2, 4]
        );

        let txt = root.join("synthetic.txt");
        fs::write(&txt, "说明行\n科目\t金额\n管理费用\t100\n").unwrap();
        let txt_binding = SourceBinding {
            reading_baseline: None,
            path: txt.to_string_lossy().to_string(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: None,
            text: Some(TextOptions {
                encoding: "utf-8".into(),
                delimiter: "\t".into(),
                header_record: 2,
            }),
        };
        assert_eq!(
            engine::inspect_raw_source(&txt_binding, 10)
                .unwrap()
                .rows
                .len(),
            3
        );
        assert_eq!(
            engine::inspect_source(&txt_binding, 10)
                .unwrap()
                .physical_rows,
            vec![3]
        );

        let malformed = root.join("duplicate.csv");
        fs::write(&malformed, "科目,科目\nA,B\n").unwrap();
        let malformed_binding = SourceBinding {
            reading_baseline: None,
            path: malformed.to_string_lossy().to_string(),
            sheet: None,
            fields: BTreeMap::new(),
            layout: None,
            text: None,
        };
        assert_eq!(
            engine::inspect_raw_source(&malformed_binding, 10)
                .unwrap()
                .rows[0],
            ["科目", "科目"]
        );
        assert!(engine::inspect_source(&malformed_binding, 10)
            .unwrap_err()
            .contains("重复列"));
    }

    #[test]
    fn new_project_roundtrips_and_preserves_excluded_rows() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/test-runs")
            .join(format!("single-file-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.csv");
        fs::write(&source, "科目编码,金额\n6602,100\n6603,20\n6602,30\n").unwrap();
        let output = root.join("runs");
        let mut bindings = BTreeMap::new();
        bindings.insert(
            "input".into(),
            SourceBinding {
                reading_baseline: None,
                path: source.to_string_lossy().to_string(),
                sheet: None,
                fields: BTreeMap::new(),
                layout: None,
                text: None,
            },
        );
        let project = ProjectFile {
            schema_version: 1,
            company_name: None,
            workflow: Workflow {
                schema_version: 1,
                name: "单文件对拍".into(),
                nodes: vec![
                    node("input", "source", json!({"source_key":"input"})),
                    node(
                        "mapping",
                        "map",
                        json!({"mappings":[{"source":"科目编码","target":"科目编码"},{"source":"金额","target":"金额"}]}),
                    ),
                    node(
                        "filter",
                        "filter",
                        json!({"condition":{"field":"科目编码","operator":"equals","value":"6602"}}),
                    ),
                    node(
                        "export",
                        "export",
                        json!({"output_root":output.to_string_lossy(),"file_name":"结果.csv"}),
                    ),
                ],
                edges: [
                    ("input", "mapping"),
                    ("mapping", "filter"),
                    ("filter", "export"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (source, target))| Edge {
                    id: format!("e{index}"),
                    source: source.into(),
                    target: target.into(),
                    source_port: Some("main".into()),
                    target_port: None,
                })
                .collect(),
            },
            bindings,
        };
        let path = root.join("project.json");
        save_project(path.to_string_lossy().to_string(), project.clone()).unwrap();
        let loaded = load_project(path.to_string_lossy().to_string()).unwrap();
        assert_eq!(validate_project_data(&loaded).unwrap().len(), 4);
        let result = run_project_data(loaded.clone()).unwrap();
        let filter = result
            .reviews
            .iter()
            .find(|item| item.node_id == "filter")
            .unwrap();
        assert_eq!((filter.rows, filter.excluded), (2, 1));
        let mut reader =
            csv::Reader::from_path(Path::new(&result.run_dir).join("结果.csv")).unwrap();
        let values: Vec<_> = reader.records().map(|record| record.unwrap()).collect();
        assert_eq!(values.len(), 2);
        assert!(values
            .iter()
            .all(|row| row.iter().any(|cell| cell == "6602")));
        let mut broken = loaded;
        broken
            .workflow
            .nodes
            .iter_mut()
            .find(|item| item.id == "mapping")
            .unwrap()
            .config = json!({"mappings":[{"source":"不存在","target":"科目编码"}]});
        assert!(validate_project_data(&broken)
            .unwrap_err()
            .contains("缺少字段"));
    }

    #[test]
    fn legacy_workflow_still_reads_without_new_port_fields() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/golden/workflow.json");
        let old: Workflow = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(old.edges.iter().all(|edge| edge.source_port.is_none()));
        validate_template(&old).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../outputs/test-runs")
            .join(format!("legacy-import-{}", std::process::id()));
        let mut sources = BTreeMap::new();
        for source in old.nodes.iter().filter(|node| node.kind == "source") {
            sources.insert(
                workflow::required_str(&source.config, "source_key")
                    .unwrap()
                    .to_string(),
                SourceBinding {
                    reading_baseline: None,
                    path: "synthetic.csv".into(),
                    sheet: None,
                    fields: BTreeMap::new(),
                    layout: None,
                    text: None,
                },
            );
        }
        save_template(
            root.to_string_lossy().to_string(),
            old,
            CompanyConfig {
                schema_version: 1,
                company: "虚构甲公司".into(),
                sources,
            },
        )
        .unwrap();
        let imported = import_legacy_project(root.to_string_lossy().to_string()).unwrap();
        assert_eq!(imported.company_name.as_deref(), Some("虚构甲公司"));
    }
}
