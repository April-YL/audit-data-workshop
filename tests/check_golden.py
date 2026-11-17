"""Run the fictional golden case against the compiled Rust CLI."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from decimal import Decimal
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
FLOW = PROJECT / "tests" / "golden"
OUTPUT = PROJECT / "outputs" / "golden-runs"


def call(binary: Path, *args: str):
    result = subprocess.run([str(binary), *args], capture_output=True, text=True, encoding="utf-8", check=True)
    return json.loads(result.stdout)


def records(preview: dict) -> list[dict[str, str]]:
    return [dict(zip(preview["columns"], values)) for values in preview["rows"]]


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "debug" / "workshop-cli.exe"
    saved = PROJECT / "outputs" / "golden-roundtrip" / "shared-template"
    subprocess.run(
        [str(binary), "save-flow", str(FLOW), str(saved)],
        capture_output=True, text=True, encoding="utf-8", check=True,
    )
    restored_workflow = call(binary, "load-flow", str(saved))
    assert restored_workflow == json.loads((saved / "workflow.json").read_text(encoding="utf-8"))
    original_workflow = json.loads((FLOW / "workflow.json").read_text(encoding="utf-8"))
    assert len(restored_workflow["nodes"]) == len(original_workflow["nodes"])
    assert len(restored_workflow["edges"]) == len(original_workflow["edges"])
    assert [(n["id"], n["kind"], n["config"]) for n in restored_workflow["nodes"]] == [
        (n["id"], n["kind"], n["config"]) for n in original_workflow["nodes"]
    ]
    template_bytes = (saved / "workflow.json").read_bytes()
    config_files = {}
    for company in ["甲", "乙"]:
        config_file = saved / "companies" / f"{company}.json"
        subprocess.run(
            [str(binary), "save-company", str(FLOW / f"{company}_company.json"), str(config_file)],
            capture_output=True, text=True, encoding="utf-8", check=True,
        )
        restored_company = call(binary, "load-company", str(config_file))
        assert restored_company == json.loads(config_file.read_text(encoding="utf-8"))
        assert restored_company["company"] == company
        config_files[company] = config_file
    assert call(binary, "load-company", str(config_files["甲"]))["company"] == "甲"
    assert (saved / "workflow.json").read_bytes() == template_bytes
    first_je = call(binary, "inspect", str(PROJECT / "data" / "synthetic" / "golden" / "甲_JE.xlsx"), "JE", "2")
    assert first_je["headers"] == ["科目编码", "借方金额", "贷方金额"]
    assert first_je["physical_rows"] == [2, 3] and first_je["truncated"]
    expected = {
        "甲": {"6602": ("80.0000", "80.0000", "0.0000", "数值相符但待复核"), "6603": ("50.0000", "", "50.0000", "仅左侧"), "6604": ("", "12.0000", "-12.0000", "仅右侧")},
        "乙": {"6602": ("40.0000", "40.0000", "0.0000", "数值相符"), "6603": ("-10.0000", "-10.0000", "0.0000", "数值相符")},
    }
    totals = {"甲": Decimal("38"), "乙": Decimal("0")}
    for company in ["甲", "乙"]:
        run = call(
            binary, "run",
            str(saved),
            str(config_files[company]),
            str(OUTPUT),
        )
        reviews = {item["node_id"]: item for item in run["reviews"]}
        db = run["database"]
        result = records(call(binary, "preview", db, "n_compare", "50"))
        assert len(result) == len(expected[company]), (company, result)
        for row in result:
            account = row["account_code"]
            actual = (row["left_amount"], row["right_amount"], row["difference"], row["match_status"])
            assert actual == expected[company][account], (company, account, actual)
        assert sum((Decimal(row["difference"]) for row in result), Decimal()) == totals[company]
        if company == "甲":
            assert reviews["je_source"]["rows"] == 5
            assert reviews["je_decimal"]["rows"] == 4 and reviews["je_decimal"]["exceptions"] == 1
            assert reviews["je_filter"]["rows"] == 3 and reviews["je_filter"]["excluded"] == 1
            bad = records(call(binary, "preview", db, "n_je_decimal__exceptions", "50"))
            assert bad[0]["__row"] == "5" and bad[0]["debit_raw"] == "错误" and bad[0]["__sheet"] == "JE"
            ignored = records(call(binary, "preview", db, "n_je_filter__excluded", "50"))
            assert ignored[0]["__row"] == "6" and ignored[0]["account_code"] == "1001"
            group = json.dumps({"company": "甲", "account_code": "6602"}, ensure_ascii=False)
            trace = call(binary, "trace", str(FLOW / "workflow.json"), db, "compare", group)
            assert [part["preview"]["total_rows"] for part in trace] == [2, 1, 1, 0], trace
            assert {r["__row"] for r in records(trace[0]["preview"])} == {"2", "3"}
            assert {r["__record"] for r in records(trace[0]["preview"])} == {"1", "2"}
        else:
            assert reviews["je_source"]["rows"] == 2 and reviews["je_decimal"]["exceptions"] == 0
            csv_sample = call(binary, "inspect", str(PROJECT / "data" / "synthetic" / "golden" / "乙_JE.csv"), "-", "20")
            assert csv_sample["headers"] == ["account", "dr", "cr"]
            assert csv_sample["physical_rows"] == [2, 3] and not csv_sample["truncated"]
            plain_rows = records(call(binary, "preview", db, "n_je_source", "10"))
            assert [(row["__row"], row["__record"]) for row in plain_rows] == [("2", "1"), ("3", "2")]
        print(f"{company}: {len(result)} groups, difference {totals[company]}, source and exception controls passed")
    multiline_file = saved / "multiline_je.csv"
    with multiline_file.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["account", "dr", "cr", "摘要"])
        writer.writerow(["6602", "40", "0", "第一行\n第二行"])
        writer.writerow(["6603", "0", "10", "正常"])
    multiline_company = json.loads(config_files["乙"].read_text(encoding="utf-8"))
    multiline_company["sources"]["JE"]["path"] = str(multiline_file)
    multiline_config = saved / "multiline_company.json"
    multiline_config.write_text(json.dumps(multiline_company, ensure_ascii=False), encoding="utf-8")
    multiline_sample = call(binary, "inspect", str(multiline_file), "-", "1")
    assert multiline_sample["physical_rows"] == [2] and multiline_sample["truncated"]
    assert multiline_sample["rows"][0][3] == "第一行\n第二行"
    multiline_run = call(binary, "run", str(saved), str(multiline_config), str(OUTPUT))
    source_rows = records(call(binary, "preview", multiline_run["database"], "n_je_source", "10"))
    assert [(row["__row"], row["__record"]) for row in source_rows] == [("2", "1"), ("4", "2")]
    print("Golden case passed; the two companies were run separately with the same workflow.")


if __name__ == "__main__":
    main()
