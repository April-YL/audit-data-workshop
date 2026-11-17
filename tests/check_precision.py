"""A value outside DECIMAL(28,4) must stay traceable as an exception, not zero."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
CASE = PROJECT / "outputs" / "precision-case"
GOLDEN = PROJECT / "tests" / "golden"
OVERFLOW = "100000000000000000000000000000"


def call(binary: Path, *args: str) -> dict:
    result = subprocess.run(
        [str(binary), *args], capture_output=True, text=True,
        encoding="utf-8", check=True,
    )
    return json.loads(result.stdout)


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "debug" / "workshop-cli.exe"
    CASE.mkdir(parents=True, exist_ok=True)
    workflow = json.loads((GOLDEN / "workflow.json").read_text(encoding="utf-8"))
    company = json.loads((GOLDEN / "乙_company.json").read_text(encoding="utf-8"))
    source = Path(company["sources"]["JE"]["path"])
    with source.open(newline="", encoding="utf-8-sig") as handle:
        rows = list(csv.reader(handle))
    assert rows[1][0] == "6602"
    rows[1][1] = OVERFLOW
    target = CASE / "overflow_je.csv"
    with target.open("w", newline="", encoding="utf-8-sig") as handle:
        csv.writer(handle).writerows(rows)
    company["sources"]["JE"]["path"] = str(target)
    (CASE / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (CASE / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")

    run = call(binary, "run", str(CASE), str(CASE / "company.json"), str(CASE / "runs"))
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert reviews["je_decimal"]["rows"] == 1, reviews["je_decimal"]
    assert reviews["je_decimal"]["exceptions"] == 1, reviews["je_decimal"]
    bad = call(binary, "preview", run["database"], "n_je_decimal__exceptions", "10")
    assert bad["total_rows"] == 1
    exception = dict(zip(bad["columns"], bad["rows"][0]))
    assert exception["debit_raw"] == OVERFLOW, exception
    assert exception["debit"] == "", exception
    assert exception["__row"] == "2", exception
    assert "金额无法转换" in exception["__error"], exception
    with (Path(run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8-sig") as handle:
        output = {row["account_code"]: row for row in csv.DictReader(handle)}
    assert output["6602"]["left_amount"] == "", output["6602"]
    assert output["6602"]["right_amount"] == "40.0000", output["6602"]
    assert output["6603"]["left_amount"] == "-10.0000", output["6603"]

    fraction_case = CASE / "excess-fraction"
    fraction_case.mkdir(exist_ok=True)
    rows[1][1] = "40.00005"
    fractional_source = fraction_case / "fraction_je.csv"
    with fractional_source.open("w", newline="", encoding="utf-8-sig") as handle:
        csv.writer(handle).writerows(rows)
    company["sources"]["JE"]["path"] = str(fractional_source)
    (fraction_case / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (fraction_case / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    fraction_run = call(binary, "run", str(fraction_case), str(fraction_case / "company.json"), str(fraction_case / "runs"))
    fraction_bad = call(binary, "preview", fraction_run["database"], "n_je_decimal__exceptions", "10")
    assert fraction_bad["total_rows"] == 1, fraction_bad
    fraction_exception = dict(zip(fraction_bad["columns"], fraction_bad["rows"][0]))
    assert fraction_exception["debit_raw"] == "40.00005", fraction_exception
    assert fraction_exception["debit"] == "", fraction_exception
    assert "精度" in fraction_exception["__error"], fraction_exception

    exponent_case = CASE / "scientific-notation"
    exponent_case.mkdir(exist_ok=True)
    rows[1][1] = "4e1"
    exponent_source = exponent_case / "exponent_je.csv"
    with exponent_source.open("w", newline="", encoding="utf-8-sig") as handle:
        csv.writer(handle).writerows(rows)
    company["sources"]["JE"]["path"] = str(exponent_source)
    (exponent_case / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (exponent_case / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    exponent_run = call(binary, "run", str(exponent_case), str(exponent_case / "company.json"), str(exponent_case / "runs"))
    exponent_bad = call(binary, "preview", exponent_run["database"], "n_je_decimal__exceptions", "10")
    assert exponent_bad["total_rows"] == 1, exponent_bad
    exponent_exception = dict(zip(exponent_bad["columns"], exponent_bad["rows"][0]))
    assert exponent_exception["debit_raw"] == "4e1", exponent_exception
    assert exponent_exception["debit"] == "", exponent_exception
    assert "科学计数法" in exponent_exception["__error"], exponent_exception

    valid_case = CASE / "four-decimals"
    valid_case.mkdir(exist_ok=True)
    rows[1][1] = "40.0001"
    valid_source = valid_case / "valid_je.csv"
    with valid_source.open("w", newline="", encoding="utf-8-sig") as handle:
        csv.writer(handle).writerows(rows)
    company["sources"]["JE"]["path"] = str(valid_source)
    (valid_case / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (valid_case / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    valid_run = call(binary, "run", str(valid_case), str(valid_case / "company.json"), str(valid_case / "runs"))
    valid_reviews = {item["node_id"]: item for item in valid_run["reviews"]}
    assert valid_reviews["je_decimal"]["exceptions"] == 0, valid_reviews["je_decimal"]
    with (Path(valid_run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8-sig") as handle:
        valid_output = {row["account_code"]: row for row in csv.DictReader(handle)}
    assert valid_output["6602"]["difference"] == "0.0001", valid_output["6602"]
    print("Overflow, excess fraction, and scientific notation are exceptions; four-decimal differences stay exact.")


if __name__ == "__main__":
    main()
