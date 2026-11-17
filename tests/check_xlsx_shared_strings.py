"""Measure fictional XLSX files with one unique shared-string memo per JE row.

Usage: python tests/check_xlsx_shared_strings.py <workshop-cli.exe> [rows]
"""
from __future__ import annotations

import csv
import json
import sys
import time
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

from check_scale import run_with_memory

PROJECT = Path(__file__).resolve().parents[1]
FLOW = PROJECT / "tests" / "golden"
DATA = PROJECT / "data" / "synthetic" / "xlsx-shared-strings"
OUTPUT = PROJECT / "outputs" / "xlsx-shared-strings-runs"


def create_case(rows: int) -> Path:
    if rows < 21 or rows > 1_048_575:
        raise ValueError("本规模样例需要 21 至 1,048,575 条数据行")
    folder = DATA / str(rows)
    folder.mkdir(parents=True, exist_ok=True)
    je_path = folder / "JE.xlsx"
    with ZipFile(je_path, "w", ZIP_DEFLATED, compresslevel=1) as archive:
        archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>')
        archive.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="JE" sheetId="1" r:id="rId1"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>')
        with archive.open("xl/sharedStrings.xml", "w", force_zip64=True) as strings:
            strings.write((f'<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{4 + rows * 4}" uniqueCount="{7 + rows}">').encode())
            for value in ("account", "dr", "cr", "memo", "6602", "1", "0"):
                strings.write(f"<si><t>{value}</t></si>".encode())
            for index in range(rows):
                strings.write(f"<si><t>unique-memo-{index:08d}</t></si>".encode())
            strings.write(b"</sst>")
        with archive.open("xl/worksheets/sheet1.xml", "w", force_zip64=True) as sheet:
            sheet.write(b'<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>')
            for number in range(1, rows + 2):
                indexes = (0, 1, 2, 3) if number == 1 else (4, 5, 6, number + 5)
                cells = b"".join(f'<c r="{column}{number}" t="s"><v>{index}</v></c>'.encode()
                                 for column, index in zip("ABCD", indexes))
                sheet.write(b'<row r="' + str(number).encode() + b'">' + cells + b'</row>')
            sheet.write(b'</sheetData></worksheet>')

    tb_path = folder / "TB.csv"
    with tb_path.open("w", newline="", encoding="utf-8") as stream:
        csv.writer(stream).writerows([["account", "dr", "cr"], ["6602", str(rows), "0"]])
    fields = {"account_code": "account", "debit_raw": "dr", "credit_raw": "cr"}
    company = {"schema_version": 1, "company": "共享字符串规模样例", "sources": {
        "JE": {"path": str(je_path), "sheet": "JE", "fields": fields},
        "TB": {"path": str(tb_path), "fields": fields}}}
    (folder / "company.json").write_text(json.dumps(company, ensure_ascii=False, indent=2), encoding="utf-8")
    return folder


def check(binary: Path, rows: int) -> dict:
    folder = create_case(rows)
    path = folder / "JE.xlsx"
    started = time.perf_counter()
    preview, preview_peak = run_with_memory(binary, "inspect", str(path), "JE", "20")
    preview_time = time.perf_counter() - started
    assert preview["headers"] == ["account", "dr", "cr", "memo"]
    assert preview["rows"][0] == ["6602", "1", "0", "unique-memo-00000000"]
    assert preview["rows"][-1][-1] == "unique-memo-00000019"
    assert preview["physical_rows"] == list(range(2, 22)) and preview["truncated"]
    started = time.perf_counter()
    run, run_peak = run_with_memory(binary, "run", str(FLOW), str(folder / "company.json"), str(OUTPUT))
    run_time = time.perf_counter() - started
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert reviews["je_source"]["rows"] == rows
    assert reviews["je_decimal"]["exceptions"] == 0
    assert reviews["compare"]["rows"] == reviews["compare"]["matched"] == 1
    assert reviews["compare"]["unresolved"] == 0
    with (Path(run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8") as stream:
        result = list(csv.DictReader(stream))
    assert len(result) == 1
    assert result[0]["left_amount"] == result[0]["right_amount"] == f"{rows}.0000"
    assert result[0]["difference"] == "0.0000"
    metrics = {"xlsx_rows": rows, "shared_strings": rows + 7,
               "xlsx_bytes": path.stat().st_size, "preview_seconds": round(preview_time, 3),
               "preview_peak_bytes": preview_peak, "run_seconds": round(run_time, 3),
               "run_peak_bytes": run_peak,
               "checks": "共享字符串预览、全量导入、金额和差额对拍通过；单次本机资源观察"}
    (Path(run["run_dir"]) / "xlsx_shared_metrics.json").write_text(
        json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8")
    return metrics


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    print(json.dumps(check(Path(sys.argv[1]), int(sys.argv[2]) if len(sys.argv) == 3 else 100_000),
                     ensure_ascii=False, indent=2))
