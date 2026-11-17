"""Generate fictional audit data and reusable workflow fixtures. No customer data."""
from __future__ import annotations

import csv
import json
from pathlib import Path
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZipFile

PROJECT = Path(__file__).resolve().parents[1]
DATA = PROJECT / "data" / "synthetic" / "golden"
FLOW = PROJECT / "tests" / "golden"


def xlsx(path: Path, sheet: str, rows: list[list[str]]) -> None:
    def colname(index: int) -> str:
        result = ""
        while index:
            index, remainder = divmod(index - 1, 26)
            result = chr(65 + remainder) + result
        return result

    cells = []
    for rownum, row in enumerate(rows, 1):
        parts = []
        for colnum, value in enumerate(row, 1):
            reference = f"{colname(colnum)}{rownum}"
            parts.append(f'<c r="{reference}" t="inlineStr"><is><t>{escape(value)}</t></is></c>')
        cells.append(f'<row r="{rownum}">{"".join(parts)}</row>')
    worksheet = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{"".join(cells)}</sheetData></worksheet>'
    workbook = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{escape(sheet)}" sheetId="1" r:id="rId1"/></sheets></workbook>'
    with ZipFile(path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        archive.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        archive.writestr("xl/workbook.xml", workbook)
        archive.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        archive.writestr("xl/worksheets/sheet1.xml", worksheet)


def csv_file(path: Path, rows: list[list[str]]) -> None:
    with path.open("w", newline="", encoding="utf-8-sig") as stream:
        csv.writer(stream).writerows(rows)


def make_node(node_id: str, kind: str, label: str, x: int, y: int, config: dict) -> dict:
    return {"id": node_id, "kind": kind, "version": 1, "label": label, "position": {"x": x, "y": y}, "config": config}


def main() -> None:
    DATA.mkdir(parents=True, exist_ok=True)
    FLOW.mkdir(parents=True, exist_ok=True)
    xlsx(DATA / "甲_JE.xlsx", "JE", [
        ["科目编码", "借方金额", "贷方金额"], ["6602", "100", "0"], ["6602", "0", "20"],
        ["6603", "50", "0"], ["6602", "错误", "0"], ["1001", "5", "0"],
    ])
    csv_file(DATA / "乙_JE.csv", [["account", "dr", "cr"], ["6602", "40", "0"], ["6603", "0", "10"]])
    csv_file(DATA / "甲_TB.csv", [["科目编码", "借方金额", "贷方金额"], ["6602", "100", "20"], ["6604", "12", "0"]])
    csv_file(DATA / "乙_TB.csv", [["account", "dr", "cr"], ["6602", "40", "0"], ["6603", "0", "10"]])
    nodes = [
        make_node("je_source", "source", "JE 输入", 0, 0, {"source_key": "JE"}),
        make_node("je_map", "map", "JE 字段映射", 200, 0, {"source_key": "JE", "fields": ["account_code", "debit_raw", "credit_raw"]}),
        make_node("je_decimal", "decimal", "JE 金额转换", 400, 0, {"fields": [{"source": "debit_raw", "target": "debit"}, {"source": "credit_raw", "target": "credit"}]}),
        make_node("je_net", "derive", "JE 净额", 600, 0, {"expression": "[debit]-[credit]", "target": "net"}),
        make_node("je_mark", "mark", "科目标记", 800, 0, {"field": "account_code", "target": "selected", "values": ["6602", "6603"]}),
        make_node("je_filter", "filter", "目标科目", 1000, 0, {"field": "selected"}),
        make_node("je_group", "aggregate", "JE 汇总", 1200, 0, {"keys": ["company", "account_code"], "amount": "net", "target": "amount"}),
        make_node("tb_source", "source", "TB 输入", 0, 250, {"source_key": "TB"}),
        make_node("tb_map", "map", "TB 字段映射", 200, 250, {"source_key": "TB", "fields": ["account_code", "debit_raw", "credit_raw"]}),
        make_node("tb_decimal", "decimal", "TB 金额转换", 400, 250, {"fields": [{"source": "debit_raw", "target": "debit"}, {"source": "credit_raw", "target": "credit"}]}),
        make_node("tb_net", "derive", "TB 净额", 600, 250, {"expression": "[debit]-[credit]", "target": "net"}),
        make_node("tb_group", "aggregate", "TB 汇总", 800, 250, {"keys": ["company", "account_code"], "amount": "net", "target": "amount"}),
        make_node("compare", "join", "按公司与科目核对", 1400, 125, {"left_id": "je_group", "right_id": "tb_group", "keys": ["company", "account_code"], "left_amount": "amount", "right_amount": "amount", "issue_sources": ["je_decimal", "tb_decimal"]}),
        make_node("check", "inspect", "结果检查", 1600, 125, {}),
        make_node("output", "export", "导出核对表", 1800, 125, {}),
    ]
    pairs = [("je_source", "je_map"), ("je_map", "je_decimal"), ("je_decimal", "je_net"), ("je_net", "je_mark"), ("je_mark", "je_filter"), ("je_filter", "je_group"), ("tb_source", "tb_map"), ("tb_map", "tb_decimal"), ("tb_decimal", "tb_net"), ("tb_net", "tb_group"), ("je_group", "compare"), ("tb_group", "compare"), ("compare", "check"), ("check", "output")]
    workflow = {"schema_version": 1, "name": "通用 JE/TB 对拍组合", "nodes": nodes, "edges": [{"id": f"e{i}", "source": a, "target": b} for i, (a, b) in enumerate(pairs)]}
    (FLOW / "workflow.json").write_text(json.dumps(workflow, ensure_ascii=False, indent=2), encoding="utf-8")
    for company in ["甲", "乙"]:
        fields = {"account_code": "科目编码", "debit_raw": "借方金额", "credit_raw": "贷方金额"} if company == "甲" else {"account_code": "account", "debit_raw": "dr", "credit_raw": "cr"}
        je = {"path": str(DATA / f"{company}_JE.{'xlsx' if company == '甲' else 'csv'}"), "fields": fields}
        if company == "甲":
            je["sheet"] = "JE"
        tb = {"path": str(DATA / f"{company}_TB.csv"), "fields": fields}
        config = {"schema_version": 1, "company": company, "sources": {"JE": je, "TB": tb}}
        (FLOW / f"{company}_company.json").write_text(json.dumps(config, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Generated synthetic fixtures in {DATA} and workflow in {FLOW}")


if __name__ == "__main__":
    main()
