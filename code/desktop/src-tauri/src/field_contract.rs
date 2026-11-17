use crate::model::{CompanyConfig, FieldDescriptor, Node};
use crate::workflow::required_str;
use serde_json::Value;

pub fn check_inputs(
    node: &Node,
    inputs: &[&[FieldDescriptor]],
    company: &CompanyConfig,
) -> Result<Vec<FieldDescriptor>, String> {
    let left = *inputs.first().ok_or("缺少上游输入")?;
    let field = |input: &[FieldDescriptor], name: &str, side: &str| {
        input.iter().find(|f| f.name == name).cloned()
            .ok_or_else(|| format!("{side}缺少字段 {name}"))
    };
    let output = |name: &str, data_type: &str, nullable| FieldDescriptor {
        name: name.into(), data_type: data_type.into(), nullable,
        origin: node.id.clone(), confirmed: true,
    };
    match node.kind.as_str() {
        "pivot" => {
            crate::pivot::read(node,left)?;
            Ok(vec![FieldDescriptor{name:"__dynamic_schema".into(),data_type:"UNKNOWN".into(),nullable:true,origin:node.id.clone(),confirmed:false}])
        }
        "union" if node.version == 2 => {
            crate::join_append::AppendRule::read(node)?.output_fields(node, inputs)
        }
        "union" => {
            let right = *inputs.get(1).ok_or("追加缺少右侧输入")?;
            for column in right {
                field(left, &column.name, "左侧")?;
            }
            left.iter().map(|column| {
                let other = field(right, &column.name, "右侧")?;
                if column.data_type != other.data_type {
                    return Err(format!("追加字段 {} 类型不一致：左侧 {}，右侧 {}", column.name, column.data_type, other.data_type));
                }
                let mut column = column.clone();
                column.nullable |= other.nullable;
                column.origin = node.id.clone();
                Ok(column)
            }).collect()
        }
        "join" if node.version == 2 => crate::join_append::join_fields(
            node, left, inputs.get(1).ok_or("关联缺少右侧输入")?,
        ),
        "join" => {
            let right = *inputs.get(1).ok_or("关联缺少右侧输入")?;
            let mut fields = Vec::new();
            for key in crate::workflow::required_strings(&node.config, "keys")? {
                let a = field(left, &key, "左侧")?;
                field(right, &key, "右侧")?;
                fields.push(output(&key, &a.data_type, true));
            }
            let a = field(left, required_str(&node.config, "left_amount")?, "左侧")?;
            let b = field(right, required_str(&node.config, "right_amount")?, "右侧")?;
            fields.extend([
                output("left_amount", &a.data_type, true),
                output("right_amount", &b.data_type, true),
                output("difference", "DECIMAL(38,4)", true),
                output("issue_count", "BIGINT", false),
                output("match_status", "VARCHAR", false),
            ]);
            Ok(fields)
        }
        "aggregate" if node.version == 1 => {
            let mut fields = crate::workflow::required_strings(&node.config, "keys")?.iter()
                .map(|key| field(left, key, "上一步"))
                .collect::<Result<Vec<_>, _>>()?;
            field(left, required_str(&node.config, "amount")?, "上一步")?;
            fields.push(output(required_str(&node.config, "target")?, "DECIMAL(38,4)", false));
            fields.push(output("__members", "BIGINT", false));
            Ok(fields)
        }
        "map" if node.config.get("mappings").is_none() => {
            let key = required_str(&node.config, "source_key")?;
            let binding = company.sources.get(key).ok_or_else(|| format!("缺少资料 {key}"))?;
            let mut fields = vec![output("company", "VARCHAR", false)];
            for name in crate::workflow::required_strings(&node.config, "fields")? {
                let source = binding.fields.get(&name).ok_or_else(|| format!("缺少字段映射 {name}"))?;
                let mut column = field(left, source, "上一步")?;
                column.name = name;
                fields.push(column);
            }
            Ok(fields)
        }
        "structure" => {
            let key = required_str(&node.config, "source_key")?;
            let binding = company.sources.get(key).ok_or_else(|| format!("缺少资料 {key}"))?;
            let sample = crate::engine::inspect_normalized_source(binding)?;
            Ok(sample.headers.iter().map(|name| output(name, "VARCHAR", true)).collect())
        }
        _ => check(node, left),
    }
}

pub fn business_name(name:&str)->Result<(),String>{
    if name.trim()!=name || name.is_empty() || name.chars().count()>100 || name.starts_with("__") || name.eq_ignore_ascii_case("company") || name.chars().any(|c|c.is_control()) {return Err(format!("字段名无效或保留：{name}"));} Ok(())
}
pub fn numeric(t:&str)->bool {let t=t.to_uppercase(); ["DECIMAL","INTEGER","BIGINT","SMALLINT","TINYINT","HUGEINT","DOUBLE","FLOAT","REAL","UBIGINT","UINTEGER"].iter().any(|s|t.starts_with(s))}
fn amount_options(item:&Value)->Result<(),String>{
    for name in ["empty_as_zero","trim_special_space","parentheses_negative","thousands"]{
        if item.get(name).is_some_and(|value|!value.is_boolean()){return Err(format!("金额选项 {name} 须明确选择"));}
    }
    if let Some(prefix)=item.get("currency_prefix"){
        let prefix=prefix.as_str().ok_or("币种前缀须为文本")?;
        if !["","¥","￥","CNY","RMB","USD","$","EUR","€","HKD","HK$"].contains(&prefix){
            return Err("请选择支持的单一币种前缀".into());
        }
    }
    Ok(())
}
pub fn check(node:&Node,input:&[FieldDescriptor])->Result<Vec<FieldDescriptor>,String>{
    let cfg=&node.config; let mut out=input.to_vec();
    let source=|name:&str| input.iter().find(|f|f.name==name).ok_or_else(||format!("上一步缺少字段：{name}"));
    let mut add=|name:&str,t:&str|->Result<(),String>{business_name(name)?;if out.iter().any(|f|f.name.to_lowercase()==name.to_lowercase()){return Err(format!("目标字段已存在：{name}"));}out.push(FieldDescriptor{name:name.into(),data_type:t.into(),nullable:true,origin:node.id.clone(),confirmed:true});Ok(())};
    match node.kind.as_str(){
      "mark"=>{source(required_str(cfg,"field")?)?;add(required_str(cfg,"target")?,"BOOLEAN")?;},
      "derive"=>{let expression=required_str(cfg,"expression")?;let compiled=crate::formula::compile(expression,input)?;let target=required_str(cfg,"target")?;add(target,compiled.data_type())?;add(&format!("{target}__error"),"VARCHAR")?;},
      "sort"|"duplicates"|"deduplicate"|"sequence"=>return crate::table_ops::check(node,input),
      "aggregate" if node.version==2=>return crate::table_ops::check(node,input),
      "filter" if cfg.get("conditions").is_some()=>{crate::table_ops::predicate(cfg,input)?;},
      "decimal"|"text_clean"=>{let fields=cfg.get("fields").and_then(Value::as_array).ok_or("缺少fields")?;if fields.is_empty(){return Err("请选择字段".into());}for f in fields{source(required_str(f,"source")?)?;let target=required_str(f,"target")?;add(target,if node.kind=="decimal"{"DECIMAL(28,4)"}else{"VARCHAR"})?;if node.kind=="decimal"&&node.version==2{amount_options(f)?;if f.get("empty_as_zero").is_some_and(|value| !value.is_boolean()){return Err("空白按零必须明确选择".into());}add(&format!("{target}__amount_status"),"VARCHAR")?;}}
        if node.kind=="text_clean"{match required_str(cfg,"mode")?{"trim"=>{},"replace"=>{required_str(cfg,"find")?;cfg.get("replacement").and_then(Value::as_str).ok_or("缺少替换内容")?;},_=>return Err("不支持的清理模式".into())}}
      },
      "text_extract"=>{source(required_str(cfg,"source")?)?;let target=required_str(cfg,"target")?;add(target,"VARCHAR")?;add(&format!("{target}__matched"),"BOOLEAN")?;let pattern=required_str(cfg,"pattern")?;if pattern.len()>1000{return Err("提取规则超过1000字节".into());}if cfg.get("group").is_some_and(|v|v.as_u64().is_none_or(|n|n>99)){return Err("提取分组须为0至99".into());}},
      "filter"=>{if let Some(c)=cfg.get("condition"){let f=source(required_str(c,"field")?)?;let op=required_str(c,"operator")?;if op.starts_with("date_"){crate::table_ops::predicate(cfg,input)?;}if op.starts_with("numeric_"){if !numeric(&f.data_type){return Err("数值筛选需要数值字段，请先转换金额".into());}if !["numeric_gt","numeric_gte","numeric_lt","numeric_lte","numeric_eq","numeric_neq"].contains(&op){return Err("未知数值条件".into());}decimal_literal(required_str(c,"value")?)?;}}},
      "map"=>{if let Some(items)=cfg.get("mappings").and_then(Value::as_array){let mut mapped=input.iter().filter(|f|f.name.starts_with("__")||f.name=="company").cloned().collect::<Vec<_>>();for item in items{let mut f=source(required_str(item,"source")?)?.clone();let target=required_str(item,"target")?;business_name(target)?;if mapped.iter().any(|f|f.name.to_lowercase()==target.to_lowercase()){return Err("映射字段重名".into());}f.name=target.into();mapped.push(f);}return Ok(mapped);}},
      _=>{}
    }Ok(out)
}
pub fn decimal_literal(v:&str)->Result<(),String>{let s=v.strip_prefix('-').or_else(||v.strip_prefix('+')).unwrap_or(v);let mut parts=s.split('.');let whole=parts.next().unwrap_or("");let frac=parts.next();if whole.is_empty()||whole.len()>24||!whole.bytes().all(|c|c.is_ascii_digit())||parts.next().is_some()||frac.is_some_and(|s|s.is_empty()||s.len()>4||!s.bytes().all(|c|c.is_ascii_digit())){return Err("比较值须为有效DECIMAL(28,4)，不可使用科学计数法".into());}Ok(())}
