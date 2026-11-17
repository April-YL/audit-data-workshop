"""Check that unsafe source and join inputs fail instead of producing results."""
from __future__ import annotations

import copy
import csv
import json
import subprocess
import sys
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
GOLDEN = PROJECT / "tests" / "golden"
OUT = PROJECT / "outputs" / "negative-cases"


def write_case(name: str, workflow: dict, company: dict) -> Path:
    folder = OUT / name
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (folder / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    return folder


def must_fail(binary: Path, name: str, workflow: dict, company: dict, expected: str) -> None:
    folder = write_case(name, workflow, company)
    result = subprocess.run(
        [str(binary), "run", str(folder), str(folder / "company.json"), str(folder / "runs")],
        capture_output=True, text=True, encoding="utf-8",
    )
    assert result.returncode != 0, f"{name}: should have failed"
    assert expected in result.stderr, (name, result.stderr)
    print(f"{name}: rejected with {expected}")


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "debug" / "workshop-cli.exe"
    workflow = json.loads((GOLDEN / "workflow.json").read_text(encoding="utf-8"))
    company = json.loads((GOLDEN / "甲_company.json").read_text(encoding="utf-8"))

    missing = copy.deepcopy(company)
    missing["sources"]["JE"]["fields"]["account_code"] = "不存在的科目列"
    must_fail(binary, "missing-column", workflow, missing, "不存在的科目列")

    empty = copy.deepcopy(company)
    source = Path(company["sources"]["TB"]["path"])
    with source.open(newline="", encoding="utf-8-sig") as handle:
        rows = list(csv.reader(handle))
    rows[1][0] = ""
    target = OUT / "empty-key" / "tb.csv"
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("w", newline="", encoding="utf-8-sig") as handle:
        csv.writer(handle).writerows(rows)
    empty["sources"]["TB"]["path"] = str(target)
    must_fail(binary, "empty-key", workflow, empty, "匹配键为空")

    disconnected = copy.deepcopy(workflow)
    disconnected["edges"] = [
        edge for edge in disconnected["edges"]
        if not (edge["source"] == "tb_group" and edge["target"] == "compare")
    ]
    must_fail(binary, "missing-join-input", disconnected, company, "按公司与科目核对 需要 2 条输入连线，实际 1 条")

    unrelated = copy.deepcopy(workflow)
    unrelated_company = copy.deepcopy(company)
    unrelated_company["sources"]["OTHER"] = copy.deepcopy(company["sources"]["JE"])
    base = next(node for node in workflow["nodes"] if node["id"] == "je_decimal")
    for node_id, kind, config, x in [
        ("other_source", "source", {"source_key": "OTHER"}, 0),
        ("other_map", "map", {"source_key": "OTHER", "fields": ["account_code", "debit_raw", "credit_raw"]}, 200),
        ("other_decimal", "decimal", copy.deepcopy(base["config"]), 400),
    ]:
        unrelated["nodes"].append({
            "id": node_id, "kind": kind, "version": 1, "label": node_id,
            "position": {"x": x, "y": 650}, "config": config,
        })
    unrelated["edges"].extend([
        {"id": "other_e1", "source": "other_source", "target": "other_map"},
        {"id": "other_e2", "source": "other_map", "target": "other_decimal"},
    ])
    join = next(node for node in unrelated["nodes"] if node["id"] == "compare")
    join["config"]["issue_sources"].append("other_decimal")
    must_fail(binary, "unrelated-exception-source", unrelated, unrelated_company, "异常来源 other_decimal 未连接到本次关联输入")

    repeated_issue = copy.deepcopy(workflow)
    join = next(node for node in repeated_issue["nodes"] if node["id"] == "compare")
    join["config"]["issue_sources"].append("je_decimal")
    must_fail(binary, "duplicate-exception-source", repeated_issue, company, "异常来源 je_decimal 重复")

    wrong_port = copy.deepcopy(workflow)
    edge = next(e for e in wrong_port["edges"] if e["source"] == "je_group" and e["target"] == "compare")
    edge["target_port"] = "right"
    must_fail(binary, "join-port-mismatch", wrong_port, company, "right 输入连线与参数指定的左右节点不一致")

    duplicate_port = copy.deepcopy(workflow)
    for edge in duplicate_port["edges"]:
        if edge["target"] == "compare":
            edge["target_port"] = "left"
    must_fail(binary, "duplicate-join-port", duplicate_port, company, "输入端口 left 重复连接")

    malformed_issue = copy.deepcopy(workflow)
    join = next(node for node in malformed_issue["nodes"] if node["id"] == "compare")
    join["config"]["issue_sources"] = "je_decimal"
    folder = write_case("malformed-exception-source", malformed_issue, company)
    result = subprocess.run(
        [str(binary), "validate", str(folder), str(folder / "company.json")],
        capture_output=True, text=True, encoding="utf-8",
    )
    assert result.returncode != 0, "malformed-exception-source: should fail preflight"
    assert "issue_sources 必须是数组" in result.stderr, result.stderr
    print("malformed-exception-source: rejected during preflight")

    duplicate = copy.deepcopy(workflow)
    for edge in duplicate["edges"]:
        if edge["source"] == "je_group" and edge["target"] == "compare":
            edge["source"] = "je_filter"
    join = next(node for node in duplicate["nodes"] if node["id"] == "compare")
    join["config"]["left_id"] = "je_filter"
    join["config"]["left_amount"] = "net"
    must_fail(binary, "duplicate-key", duplicate, company, "重复键组")


if __name__ == "__main__":
    main()
