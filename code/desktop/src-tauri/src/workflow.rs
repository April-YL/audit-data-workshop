use crate::model::{CompanyConfig, Node, Workflow};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub fn identifier(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 100
        || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!("非法字段或节点标识：{value}"));
    }
    Ok(format!("\"{}\"", value))
}

pub fn sql_name(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn reaches(start: &str, target: &str, next: &BTreeMap<&str, Vec<&str>>) -> bool {
    let mut pending = VecDeque::from([start]);
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop_front() {
        if id == target {
            return true;
        }
        if visited.insert(id) {
            pending.extend(next.get(id).into_iter().flatten().copied());
        }
    }
    false
}

pub fn ordered_nodes(workflow: &Workflow, company: &CompanyConfig) -> Result<Vec<Node>, String> {
    if workflow.schema_version != 1 || company.schema_version != 1 {
        return Err("不支持的工作流或公司配置版本".into());
    }
    if workflow.nodes.is_empty() {
        return Err("画布为空".into());
    }
    let mut nodes = BTreeMap::new();
    let mut indegree = BTreeMap::new();
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &workflow.nodes {
        identifier(&node.id)?;
        if node.kind == "calculate" { return Err(format!("节点 {} 使用已移除的两列相减，请删除该节点并改用公式计算", node.label)); }
        if node.version != 1 && !(node.version == 2 && ["aggregate", "decimal", "join", "union"].contains(&node.kind.as_str())) {
            return Err(format!(
                "节点 {} 的语义版本 {} 暂不支持",
                node.label, node.version
            ));
        }
        if nodes.insert(node.id.as_str(), node).is_some() {
            return Err(format!("重复节点 ID：{}", node.id));
        }
        indegree.insert(node.id.as_str(), 0usize);
        if node.kind == "source" {
            let key = required_str(&node.config, "source_key")?;
            if !company.sources.contains_key(key) {
                return Err(format!("公司 {} 缺少资料绑定 {key}", company.company));
            }
        }
    }
    let mut edge_ids = BTreeSet::new();
    for edge in &workflow.edges {
        if !edge_ids.insert(&edge.id) {
            return Err(format!("重复连线 ID：{}", edge.id));
        }
        if !nodes.contains_key(edge.source.as_str()) || !nodes.contains_key(edge.target.as_str()) {
            return Err(format!("悬空连线：{}", edge.id));
        }
        if edge.source == edge.target {
            return Err(format!("节点 {} 不能连接自身", edge.source));
        }
        if edge
            .source_port
            .as_deref()
            .is_some_and(|port| port != "main")
        {
            return Err(format!("连线 {} 的来源输出口不存在", edge.id));
        }
        if nodes[edge.source.as_str()].kind == "export" {
            return Err(format!("导出节点 {} 不能连接下游", edge.source));
        }
        *indegree.get_mut(edge.target.as_str()).unwrap() += 1;
        next.entry(&edge.source).or_default().push(&edge.target);
    }
    for node in &workflow.nodes {
        let inputs = indegree[&node.id.as_str()];
        let expected = match node.kind.as_str() {
            "source" => 0,
            "union" if node.version == 2 => crate::join_append::AppendRule::read(node)?.ports.len(),
            "union" | "join" => 2,
            "map" | "structure" | "text_clean" | "decimal" | "text_extract" | "mark" | "filter"
            | "aggregate" | "pivot" | "sequence" | "inspect" | "export" | "derive" | "sort" | "duplicates" | "deduplicate" => 1,
            other => return Err(format!("未知节点类型：{other}")),
        };
        if inputs != expected {
            return Err(format!(
                "{} 需要 {expected} 条输入连线，实际 {inputs} 条",
                node.label
            ));
        }
        let incoming = workflow.edges.iter().filter(|e| e.target == node.id);
        if node.kind == "union" && node.version == 2 {
            let rule = crate::join_append::AppendRule::read(node)?;
            let ports = rule.ports.iter().map(String::as_str).collect::<Vec<_>>();
            crate::join_append::ordered_inputs(workflow, node, &ports)?;
        } else if (node.kind == "union" && incoming.clone().all(|edge| edge.target_port.is_some()))
            || (node.kind == "join" && node.version == 2)
        {
            crate::join_append::ordered_inputs(workflow, node, &["left", "right"])?;
        } else if node.kind == "join" || node.kind == "union" {
            let mut used_ports = BTreeSet::new();
            for edge in incoming.clone() {
                if let Some(port) = edge.target_port.as_deref() {
                    if port != "left" && port != "right" {
                        return Err(format!("{} 的输入端口 {port} 不存在", node.label));
                    }
                    if !used_ports.insert(port) {
                        return Err(format!("{} 的输入端口 {port} 重复连接", node.label));
                    }
                }
            }
        } else if incoming.clone().any(|e| e.target_port.is_some()) {
            return Err(format!("{} 不接受左右输入端口", node.label));
        }
        if node.kind == "structure" || (node.kind == "map" && node.config.get("mappings").is_none())
        {
            let source_id = workflow
                .edges
                .iter()
                .find(|e| e.target == node.id)
                .ok_or("结构整理或字段映射缺少输入")?
                .source
                .as_str();
            let source = nodes[source_id];
            if node.kind == "structure"
                && source
                    .config
                    .get("normalize_structure")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            {
                return Err(format!(
                    "{} 的上游输入已完成结构整理，不能重复整理；请直接连接后续处理",
                    node.label
                ));
            }
            let upstream_key = required_str(&source.config, "source_key")?;
            let own_key = required_str(&node.config, "source_key")?;
            let allowed = if node.kind == "structure" {
                source.kind == "source"
            } else {
                source.kind == "source" || source.kind == "structure"
            };
            if !allowed || upstream_key != own_key {
                return Err(format!(
                    "{} 须连接相同资料绑定的输入或结构整理节点",
                    node.label
                ));
            }
            if node.kind == "map"
                && source.kind == "source"
                && source
                    .config
                    .get("normalize_structure")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
                && company
                    .sources
                    .get(own_key)
                    .is_some_and(|binding| binding.layout.is_some())
            {
                return Err(format!(
                    "{} 的资料已启用复杂结构，请先连接结构整理节点",
                    node.label
                ));
            }
        }
        if node.kind == "join" && node.version == 1 {
            let left = required_str(&node.config, "left_id")?;
            let right = required_str(&node.config, "right_id")?;
            if left == right
                || !workflow
                    .edges
                    .iter()
                    .any(|e| e.source == left && e.target == node.id)
                || !workflow
                    .edges
                    .iter()
                    .any(|e| e.source == right && e.target == node.id)
            {
                return Err(format!("{} 须明确指定已连接的左侧和右侧节点", node.label));
            }
            for edge in incoming {
                if let Some(port) = edge.target_port.as_deref() {
                    if (port == "left" && edge.source != left)
                        || (port == "right" && edge.source != right)
                    {
                        return Err(format!(
                            "{} 的 {port} 输入连线与参数指定的左右节点不一致",
                            node.label
                        ));
                    }
                }
            }
            let issues: &[serde_json::Value] = match node.config.get("issue_sources") {
                Some(value) => value.as_array().ok_or("issue_sources 必须是数组")?,
                None => &[],
            };
            let mut unique_issues = BTreeSet::new();
            for issue in issues {
                let id = issue.as_str().ok_or("异常来源必须是节点 ID")?;
                if nodes.get(id).map(|n| n.kind.as_str()) != Some("decimal") {
                    return Err(format!(
                        "{} 的异常来源 {id} 必须指向金额转换节点",
                        node.label
                    ));
                }
                if !unique_issues.insert(id) {
                    return Err(format!("{} 的异常来源 {id} 重复，不能重复计数", node.label));
                }
                if !reaches(id, left, &next) && !reaches(id, right, &next) {
                    return Err(format!(
                        "{} 的异常来源 {id} 未连接到本次关联输入",
                        node.label
                    ));
                }
            }
        }
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut result = Vec::new();
    while let Some(id) = queue.pop_front() {
        result.push((*nodes[id]).clone());
        if let Some(children) = next.get(id) {
            for child in children {
                let count = indegree.get_mut(child).unwrap();
                *count -= 1;
                if *count == 0 {
                    queue.push_back(child);
                }
            }
        }
    }
    if result.len() != workflow.nodes.len() {
        return Err("工作流包含环路".into());
    }
    Ok(result)
}

pub fn required_str<'a>(config: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("缺少参数：{key}"))
}

pub fn required_strings(config: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    let values = config
        .get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("缺少数组参数：{key}"))?;
    values
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{key} 必须是文本数组"))
        })
        .collect()
}

#[cfg(test)]
mod removed_calculate_tests {
    use super::*;
    #[test]
    fn former_calculate_nodes_are_rejected_at_both_versions() {
        for version in [1, 2] {
            let workflow = serde_json::from_value(serde_json::json!({
                "schema_version": 1, "name": "legacy",
                "nodes": [{"id":"diff","kind":"calculate","version":version,"label":"旧差额",
                           "position":{"x":0,"y":0},"config":{}}],
                "edges": []
            })).unwrap();
            let company = CompanyConfig { schema_version:1, company:String::new(), sources:BTreeMap::new() };
            assert!(ordered_nodes(&workflow,&company).unwrap_err().contains("改用公式计算"));
        }
    }
}