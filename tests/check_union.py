"""Verify generic append preserves rows and source identity for two similar files."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
CASE = PROJECT / "outputs" / "union-test"


def node(id: str, kind: str, config: dict, x: int, y: int) -> dict:
    return {
        "id": id, "kind": kind, "version": 1, "label": id,
        "position": {"x": x, "y": y}, "config": config,
    }


def call(binary: Path, *args: str) -> dict:
    result = subprocess.run(
        [str(binary), *args], capture_output=True, text=True, encoding="utf-8", check=True,
    )
    return json.loads(result.stdout)


def check_type_mismatch(binary: Path) -> None:
    """A numeric amount must not silently turn into text during append."""
    type_case = CASE / "type-mismatch"
    type_case.mkdir(parents=True, exist_ok=True)
    for filename, headers, row in [
        ("numeric.csv", ["account", "raw"], ["6602", "10"]),
        ("text.csv", ["account", "raw", "net", "error"], ["6602", "10", "10", ""]),
    ]:
        with (type_case / filename).open("w", newline="", encoding="utf-8") as handle:
            writer = csv.writer(handle)
            writer.writerow(headers)
            writer.writerow(row)
    company = {
        "schema_version": 1, "company": "虚构集团",
        "sources": {
            "NUM": {"path": str(type_case / "numeric.csv"), "fields": {"account_code": "account", "amount_raw": "raw"}},
            "TEXT": {"path": str(type_case / "text.csv"), "fields": {"account_code": "account", "amount_raw": "raw", "net": "net", "__error": "error"}},
        },
    }
    workflow = {
        "schema_version": 1, "name": "类型不一致应拒绝追加",
        "nodes": [
            node("num_source", "source", {"source_key": "NUM"}, 0, 0),
            node("num_map", "map", {"source_key": "NUM", "fields": ["account_code", "amount_raw"]}, 100, 0),
            node("to_decimal", "decimal", {"fields": [{"source": "amount_raw", "target": "net"}]}, 200, 0),
            node("txt_source", "source", {"source_key": "TEXT"}, 0, 200),
            node("txt_map", "map", {"source_key": "TEXT", "fields": ["account_code", "amount_raw", "net", "__error"]}, 200, 200),
            node("append", "union", {}, 300, 100),
            node("export", "export", {}, 400, 100),
        ],
        "edges": [
            {"id": f"e{i}", "source": source, "target": target}
            for i, (source, target) in enumerate([
                ("num_source", "num_map"), ("num_map", "to_decimal"),
                ("txt_source", "txt_map"), ("to_decimal", "append"),
                ("txt_map", "append"), ("append", "export"),
            ])
        ],
    }
    # Serialize the right edge before the left one. Explicit canvas ports,
    # not JSON edge order, must determine the sides named in the error.
    workflow["edges"][3]["target_port"] = "left"
    workflow["edges"][4]["target_port"] = "right"
    workflow["edges"][3], workflow["edges"][4] = workflow["edges"][4], workflow["edges"][3]
    (type_case / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (type_case / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    result = subprocess.run(
        [str(binary), "run", str(type_case), str(type_case / "company.json"), str(type_case / "runs")],
        capture_output=True, text=True, encoding="utf-8",
    )
    assert result.returncode != 0, result.stdout
    assert "字段 net 类型不一致" in result.stderr, result.stderr
    assert "左侧 DECIMAL(28,4)，右侧 VARCHAR" in result.stderr, result.stderr


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "debug" / "workshop-cli.exe"
    CASE.mkdir(parents=True, exist_ok=True)
    samples = {
        "A": [("6602", "10", "0"), ("6603", "0", "2")],
        "B": [("6602", "7", "0"), ("6604", "0", "3")],
    }
    headers = {
        "A": ["科\"目", "借,方", "贷方"],
        "B": ["account", "dr", "cr"],
    }
    sources = {}
    for role, rows in samples.items():
        path = CASE / f"{role}.csv"
        with path.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.writer(handle)
            writer.writerow(headers[role])
            writer.writerows(rows)
        sources[role] = {
            "path": str(path),
            "fields": dict(zip(["account_code", "debit_raw", "credit_raw"], headers[role])),
        }
    workflow = {
        "schema_version": 1, "name": "同类资料追加验证",
        "nodes": [
            node("a_source", "source", {"source_key": "A"}, 0, 0),
            node("a_map", "map", {"source_key": "A", "fields": ["account_code", "debit_raw", "credit_raw"]}, 200, 0),
            node("b_source", "source", {"source_key": "B"}, 0, 200),
            node("b_map", "map", {"source_key": "B", "fields": ["account_code", "debit_raw", "credit_raw"]}, 200, 200),
            node("append", "union", {}, 400, 100),
            node("export", "export", {}, 600, 100),
        ],
        "edges": [
            {"id": f"e{i}", "source": source, "target": target}
            for i, (source, target) in enumerate([
                ("a_source", "a_map"), ("b_source", "b_map"),
                ("a_map", "append"), ("b_map", "append"), ("append", "export"),
            ])
        ],
    }
    sources["UNUSED"] = {"path": str(CASE / "not-used.csv"), "fields": {}}
    company = {"schema_version": 1, "company": "虚构集团", "sources": sources}
    (CASE / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (CASE / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    run = call(binary, "run", str(CASE), str(CASE / "company.json"), str(CASE / "runs"))
    assert set(run["input_fingerprints"]) == {"A", "B"}
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert reviews["append"]["rows"] == 4
    preview = call(binary, "preview", run["database"], "n_append", "10")
    actual = [dict(zip(preview["columns"], row)) for row in preview["rows"]]
    assert preview["total_rows"] == 4
    assert {(r["__role"], r["account_code"], r["debit_raw"], r["credit_raw"]) for r in actual} == {
        ("A", "6602", "10", "0"), ("A", "6603", "0", "2"),
        ("B", "6602", "7", "0"), ("B", "6604", "0", "3"),
    }
    for row in actual:
        assert row["__file"] == str(CASE / f"{row['__role']}.csv")
        assert (row["__row"], row["__record"]) in {("2", "1"), ("3", "2")}
    check_type_mismatch(binary)
    print("Generic append passed: four rows and both source files retained.")


if __name__ == "__main__":
    main()
