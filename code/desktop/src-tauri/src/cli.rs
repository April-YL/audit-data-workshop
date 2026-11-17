use audit_data_workshop_lib::{
    cli_inspect_source, cli_preview, cli_trace, cli_validate, load_company_file, load_template,
    load_workflow_file,
    model::{CompanyConfig, SourceBinding, Workflow},
    run_cli, save_company_file, save_template, save_workflow_file,
};
use std::{collections::BTreeMap, env, fs, path::Path};

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate") if args.len()==4 => {
            let folder = Path::new(&args[2]);
            let wf: Workflow = serde_json::from_slice(&fs::read(folder.join("workflow.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let cfg: CompanyConfig = serde_json::from_slice(&fs::read(&args[3]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let result = cli_validate(wf,cfg)?;
            println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
        }
        Some("run") if args.len()==5 => {
            let folder = Path::new(&args[2]);
            let wf: Workflow = serde_json::from_slice(&fs::read(folder.join("workflow.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let cfg: CompanyConfig = serde_json::from_slice(&fs::read(&args[3]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let result = run_cli(wf,cfg,&args[4])?;
            println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
        }
        Some("preview") if args.len()==5 => {
            let limit = args[4].parse().map_err(|_|"行数需为整数")?;
            let result = cli_preview(&args[2],&args[3],limit)?;
            println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
        }
        Some("inspect") if args.len()==5 => {
            let limit = args[4].parse().map_err(|_|"行数需为整数")?;
            let binding=SourceBinding{path:args[2].clone(),sheet:if args[3]=="-" {None}else{Some(args[3].clone())},fields:BTreeMap::new(),layout:None,text:None,reading_baseline:None};
            let result=cli_inspect_source(&binding,limit)?;
            println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
        }
        Some("trace") if args.len()==6 => {
            let wf: Workflow = serde_json::from_slice(&fs::read(&args[2]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let group=serde_json::from_str(&args[5]).map_err(|e|format!("追溯键 JSON 无效：{e}"))?;
            let result=cli_trace(&args[3],&wf,&args[4],&group)?;
            println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
        }
        Some("save") if args.len()==5 => {
            let source = Path::new(&args[2]);
            let wf: Workflow = serde_json::from_slice(&fs::read(source.join("workflow.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let cfg: CompanyConfig = serde_json::from_slice(&fs::read(&args[3]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            save_template(args[4].clone(),wf,cfg)?;
            println!("已保存");
        }
        Some("load") if args.len()==3 => {
            let (wf,cfg)=load_template(args[2].clone())?;
            println!("{}",serde_json::to_string_pretty(&(wf,cfg)).map_err(|e|e.to_string())?);
        }
        Some("save-flow") if args.len()==4 => {
            let wf: Workflow = serde_json::from_slice(&fs::read(Path::new(&args[2]).join("workflow.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            save_workflow_file(args[3].clone(),wf)?;
            println!("已保存模板");
        }
        Some("load-flow") if args.len()==3 => {
            let wf=load_workflow_file(args[2].clone())?;
            println!("{}",serde_json::to_string_pretty(&wf).map_err(|e|e.to_string())?);
        }
        Some("save-company") if args.len()==4 => {
            let cfg: CompanyConfig=serde_json::from_slice(&fs::read(&args[2]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            save_company_file(args[3].clone(),cfg)?;
            println!("已保存公司配置");
        }
        Some("load-company") if args.len()==3 => {
            let cfg=load_company_file(args[2].clone())?;
            println!("{}",serde_json::to_string_pretty(&cfg).map_err(|e|e.to_string())?);
        }
        _ => return Err("用法：workshop-cli validate <工作流文件夹> <公司配置.json> | run <工作流文件夹> <公司配置.json> <输出目录> | save-flow <源工作流文件夹> <目标文件夹> | load-flow <文件夹> | save-company <源配置.json> <目标配置.json> | load-company <配置.json> | inspect <数据文件> <工作表或-> <行数> | preview <run.duckdb> <表名> <行数> | trace <工作流.json> <run.duckdb> <关联节点> <键JSON>".into()),
    }
    Ok(())
}
