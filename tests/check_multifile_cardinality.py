"""Generate fictional mixed-header JE files and check every high-cardinality output row.

Usage: python tests/check_multifile_cardinality.py <workshop-cli.exe> [groups]
All inputs and expected values are synthetic; groups defaults to 20,000.
"""
from __future__ import annotations

import csv
import json
import subprocess
import sys
import time
from contextlib import ExitStack
from decimal import Decimal
from pathlib import Path

from check_scale import run_with_memory

PROJECT = Path(__file__).resolve().parents[1]
DATA = PROJECT / "data" / "synthetic" / "multifile-cardinality"
OUTPUT = PROJECT / "outputs" / "multifile-cardinality"


def node(identifier: str, kind: str, config: dict, x: int, y: int) -> dict:
    return {"id": identifier, "kind": kind, "version": 1, "label": identifier,
            "position": {"x": x, "y": y}, "config": config}


def create_case(groups: int) -> Path:
    if groups < 4:
        raise ValueError("至少需要 4 个科目组")
    folder = DATA / str(groups)
    folder.mkdir(parents=True, exist_ok=True)
    sources: dict[str, dict] = {}
    nodes: list[dict] = []
    edges: list[dict] = []

    def edge(source: str, target: str, port: str | None = None) -> None:
        item = {"id": f"e{len(edges)}", "source": source, "target": target}
        if port:
            item["target_port"] = port
        edges.append(item)

    with ExitStack() as stack:
        je_writers = []
        for part in range(4):
            role = f"JE{part}"
            path = folder / f"{role}.csv"
            file = stack.enter_context(path.open("w", newline="", encoding="utf-8"))
            writer = csv.writer(file)
            header = (["科\"目,编码", "借方", "贷方", "摘要"] if part % 2 == 0
                      else ["acct", "dr", "cr", "memo"])
            writer.writerow(header)
            je_writers.append(writer)
            sources[role] = {"path": str(path), "fields": dict(zip(
                ["account_code", "debit_raw", "credit_raw"], header[:3]))}
            nodes.extend([
                node(f"{role}_source", "source", {"source_key": role}, 0, part * 180),
                node(f"{role}_map", "map", {"source_key": role,
                     "fields": ["account_code", "debit_raw", "credit_raw"]}, 180, part * 180),
            ])
            edge(f"{role}_source", f"{role}_map")

        tb_path = folder / "TB.csv"
        tb_file = stack.enter_context(tb_path.open("w", newline="", encoding="utf-8"))
        tb = csv.writer(tb_file)
        tb.writerow(["code", "debit", "credit"])
        for index in range(groups):
            code = f"A{index:08d}"
            memo = "跨行\n摘要" if index == 1 else "虚构业务"
            je_writers[index % 4].writerow([code, "2", "0", memo])
            je_writers[index % 4].writerow([code, "0", "1", "冲回"])
            if index != 0:
                tb.writerow([code, "2" if index % 997 == 0 else "1", "0"])
        tb.writerow(["Z_ONLY_TB", "3", "0"])

    sources["TB"] = {"path": str(tb_path), "fields": {
        "account_code": "code", "debit_raw": "debit", "credit_raw": "credit"}}
    nodes.extend([
        node("union01", "union", {}, 380, 90),
        node("union23", "union", {}, 380, 450),
        node("union_all", "union", {}, 580, 270),
        node("je_decimal", "decimal", {"fields": [
            {"source": "debit_raw", "target": "debit"},
            {"source": "credit_raw", "target": "credit"}]}, 760, 270),
        node("je_net", "derive", {"expression": "[debit]-[credit]", "target": "net"}, 940, 270),
        node("je_group", "aggregate", {"keys": ["company", "account_code"],
             "amount": "net", "target": "amount"}, 1120, 270),
        node("tb_source", "source", {"source_key": "TB"}, 0, 800),
        node("tb_map", "map", {"source_key": "TB",
             "fields": ["account_code", "debit_raw", "credit_raw"]}, 180, 800),
        node("tb_decimal", "decimal", {"fields": [
            {"source": "debit_raw", "target": "debit"},
            {"source": "credit_raw", "target": "credit"}]}, 380, 800),
        node("tb_net", "derive", {"expression": "[debit]-[credit]", "target": "net"}, 580, 800),
        node("tb_group", "aggregate", {"keys": ["company", "account_code"],
             "amount": "net", "target": "amount"}, 760, 800),
        node("compare", "join", {"left_id": "je_group", "right_id": "tb_group",
             "keys": ["company", "account_code"], "left_amount": "amount",
             "right_amount": "amount", "issue_sources": ["je_decimal", "tb_decimal"]}, 1320, 450),
        node("check", "inspect", {}, 1500, 450),
        node("output", "export", {}, 1680, 450),
    ])
    # Deliberately save right-port edges before left-port edges. The engine
    # must interpret saved ports rather than incidental JSON list order.
    edge("JE1_map", "union01", "right")
    edge("JE0_map", "union01", "left")
    edge("JE3_map", "union23", "right")
    edge("JE2_map", "union23", "left")
    edge("union23", "union_all", "right")
    edge("union01", "union_all", "left")
    for source, target in [
        ("union_all", "je_decimal"), ("je_decimal", "je_net"),
        ("je_net", "je_group"), ("tb_source", "tb_map"),
        ("tb_map", "tb_decimal"), ("tb_decimal", "tb_net"),
        ("tb_net", "tb_group"),
    ]:
        edge(source, target)
    edge("tb_group", "compare", "right")
    edge("je_group", "compare", "left")
    edge("compare", "check")
    edge("check", "output")
    workflow = {"schema_version": 1, "name": "虚构异构多文件高基数对拍",
                "nodes": nodes, "edges": edges}
    company = {"schema_version": 1, "company": "虚构集团", "sources": sources}
    (folder / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False, indent=2), encoding="utf-8")
    (folder / "company.json").write_text(json.dumps(company, ensure_ascii=False, indent=2), encoding="utf-8")
    return folder


def call(binary: Path, *args: str) -> dict:
    process = subprocess.run([str(binary), *args], capture_output=True, text=True,
                             encoding="utf-8", check=True)
    return json.loads(process.stdout)


def check(binary: Path, groups: int) -> dict:
    folder = create_case(groups)
    call(binary, "validate", str(folder), str(folder / "company.json"))
    started = time.perf_counter()
    run, peak = run_with_memory(binary, "run", str(folder), str(folder / "company.json"), str(OUTPUT))
    elapsed = time.perf_counter() - started
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert set(run["input_fingerprints"]) == {"JE0", "JE1", "JE2", "JE3", "TB"}
    assert reviews["union_all"]["rows"] == 2 * groups
    assert reviews["je_group"]["rows"] == groups
    assert reviews["tb_group"]["rows"] == groups
    assert reviews["compare"]["rows"] == groups + 1
    assert reviews["compare"]["left_only"] == reviews["compare"]["right_only"] == 1
    assert reviews["je_decimal"]["exceptions"] == reviews["tb_decimal"]["exceptions"] == 0

    seen: set[str] = set()
    total_difference = Decimal(0)
    with (Path(run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8") as stream:
        for row in csv.DictReader(stream):
            code = row["account_code"]
            assert code not in seen, code
            seen.add(code)
            assert row["company"] == "虚构集团"
            if code == "Z_ONLY_TB":
                expected_left, expected_right, status = "", "3.0000", "仅右侧"
            else:
                index = int(code[1:])
                assert code == f"A{index:08d}" and index < groups
                expected_left = "1.0000"
                expected_right = "" if index == 0 else ("2.0000" if index % 997 == 0 else "1.0000")
                status = "仅左侧" if index == 0 else ("金额不符" if index % 997 == 0 else "数值相符")
            assert (row["left_amount"], row["right_amount"], row["match_status"]) == (
                expected_left, expected_right, status), (code, row)
            assert row["issue_count"] == "0"
            expected_difference = Decimal(expected_left or "0") - Decimal(expected_right or "0")
            assert Decimal(row["difference"]) == expected_difference, code
            total_difference += expected_difference
    assert len(seen) == groups + 1
    assert total_difference == Decimal(-2 - (groups - 1) // 997)

    key = json.dumps({"company": "虚构集团", "account_code": "A00000001"}, ensure_ascii=False)
    traced = call(binary, "trace", str(folder / "workflow.json"), run["database"], "compare", key)
    assert len(traced) >= 2
    assert isinstance(traced, list)
    left_preview = traced[0]["preview"]
    left_rows = left_preview["rows"]
    assert len(left_rows) == 2
    left_columns = left_preview["columns"]
    left_details = [dict(zip(left_columns, row)) for row in left_rows]
    assert {row["__role"] for row in left_details} == {"JE1"}
    assert {row["__file"] for row in left_details} == {str(folder / "JE1.csv")}
    assert {(row["__row"], row["__record"]) for row in left_details} == {("2", "1"), ("4", "2")}
    metrics = {"groups": groups, "je_rows": 2 * groups, "tb_rows": groups,
               "result_rows": groups + 1, "difference_total": str(total_difference),
               "elapsed_seconds": round(elapsed, 3), "memory_peak_bytes": peak,
               "run_disk_bytes": sum(f.stat().st_size for f in Path(run["run_dir"]).rglob("*") if f.is_file()),
               "checks": "五文件异构表头、端口逆序、逐行完整金额、未匹配、差异与来源均通过"}
    (Path(run["run_dir"]) / "multifile_metrics.json").write_text(
        json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8")
    return metrics


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    print(json.dumps(check(Path(sys.argv[1]), int(sys.argv[2]) if len(sys.argv) == 3 else 20_000),
                     ensure_ascii=False, indent=2))
