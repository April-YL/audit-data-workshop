"""Check sparse XLSX row/column positions against the original workbook coordinates."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

PROJECT = Path(__file__).resolve().parents[1]
CASE = PROJECT / "data" / "synthetic" / "xlsx-positions"
OUTPUT = PROJECT / "outputs" / "xlsx-positions"


def call(binary: Path, *args: str) -> dict:
    result = subprocess.run([str(binary), *args], capture_output=True, text=True,
                            encoding="utf-8", check=True)
    return json.loads(result.stdout)


def make_case() -> Path:
    CASE.mkdir(parents=True, exist_ok=True)
    file = CASE / "sparse.xlsx"
    sheet_xml = b'''<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="3"><c r="B3" t="inlineStr"><is><t>account</t></is></c><c r="C3" t="inlineStr"><is><t>dr</t></is></c><c r="D3" t="inlineStr"><is><t>cr</t></is></c></row>
<row r="4"><c r="B4" t="inlineStr"><is><t>6602</t></is></c><c r="C4" t="inlineStr"><is><t>10</t></is></c><c r="D4" t="inlineStr"><is><t>0</t></is></c></row>
<row r="6"><c r="B6" t="inlineStr"><is><t>6603</t></is></c><c r="C6" t="inlineStr"><is><t>0</t></is></c><c r="D6" t="inlineStr"><is><t>2</t></is></c></row>
</sheetData></worksheet>'''
    with ZipFile(file, "w", ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        archive.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="JE" sheetId="1" r:id="rId1"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        archive.writestr("xl/worksheets/sheet1.xml", sheet_xml)
    workflow = {"schema_version": 1, "name": "Excel 坐标检查", "nodes": [
        {"id": "source", "kind": "source", "label": "输入", "position": {"x": 0, "y": 0},
         "config": {"source_key": "JE"}},
        {"id": "output", "kind": "export", "label": "导出", "position": {"x": 200, "y": 0},
         "config": {}}],
        "edges": [{"id": "e0", "source": "source", "target": "output"}]}
    company = {"schema_version": 1, "company": "坐标样例", "sources": {
        "JE": {"path": str(file), "sheet": "JE", "fields": {}}}}
    (CASE / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (CASE / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    return file


def check(binary: Path) -> None:
    file = make_case()
    preview = call(binary, "inspect", str(file), "JE", "3")
    assert preview["headers"] == ["account", "dr", "cr"]
    assert preview["physical_rows"] == [4, 5, 6]
    assert preview["rows"] == [["6602", "10", "0"], ["", "", ""], ["6603", "0", "2"]]
    assert preview["truncated"] is False
    single = call(binary, "inspect", str(file), "JE", "1")
    assert single["physical_rows"] == [4] and single["truncated"] is True
    run = call(binary, "run", str(CASE), str(CASE / "company.json"), str(OUTPUT))
    with (Path(run["run_dir"]) / "output.csv").open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    assert [(row["__row"], row["__record"], row["account"]) for row in rows] == [
        ("4", "1", "6602"), ("5", "2", ""), ("6", "3", "6603")]
    print("Sparse XLSX preview and import retained B3 header, empty row 5, and original row numbers.")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    check(Path(sys.argv[1]))
