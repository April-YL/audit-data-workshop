"""Check reusable text extraction against a small synthetic subject column."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
CASE = PROJECT / "outputs" / "text-extract-test"


def command(binary: Path, *args: str) -> dict:
    result = subprocess.run([str(binary), *args], capture_output=True, text=True,
                            encoding="utf-8", check=True)
    return json.loads(result.stdout)


def node(id: str, kind: str, config: dict, x: int) -> dict:
    return {"id": id, "kind": kind, "version": 1, "label": id,
            "position": {"x": x, "y": 0}, "config": config}


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "release" / "workshop-cli.exe"
    CASE.mkdir(parents=True, exist_ok=True)
    source = CASE / "subjects.csv"
    values = ["6602 管理费用", "6603-销售费用", "12345 研发费用", "不规范", ""]
    with source.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["科目完整名称"])
        writer.writerows([[value] for value in values])
    company = {"schema_version": 1, "company": "虚构企业", "sources": {
        "SUBJECTS": {"path": str(source), "fields": {"subject_raw": "科目完整名称"}}
    }}
    pattern = r"^([0-9]{4,6})[[:space:]-]+(.+)$"
    workflow = {"schema_version": 1, "name": "文本提取通用操作验证", "nodes": [
        node("source", "source", {"source_key": "SUBJECTS"}, 0),
        node("map", "map", {"source_key": "SUBJECTS", "fields": ["subject_raw"]}, 240),
        node("code", "text_extract", {"source": "subject_raw", "target": "account_code", "pattern": pattern, "group": 1}, 480),
        node("name", "text_extract", {"source": "subject_raw", "target": "account_name", "pattern": pattern, "group": 2}, 720),
        node("export", "export", {}, 960),
    ], "edges": [{"id": f"e{i}", "source": a, "target": b} for i, (a, b) in enumerate([
        ("source", "map"), ("map", "code"), ("code", "name"), ("name", "export")
    ])]}
    (CASE / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (CASE / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    run = command(binary, "run", str(CASE), str(CASE / "company.json"), str(CASE / "runs"))
    review = {item["node_id"]: item for item in run["reviews"]}
    assert review["code"]["rows"] == 5 and review["code"]["exceptions"] == 2, review["code"]
    assert review["name"]["rows"] == 5 and review["name"]["exceptions"] == 2, review["name"]
    preview = command(binary, "preview", run["database"], "n_name", "20")
    rows = [dict(zip(preview["columns"], row)) for row in preview["rows"]]
    assert preview["total_rows"] == 5
    assert [(r["account_code"], r["account_name"]) for r in rows] == [
        ("6602", "管理费用"), ("6603", "销售费用"), ("12345", "研发费用"), ("", ""), ("", "")
    ], rows
    assert [r["__row"] for r in rows] == ["2", "3", "4", "5", "6"], rows
    exceptions = command(binary, "preview", run["database"], "n_name__exceptions", "20")
    assert exceptions["total_rows"] == 2
    assert {row[exceptions["columns"].index("subject_raw")] for row in exceptions["rows"]} == {"不规范", ""}
    print("Text extraction passed: 5 source rows retained, 3 matched, 2 flagged in exception table.")


if __name__ == "__main__":
    main()
