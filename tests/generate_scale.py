"""Stream fictional CSV inputs for later single-machine scale checks.

Usage: python tests/generate_scale.py 10000
Outputs stay in data/synthetic/scale/<row-count>/; no customer data is read.
"""
from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]


def generate(rows: int) -> Path:
    if rows <= 0:
        raise ValueError("rows must be positive")
    folder = PROJECT / "data" / "synthetic" / "scale" / str(rows)
    folder.mkdir(parents=True, exist_ok=True)
    je_path = folder / "JE.csv"
    tb_path = folder / "TB.csv"
    with je_path.open("w", newline="", encoding="utf-8-sig") as je_stream, tb_path.open(
        "w", newline="", encoding="utf-8-sig"
    ) as tb_stream:
        je = csv.writer(je_stream)
        tb = csv.writer(tb_stream)
        header = ["account", "dr", "cr"]
        je.writerow(header)
        tb.writerow(header)
        for index in range(rows):
            record = ["6602", "2", "0"] if index % 2 == 0 else ["6603", "0", "1"]
            je.writerow(record)
            tb.writerow(record)
    even = (rows + 1) // 2
    odd = rows // 2
    expected = {
        "rows_per_file": rows,
        "company": "规模样例",
        "je_net": even * 2 - odd,
        "tb_net": even * 2 - odd,
        "groups": {"6602": even * 2, "6603": -odd},
        "difference": 0,
        "notes": "纯虚构同口径数据；每个文件含标题行，两个分组均应完全匹配。",
    }
    (folder / "expected.json").write_text(
        json.dumps(expected, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    fields = {"account_code": "account", "debit_raw": "dr", "credit_raw": "cr"}
    company = {
        "schema_version": 1,
        "company": expected["company"],
        "sources": {
            "JE": {"path": str(je_path), "fields": fields},
            "TB": {"path": str(tb_path), "fields": fields},
        },
    }
    (folder / "company.json").write_text(
        json.dumps(company, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    return folder


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rows", type=int, help="data rows per CSV file")
    args = parser.parse_args()
    print(generate(args.rows))
