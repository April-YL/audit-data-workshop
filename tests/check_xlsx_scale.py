"""Measure bounded XLSX preview and full import with fictional JE/TB controls.

Usage: python tests/check_xlsx_scale.py <workshop-cli.exe> [JE rows]
The generated XLSX contains inline strings, so shared-string-heavy files are
outside this case. Output stays in data/synthetic and outputs.
"""
from __future__ import annotations

import csv
import json
import subprocess
import sys
import time
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

from check_scale import run_with_memory

PROJECT = Path(__file__).resolve().parents[1]
FLOW = PROJECT / "tests" / "golden"
DATA = PROJECT / "data" / "synthetic" / "xlsx-scale"
OUTPUT = PROJECT / "outputs" / "xlsx-scale-runs"


def create_xlsx(path: Path, rows: int) -> None:
    def cell(column: str, row: int, value: str) -> bytes:
        return (f'<c r="{column}{row}" t="inlineStr"><is><t>{value}</t></is></c>').encode()

    with ZipFile(path, "w", ZIP_DEFLATED, compresslevel=1) as archive:
        archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        archive.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="JE" sheetId="1" r:id="rId1"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        with archive.open("xl/worksheets/sheet1.xml", "w", force_zip64=True) as sheet:
            sheet.write(b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>')
            sheet.write(b'<row r="1">' + cell("A", 1, "account") + cell("B", 1, "dr") + cell("C", 1, "cr") + b'</row>')
            for number in range(2, rows + 2):
                sheet.write(b'<row r="' + str(number).encode() + b'">' +
                            cell("A", number, "6602") + cell("B", number, "1") +
                            cell("C", number, "0") + b'</row>')
            sheet.write(b'</sheetData></worksheet>')


def create_case(rows: int) -> Path:
    if rows < 1 or rows > 1_048_575:
        raise ValueError("Excel 数据行数须在 1 至 1,048,575 之间")
    folder = DATA / str(rows)
    folder.mkdir(parents=True, exist_ok=True)
    je_path = folder / "JE.xlsx"
    tb_path = folder / "TB.csv"
    create_xlsx(je_path, rows)
    with tb_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream)
        writer.writerows([["account", "dr", "cr"], ["6602", str(rows), "0"]])
    fields = {"account_code": "account", "debit_raw": "dr", "credit_raw": "cr"}
    company = {"schema_version": 1, "company": "Excel规模样例", "sources": {
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
    assert preview["headers"] == ["account", "dr", "cr"]
    assert len(preview["rows"]) == min(rows, 20)
    assert preview["physical_rows"] == list(range(2, min(rows, 20) + 2))
    assert preview["truncated"] == (rows > 20)
    started = time.perf_counter()
    run, run_peak = run_with_memory(binary, "run", str(FLOW), str(folder / "company.json"), str(OUTPUT))
    run_time = time.perf_counter() - started
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert reviews["je_source"]["rows"] == rows
    assert reviews["je_decimal"]["exceptions"] == 0
    assert reviews["je_filter"]["excluded"] == 0
    assert reviews["compare"]["rows"] == reviews["compare"]["matched"] == 1
    assert reviews["compare"]["unresolved"] == 0
    assert set(run["input_fingerprints"]) == {"JE", "TB"}
    with (Path(run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8") as stream:
        records = list(csv.DictReader(stream))
    assert len(records) == 1
    record = records[0]
    assert record["account_code"] == "6602"
    assert record["left_amount"] == record["right_amount"] == f"{rows}.0000"
    assert record["difference"] == "0.0000" and record["match_status"] == "数值相符"
    metrics = {"xlsx_rows": rows, "xlsx_bytes": path.stat().st_size,
               "preview_seconds": round(preview_time, 3), "preview_peak_bytes": preview_peak,
               "run_seconds": round(run_time, 3), "run_peak_bytes": run_peak,
               "run_disk_bytes": sum(f.stat().st_size for f in Path(run["run_dir"]).rglob("*") if f.is_file()),
               "checks": "局部预览行号、全量行数、金额、差额、来源指纹均通过；仅测内联字符串 XLSX"}
    (Path(run["run_dir"]) / "xlsx_metrics.json").write_text(
        json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8")
    return metrics


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    print(json.dumps(check(Path(sys.argv[1]), int(sys.argv[2]) if len(sys.argv) == 3 else 100_000),
                     ensure_ascii=False, indent=2))
