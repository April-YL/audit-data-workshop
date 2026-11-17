"""Golden check for a title row, merged multi-level headers, and merged data cells."""
from __future__ import annotations

import csv
import json
import subprocess
import sys
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

PROJECT = Path(__file__).resolve().parents[1]
CASE = PROJECT / "data" / "synthetic" / "structure"
OUTPUT = PROJECT / "outputs" / "structure-test"


def run_cli(binary: Path, *args: str) -> dict:
    result = subprocess.run([str(binary), *args], capture_output=True, text=True,
                            encoding="utf-8", check=True)
    return json.loads(result.stdout)


def make_xlsx(path: Path) -> None:
    sheet = '''<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>虚构科目余额资料</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>科目</t></is></c><c r="B2" t="inlineStr"><is><t>本期发生额</t></is></c><c r="D2" t="inlineStr"><is><t>备注</t></is></c></row>
<row r="3"><c r="B3" t="inlineStr"><is><t>借方</t></is></c><c r="C3" t="inlineStr"><is><t>贷方</t></is></c></row>
<row r="4"><c r="A4" t="inlineStr"><is><t>6602</t></is></c><c r="B4"><v>10</v></c><c r="C4"><v>0</v></c></row>
<row r="5"><c r="B5"><v>5</v></c><c r="C5"><v>0</v></c></row>
<row r="6"><c r="A6" t="inlineStr"><is><t>6603</t></is></c><c r="B6"><v>0</v></c><c r="C6"><v>3</v></c></row>
<row r="7"><c r="B7"><v>1</v></c><c r="C7"><v>0</v></c></row>
<row r="8"><c r="B8"><v>2</v></c><c r="C8"><v>0</v></c></row>
<row r="9"><c r="A9" t="inlineStr"><is><t>以下为说明，不属于明细</t></is></c></row>
</sheetData><mergeCells count="6"><mergeCell ref="A1:D1"/><mergeCell ref="A2:A3"/><mergeCell ref="B2:C2"/><mergeCell ref="A4:A5"/><mergeCell ref="A6:A7"/><mergeCell ref="A9:D9"/></mergeCells></worksheet>'''
    with ZipFile(path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        archive.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="TB" sheetId="1" r:id="rId1"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        archive.writestr("xl/worksheets/sheet1.xml", sheet)


def node(id: str, kind: str, config: dict, x: int) -> dict:
    return {"id": id, "kind": kind, "version": 1, "label": id,
            "position": {"x": x, "y": 0}, "config": config}


def main() -> None:
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else PROJECT / "outputs" / "cargo-target" / "debug" / "workshop-cli.exe"
    CASE.mkdir(parents=True, exist_ok=True)
    path = CASE / "merged.xlsx"
    make_xlsx(path)
    company = {"schema_version": 1, "company": "虚构公司", "sources": {"TB": {
        "path": str(path), "sheet": "TB",
        "layout": {"first_col": 1, "last_col": 4, "header_rows": [2, 3], "data_start_row": 4, "data_end_row": 8,
                   "fill_merged_data_cols": [1]},
        "fields": {"account_code": "科目", "debit_raw": "本期发生额 / 借方",
                   "credit_raw": "本期发生额 / 贷方"}
    }}}
    workflow = {"schema_version": 1, "name": "结构整理黄金样例", "nodes": [
        node("source", "source", {"source_key": "TB"}, 0),
        node("structure", "structure", {"source_key": "TB"}, 240),
        node("map", "map", {"source_key": "TB", "fields": ["account_code", "debit_raw", "credit_raw"]}, 480),
        node("export", "export", {}, 720),
    ], "edges": [{"id": f"e{i}", "source": a, "target": b} for i, (a, b) in enumerate([
        ("source", "structure"), ("structure", "map"), ("map", "export")
    ])]}
    (CASE / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False), encoding="utf-8")
    (CASE / "company.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    run = run_cli(binary, "run", str(CASE), str(CASE / "company.json"), str(OUTPUT))
    preview = run_cli(binary, "preview", run["database"], "n_structure", "20")
    rows = [dict(zip(preview["columns"], item)) for item in preview["rows"]]
    assert preview["total_rows"] == 5, rows
    assert [(row["__row"], row["__record"], row["科目"], row["本期发生额 / 借方"], row["本期发生额 / 贷方"]) for row in rows] == [
        ("4", "1", "6602", "10", "0"), ("5", "2", "6602", "5", "0"),
        ("6", "3", "6603", "0", "3"), ("7", "4", "6603", "1", "0"),
        ("8", "5", "", "2", "0"),
    ], rows
    assert [row["科目__filled_from_merge"] for row in rows] == ["false", "true", "false", "true", "false"], rows
    source = run_cli(binary, "preview", run["database"], "n_source", "20")
    raw = [dict(zip(source["columns"], item)) for item in source["rows"]]
    assert raw[4]["col_1"] == "", "Original blank inside merged A5 must remain traceable"
    with (Path(run["run_dir"]) / "export.csv").open(newline="", encoding="utf-8") as handle:
        exported = list(csv.DictReader(handle))
    assert len(exported) == 5 and exported[1]["account_code"] == "6602"
    assert exported[4]["account_code"] == "", "Unmerged blank A8 must not be filled"
    company["sources"]["TB"]["layout"]["data_end_row"] = 6
    (CASE / "company_cut_merge.json").write_text(json.dumps(company, ensure_ascii=False), encoding="utf-8")
    invalid = subprocess.run([str(binary), "run", str(CASE), str(CASE / "company_cut_merge.json"), str(OUTPUT)],
                             capture_output=True, text=True, encoding="utf-8")
    assert invalid.returncode != 0 and "跨越数据边界" in invalid.stderr, invalid.stderr
    print("Structure golden passed: two header rows, lower data merge, footer excluded, source blanks retained.")


if __name__ == "__main__":
    main()
