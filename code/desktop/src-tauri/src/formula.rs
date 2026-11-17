//! A small expression language compiled to DuckDB SQL. No user SQL is executed.
use crate::{engine::lit, field_contract, model::FieldDescriptor, workflow::sql_name};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Field(String),
    Text(String),
    Num(String),
    Word(String),
    Op(String),
    L,
    R,
    Comma,
}
#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Text,
    Number,
    Bool,
    Date,
}
#[derive(Clone, Debug)]
pub struct Compiled {
    pub sql: String,
    pub error: String,
    ty: Ty,
    pub referenced_fields: Vec<String>,
    uses_row_offset: bool,
}
impl Compiled {
    pub fn data_type(&self) -> &str {
        match self.ty {
            Ty::Text => "VARCHAR",
            Ty::Number => "DECIMAL(28,4)",
            Ty::Bool => "BOOLEAN",
            Ty::Date => "DATE",
        }
    }
}
fn tokens(s: &str) -> Result<Vec<Token>, String> {
    if s.len() > 4096 {
        return Err("公式超过4096字节".into());
    }
    let chars = s.chars().collect::<Vec<_>>();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '[' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                if i == chars.len() {
                    return Err("字段引用缺少 ]".into());
                }
                out.push(Token::Field(chars[start..i].iter().collect()));
                i += 1;
            }
            '"' => {
                i += 1;
                let mut v = String::new();
                loop {
                    if i >= chars.len() {
                        return Err("文字缺少结束双引号".into());
                    }
                    if chars[i] == '"' {
                        i += 1;
                        if i < chars.len() && chars[i] == '"' {
                            v.push('"');
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        v.push(chars[i]);
                        i += 1;
                    }
                }
                out.push(Token::Text(v));
            }
            '(' => {
                out.push(Token::L);
                i += 1;
            }
            ')' => {
                out.push(Token::R);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '+' | '-' | '*' | '/' | '&' | '=' | '<' | '>' => {
                let mut op = c.to_string();
                i += 1;
                if i < chars.len()
                    && ((c == '<' && (chars[i] == '>' || chars[i] == '='))
                        || (c == '>' && chars[i] == '='))
                {
                    op.push(chars[i]);
                    i += 1;
                }
                out.push(Token::Op(op));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let v = chars[start..i].iter().collect::<String>();
                field_contract::decimal_literal(&v)?;
                out.push(Token::Num(v));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Word(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .to_ascii_uppercase(),
                ));
            }
            _ => return Err(format!("公式字符不支持：{c}；字段请用[字段名]")),
        }
        if out.len() > 256 {
            return Err("公式过于复杂，请拆成多个步骤".into());
        }
    }
    Ok(out)
}
struct Parser<'a> {
    t: Vec<Token>,
    i: usize,
    fields: &'a [FieldDescriptor],
    refs: BTreeSet<String>,
    depth: usize,
    order: &'a str,
    uses_row_offset: bool,
}
fn item(sql: String, ty: Ty, error: String) -> Compiled {
    Compiled {
        sql,
        error,
        ty,
        referenced_fields: vec![],
        uses_row_offset: false,
    }
}
fn ok(sql: String, ty: Ty) -> Compiled {
    item(sql, ty, "NULL::VARCHAR".into())
}
fn merged(a: &Compiled, b: &Compiled) -> String {
    format!("COALESCE({}, {})", a.error, b.error)
}
fn text(e: &Compiled) -> String {
    format!("CAST(({}) AS VARCHAR)", e.sql)
}
// Amounts use scaled integers, including division. No floating-point intermediate.
fn scaled(e: &Compiled) -> String {
    format!("TRY_CAST(({}) * 10000 AS HUGEINT)", e.sql)
}
fn unscale(s: &str) -> String {
    format!("TRY_CAST((CASE WHEN ({s})<0 THEN '-' ELSE '' END)||CAST(abs(({s}) // 10000) AS VARCHAR)||'.'||lpad(CAST(abs(({s}) % 10000) AS VARCHAR),4,'0') AS DECIMAL(28,4))")
}
impl Parser<'_> {
    fn expr(&mut self, min: u8) -> Result<Compiled, String> {
        self.depth += 1;
        if self.depth > 16 {
            return Err("公式嵌套超过16层".into());
        }
        let mut a = self.atom()?;
        loop {
            let op = match self.t.get(self.i) {
                Some(Token::Op(v)) => v.clone(),
                _ => break,
            };
            let p = match op.as_str() {
                "=" | "<>" | "<" | ">" | "<=" | ">=" => 1,
                "&" => 2,
                "+" | "-" => 3,
                "*" | "/" => 4,
                _ => 0,
            };
            if p < min {
                break;
            }
            self.i += 1;
            let b = self.expr(p + 1)?;
            a = self.binary(a, &op, b)?;
            if a.sql.len() + a.error.len() > 500_000 {
                return Err("编译后的公式过大，请拆分步骤".into());
            }
        }
        self.depth -= 1;
        Ok(a)
    }
    fn atom(&mut self) -> Result<Compiled, String> {
        let token = self.t.get(self.i).cloned().ok_or("公式不完整")?;
        self.i += 1;
        match token {
            Token::Num(n) => Ok(ok(
                format!("CAST({} AS DECIMAL(28,4))", lit(&n)),
                Ty::Number,
            )),
            Token::Text(v) => Ok(ok(lit(&v), Ty::Text)),
            Token::Field(name) => {
                let f = self
                    .fields
                    .iter()
                    .find(|f| f.name == name && !name.starts_with("__"))
                    .ok_or_else(|| format!("引用字段不存在：{name}"))?;
                self.refs.insert(name.clone());
                let ty = if f.data_type == "BOOLEAN" {
                    Ty::Bool
                } else if f.data_type == "DATE" {
                    Ty::Date
                } else if field_contract::numeric(&f.data_type) {
                    if matches!(f.data_type.as_str(), "DOUBLE" | "FLOAT" | "REAL") {
                        return Err("公式金额不接受浮点字段，请先明确转换精度".into());
                    }
                    Ty::Number
                } else {
                    Ty::Text
                };
                if ty == Ty::Number {
                    let sql = format!("TRY_CAST({} AS DECIMAL(28,4))", sql_name(&name));
                    let original = sql_name(&name);
                    let error=format!("CASE WHEN {original} IS NOT NULL AND (({sql}) IS NULL OR ({original})<>({sql})) THEN '数值超范围或超过四位精度' END");
                    Ok(item(
                        format!("CASE WHEN ({error}) IS NULL THEN {sql} END"),
                        ty,
                        error,
                    ))
                } else {
                    Ok(ok(sql_name(&name), ty))
                }
            }
            Token::Op(op) if op == "-" || op == "+" => {
                let e = self.expr(5)?;
                if e.ty != Ty::Number {
                    return Err("正负号需要数值".into());
                }
                Ok(item(format!("({op}({}))", e.sql), e.ty, e.error))
            }
            Token::L => {
                let e = self.expr(1)?;
                self.expect(Token::R)?;
                Ok(e)
            }
            Token::Word(w) if matches!(w.as_str(), "PREV" | "NEXT") => self.row_offset(&w),
            Token::Word(w) if w == "TRUE" || w == "FALSE" => Ok(ok(w, Ty::Bool)),
            Token::Word(w) => {
                self.expect(Token::L)?;
                let mut args = Vec::new();
                if self.t.get(self.i) != Some(&Token::R) {
                    loop {
                        args.push(self.expr(1)?);
                        if self.t.get(self.i) == Some(&Token::Comma) {
                            self.i += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Token::R)?;
                self.call(&w, args)
            }
            _ => Err("公式需要字段、常量或函数".into()),
        }
    }
    fn row_key(&mut self) -> Result<String, String> {
        let Some(Token::Field(name)) = self.t.get(self.i) else {
            return Err("跨行排序和分组须引用字段".into());
        };
        if name.starts_with("__") || !self.fields.iter().any(|f| f.name == *name) {
            return Err(format!("跨行字段不存在：{name}"));
        }
        self.i += 1;
        self.refs.insert(name.clone());
        Ok(sql_name(name))
    }

    fn row_offset(&mut self, method: &str) -> Result<Compiled, String> {
        self.expect(Token::L)?;
        if !matches!(self.t.get(self.i), Some(Token::Field(_))) {
            return Err("跨行取值的第一个参数须为字段".into());
        }
        let value = self.atom()?;
        self.expect(Token::Comma)?;
        let distance = match self.t.get(self.i) {
            Some(Token::Num(n)) => n.parse::<u32>().ok().filter(|n| (1..=10000).contains(n)),
            _ => None,
        }.ok_or("跨行距离须为 1 至 10000 的整数")?;
        self.i += 1;
        self.expect(Token::Comma)?;
        let order = self.row_key()?;
        let mut groups = Vec::new();
        while self.t.get(self.i) == Some(&Token::Comma) {
            self.i += 1;
            groups.push(self.row_key()?);
        }
        self.expect(Token::R)?;
        self.uses_row_offset = true;
        let partition = if groups.is_empty() { String::new() } else {
            format!("PARTITION BY {} ", groups.join(","))
        };
        let window = format!("OVER ({partition}ORDER BY {order} ASC NULLS LAST, {})", self.order);
        let name = if method == "PREV" { "LAG" } else { "LEAD" };
        let shifted = format!("{name}(({}),{distance}) {window}", value.sql);
        let error = format!("CASE WHEN {name}(1,{distance}) {window} IS NULL THEN '组内没有目标行' ELSE {name}(({}),{distance}) {window} END", value.error);
        Ok(item(shifted, value.ty, error))
    }

    fn expect(&mut self, t: Token) -> Result<(), String> {
        if self.t.get(self.i) != Some(&t) {
            return Err(format!("公式缺少 {t:?}"));
        }
        self.i += 1;
        Ok(())
    }
    fn binary(&self, a: Compiled, op: &str, b: Compiled) -> Result<Compiled, String> {
        let error = merged(&a, &b);
        if op == "&" {
            return Ok(item(
                format!("({} || {})", text(&a), text(&b)),
                Ty::Text,
                error,
            ));
        }
        if ["=", "<>", "<", ">", "<=", ">="].contains(&op) {
            if a.ty != b.ty {
                return Err("比较两侧类型不同，请明确转换".into());
            }
            return Ok(item(
                format!("(({}) {op} ({}))", a.sql, b.sql),
                Ty::Bool,
                error,
            ));
        }
        if a.ty != Ty::Number || b.ty != Ty::Number {
            return Err("四则运算只接受数值；文本金额请先转换".into());
        }
        let x = scaled(&a);
        let y = scaled(&b);
        let (integer, bad) = match op {
            "+" | "-" => (format!("TRY(({x}) {op} ({y}))"), "FALSE".into()),
            "*" => {
                let p = format!("TRY(({x}) * ({y}))");
                (
                    format!("({p}) // 10000"),
                    format!("({p}) IS NULL OR ({p}) % 10000 <> 0"),
                )
            }
            "/" => {
                let p = format!("TRY(({x}) * 10000)");
                (
                    format!("({p}) // NULLIF(({y}),0)"),
                    format!("({y})=0 OR ({p}) % NULLIF(({y}),0) <> 0"),
                )
            }
            _ => return Err("不支持的运算符".into()),
        };
        let sql = unscale(&integer);
        let e=format!("COALESCE({error}, CASE WHEN ({}) IS NOT NULL AND ({}) IS NOT NULL AND (({bad}) OR ({sql}) IS NULL) THEN '除零、超精度或数值溢出' END)",a.sql,b.sql);
        Ok(item(
            format!("CASE WHEN ({e}) IS NULL THEN {sql} END"),
            Ty::Number,
            e,
        ))
    }
    fn call(&self, name: &str, a: Vec<Compiled>) -> Result<Compiled, String> {
        let arity = match name {
            "ABS" | "VALUE" | "YEAR" | "MONTH" | "DAY" | "TOTEXT" => 1,
            "ROUND" | "DATEVALUE" | "TEXT" | "DATEDIFF" => 2,
            "DATE" => 3,
            "AND" | "OR" if a.len() >= 2 => a.len(),
            "IF" | "MID" | "SUBSTITUTE" => 3,
            "LEFT" | "RIGHT" | "FIND" => 2,
            "LEN" | "TRIM" | "ISBLANK" => 1,
            "TEXTBEFORE" | "TEXTAFTER" => {
                if a.len() == 2 {
                    2
                } else {
                    3
                }
            }
            _ => return Err(format!("不支持的函数：{name}")),
        };
        if a.len() != arity {
            return Err(format!("{name}需要{arity}个参数"));
        }
        if name == "IF" {
            if a[0].ty != Ty::Bool || a[1].ty != a[2].ty {
                return Err("IF需要布尔条件及同类型的两个结果".into());
            }
            return Ok(item(
                format!(
                    "CASE WHEN ({}) IS TRUE THEN ({}) ELSE ({}) END",
                    a[0].sql, a[1].sql, a[2].sql
                ),
                a[1].ty.clone(),
                format!(
                    "COALESCE({},CASE WHEN ({}) IS TRUE THEN ({}) ELSE ({}) END)",
                    a[0].error, a[0].sql, a[1].error, a[2].error
                ),
            ));
        }
        let errors = format!(
            "COALESCE({},NULL::VARCHAR)",
            a.iter()
                .map(|v| v.error.clone())
                .collect::<Vec<_>>()
                .join(",")
        );
        let s = text(&a[0]);
        if name == "TOTEXT" {
            return Ok(item(s, Ty::Text, errors));
        }
        if name == "AND" || name == "OR" {
            if a.iter().any(|value| value.ty != Ty::Bool) {
                return Err("AND和OR需要布尔条件".into());
            }
            return Ok(item(
                format!(
                    "({})",
                    a.iter()
                        .map(|value| format!("({})", value.sql))
                        .collect::<Vec<_>>()
                        .join(if name == "AND" { " AND " } else { " OR " })
                ),
                Ty::Bool,
                errors,
            ));
        }
        if ["ABS", "ROUND"].contains(&name) {
            if a[0].ty != Ty::Number {
                return Err("数学函数需要数值字段".into());
            }
            let sql = if name == "ABS" {
                format!("ABS({})", a[0].sql)
            } else {
                integer_arg(&a[1])?;
                format!("CASE WHEN ({}) BETWEEN 0 AND 4 AND ({0})=TRUNC({0}) THEN ROUND({},TRY_CAST(({}) AS INTEGER)) END",a[1].sql,a[0].sql,a[1].sql)
            };
            let sql=format!("TRY_CAST(({sql}) AS DECIMAL(28,4))");
            return Ok(item(sql.clone(),Ty::Number,format!("COALESCE({errors},CASE WHEN ({}) IS NOT NULL AND ({sql}) IS NULL THEN '舍入位数须为0至4的整数，金额不可超出范围' END)",a[0].sql)));
        }
        if name == "VALUE" {
            let valid = format!(
                "regexp_full_match(TRIM({s}),'[+-]?([0-9]+([.][0-9]{{0,4}})?|[.][0-9]{{1,4}})')"
            );
            let sql = format!("CASE WHEN {valid} THEN TRY_CAST(TRIM({s}) AS DECIMAL(28,4)) END");
            return Ok(item(sql.clone(),Ty::Number,format!("COALESCE({errors},CASE WHEN ({}) IS NOT NULL AND ({sql}) IS NULL THEN '文本无法按四位精度转换数值' END)",a[0].sql)));
        }
        if name == "DATEVALUE" || name == "TEXT" {
            let format=match a[1].sql.as_str(){
                "'yyyy-mm-dd'"=>"%Y-%m-%d","'yyyy/mm/dd'"=>"%Y/%m/%d","'yyyymmdd'"=>"%Y%m%d",
                "'dd/mm/yyyy'"=>"%d/%m/%Y","'mm/dd/yyyy'"=>"%m/%d/%Y",
                _=>return Err("日期格式须明确写为双引号内的yyyy-mm-dd、yyyy/mm/dd、yyyymmdd、dd/mm/yyyy或mm/dd/yyyy".into()),
            };
            let (sql, ty) = if name == "DATEVALUE" {
                if a[0].ty != Ty::Text {
                    return Err("DATEVALUE的第一个参数须为文本".into());
                }
                let parsed = format!("CAST(TRY_STRPTIME({s},{}) AS DATE)", lit(format));
                (
                    format!(
                        "CASE WHEN STRFTIME({parsed},{})={s} THEN {parsed} END",
                        lit(format)
                    ),
                    Ty::Date,
                )
            } else {
                if a[0].ty != Ty::Date {
                    return Err("TEXT目前用于明确日期格式化".into());
                }
                (format!("STRFTIME({}, {})", a[0].sql, lit(format)), Ty::Text)
            };
            return Ok(item(sql.clone(),ty,format!("COALESCE({errors},CASE WHEN ({}) IS NOT NULL AND ({sql}) IS NULL THEN '日期与指定格式不符或日期无效' END)",a[0].sql)));
        }
        if ["YEAR", "MONTH", "DAY", "DATEDIFF"].contains(&name) {
            if a.iter().any(|value| value.ty != Ty::Date) {
                return Err("日期计算需要日期类型，请先用DATEVALUE指定格式转换".into());
            }
            let sql = if name == "DATEDIFF" {
                format!("DATE_DIFF('day',{}, {})", a[0].sql, a[1].sql)
            } else {
                format!("{}({})", name, a[0].sql)
            };
            return Ok(item(
                format!("CAST(({sql}) AS DECIMAL(28,4))"),
                Ty::Number,
                errors,
            ));
        }
        if name == "DATE" {
            for value in &a {
                integer_arg(value)?;
            }
            let sql=format!("TRY(MAKE_DATE(TRY_CAST(({}) AS BIGINT),TRY_CAST(({}) AS BIGINT),TRY_CAST(({}) AS BIGINT)))",a[0].sql,a[1].sql,a[2].sql);
            let valid = a
                .iter()
                .map(|value| format!("({})=TRUNC({0})", value.sql))
                .collect::<Vec<_>>()
                .join(" AND ");
            let sql = format!("CASE WHEN {valid} THEN {sql} END");
            return Ok(item(sql.clone(),Ty::Date,format!("COALESCE({errors},CASE WHEN ({}) IS NOT NULL AND ({}) IS NOT NULL AND ({}) IS NOT NULL AND ({sql}) IS NULL THEN '年月日须为有效日期的整数' END)",a[0].sql,a[1].sql,a[2].sql)));
        }
        if name == "ISBLANK" {
            return Ok(item(
                format!("({} IS NULL OR {s}='')", a[0].sql),
                Ty::Bool,
                errors,
            ));
        }
        let (sql, ty, invalid) = match name {
            "TRIM" => (
                format!("trim({s},' '||chr(9)||chr(10)||chr(13))"),
                Ty::Text,
                "FALSE".into(),
            ),
            "LEN" => (
                format!("CAST(length({s}) AS DECIMAL(28,4))"),
                Ty::Number,
                "FALSE".into(),
            ),
            "LEFT" | "RIGHT" => {
                integer_arg(&a[1])?;
                let n = &a[1].sql;
                (
                    format!(
                        "{}({s},GREATEST(0,TRY_CAST(({n}) AS BIGINT)))",
                        name.to_ascii_lowercase()
                    ),
                    Ty::Text,
                    format!("({n})<0 OR ({n})<>trunc({n}) OR TRY_CAST(({n}) AS BIGINT) IS NULL"),
                )
            }
            "MID" => {
                integer_arg(&a[1])?;
                integer_arg(&a[2])?;
                let start = &a[1].sql;
                let len = &a[2].sql;
                (format!("substring({s},GREATEST(1,TRY_CAST(({start}) AS BIGINT)),GREATEST(0,TRY_CAST(({len}) AS BIGINT)))"),Ty::Text,format!("({start})<1 OR ({start})<>trunc({start}) OR ({len})<0 OR ({len})<>trunc({len}) OR TRY_CAST(({start}) AS BIGINT) IS NULL OR TRY_CAST(({len}) AS BIGINT) IS NULL"))
            }
            "SUBSTITUTE" => (
                format!("replace({s},{},{})", text(&a[1]), text(&a[2])),
                Ty::Text,
                format!("{}=''", text(&a[1])),
            ),
            "FIND" => {
                let p = format!("strpos({}, {s})", text(&a[1]));
                (
                    format!("CAST({p} AS DECIMAL(28,4))"),
                    Ty::Number,
                    format!("{p}=0"),
                )
            }
            "TEXTBEFORE" | "TEXTAFTER" => {
                let d = text(&a[1]);
                let n = if a.len() == 3 {
                    integer_arg(&a[2])?;
                    a[2].sql.clone()
                } else {
                    "1".into()
                };
                let split = format!("string_split({s},{d})");
                let pos = format!("TRY_CAST(({n}) AS BIGINT)");
                let slice = if name == "TEXTBEFORE" {
                    format!("list_slice({split},1,{pos})")
                } else {
                    format!("list_slice({split},{pos}+1,len({split}))")
                };
                (format!("array_to_string({slice},{d})"),Ty::Text,format!("{d}='' OR ({n})<1 OR ({n})<>trunc({n}) OR TRY_CAST(({n}) AS BIGINT) IS NULL OR len({split})<=({n})"))
            }
            _ => unreachable!(),
        };
        let nulls = a
            .iter()
            .map(|e| format!("({}) IS NULL", e.sql))
            .collect::<Vec<_>>()
            .join(" OR ");
        let err=format!("COALESCE({errors},CASE WHEN NOT ({nulls}) AND ({invalid}) THEN '文本参数越界或未找到标记' END)");
        Ok(item(
            format!("CASE WHEN ({nulls}) OR ({err}) IS NOT NULL THEN NULL ELSE ({sql}) END"),
            ty,
            err,
        ))
    }
}
fn integer_arg(e: &Compiled) -> Result<(), String> {
    if e.ty != Ty::Number {
        Err("位置、长度及次数须为数值".into())
    } else {
        Ok(())
    }
}
pub fn compile(expression: &str, fields: &[FieldDescriptor]) -> Result<Compiled, String> {
    compile_ordered(expression, fields, "1")
}

pub fn compile_for_table(
    expression: &str, fields: &[FieldDescriptor], conn: &duckdb::Connection, table: &str,
) -> Result<Compiled, String> {
    let checked = compile(expression, fields)?;
    if !checked.uses_row_offset {
        return Ok(checked);
    }
    let order = crate::table_ops::stable_order(conn, table, fields)?;
    compile_ordered(expression, fields, &order)
}

fn compile_ordered(expression: &str, fields: &[FieldDescriptor], order: &str) -> Result<Compiled, String> {
    let mut p = Parser {
        t: tokens(expression)?,
        i: 0,
        fields,
        refs: BTreeSet::new(),
        depth: 0,
        order,
        uses_row_offset: false,
    };
    let mut e = p.expr(1)?;
    if p.i != p.t.len() {
        return Err("公式尾部存在未识别内容".into());
    }
    e.uses_row_offset = p.uses_row_offset;
    e.referenced_fields = p.refs.into_iter().collect();
    Ok(e)
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets_follow_explicit_order_and_stay_inside_groups() {
        let db = duckdb::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sample AS SELECT * FROM (VALUES (4,'A',2,18),(2,'B',1,9),(3,'A',2,15),(1,'A',1,10)) t(__order, grp, month, amount)").unwrap();
        let fields = crate::engine::descriptors(&db,"sample","sample").unwrap();
        let e = super::compile_for_table("[amount]-PREV([amount],1,[month],[grp])",&fields,&db,"sample").unwrap();
        let rows = db.prepare(&format!("SELECT CAST(({}) AS VARCHAR),({}) FROM sample ORDER BY __order",e.sql,e.error)).unwrap()
            .query_map([],|r| Ok((r.get::<_,Option<String>>(0)?, r.get::<_,Option<String>>(1)?))).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap();
        assert_eq!(rows, vec![(None,Some("组内没有目标行".into())),(None,Some("组内没有目标行".into())),(Some("5.0000".into()),None),(Some("3.0000".into()),None)]);
        let e = super::compile_for_table("NEXT([grp],2,[month],[grp])",&fields,&db,"sample").unwrap();
        let next: Option<String> = db.query_row(&format!("SELECT val FROM (SELECT __order, ({}) AS val FROM sample) WHERE __order=1", e.sql),[],|r|r.get(0)).unwrap();
        assert_eq!(next.as_deref(),Some("A"));
        assert!(super::compile("PREV([amount],0,[month])",&fields).is_err());
        assert!(super::compile("PREV([amount],1,[missing])",&fields).is_err());
        assert!(super::compile_for_table("PREV([amount],1,[month])",&fields[1..],&db,"sample").is_err());
    }
    use super::*;
    use duckdb::Connection;
    #[test]
    fn explicit_dates_logic_and_rounding_have_fixed_results() {
        for (expression, expected) in [
            ("ABS(-12.5)", "12.5000"),
            ("ROUND(12.3456,2)", "12.3500"),
            ("VALUE(\" 001.25 \")", "1.2500"),
            ("AND(TRUE,1<2)", "true"),
            ("OR(FALSE,2<1)", "false"),
            ("DATE(2024,2,29)", "2024-02-29"),
            ("DATEVALUE(\"01/02/2024\",\"dd/mm/yyyy\")", "2024-02-01"),
            ("DATEVALUE(\"01/02/2024\",\"mm/dd/yyyy\")", "2024-01-02"),
            ("TEXT(DATE(2024,2,29),\"yyyymmdd\")", "20240229"),
            ("YEAR(DATE(2024,2,29))", "2024.0000"),
            ("MONTH(DATE(2024,2,29))", "2.0000"),
            ("DAY(DATE(2024,2,29))", "29.0000"),
            ("DATEDIFF(DATE(2024,2,28),DATE(2024,3,1))", "2.0000"),
        ] {
            let (value, error) = eval(expression);
            assert_eq!(value.as_deref(), Some(expected), "{expression}");
            assert_eq!(error, None, "{expression}");
        }
        for expression in [
            "DATE(2023,2,29)",
            "VALUE(\"1.12345\")",
            "ROUND(1,5)",
            "DATEVALUE(\"2024-2-1\",\"yyyy-mm-dd\")",
        ] {
            let (value, error) = eval(expression);
            assert_eq!(value, None, "{expression}");
            assert!(error.is_some(), "{expression}");
        }
        assert!(compile("DATEVALUE(\"01/02/2024\",\"auto\")", &[]).is_err());
    }
    fn eval(s: &str) -> (Option<String>, Option<String>) {
        let e = compile(s, &[]).unwrap();
        Connection::open_in_memory()
            .unwrap()
            .query_row(
                &format!("SELECT CAST(({}) AS VARCHAR),({})", e.sql, e.error),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    #[test]
    fn exact_amounts_and_row_errors() {
        assert_eq!(eval("10/4"), (Some("2.5000".into()), None));
        assert!(eval("1/3").1.is_some());
        assert!(eval("1/0").1.is_some());
        assert_eq!(eval("0.0001+0.0002").0.as_deref(), Some("0.0003"));
        assert!(eval("0.0001*0.0001").1.is_some());
        assert_eq!(eval("IF(TRUE,10/4,1/0)").1, None);
    }
    #[test]
    fn text_nested_and_unicode() {
        assert_eq!(
            eval("TEXTBEFORE(TEXTAFTER(\"6602管理费用/职工薪酬/工资\",\"/\"),\"/\")")
                .0
                .as_deref(),
            Some("职工薪酬")
        );
        assert_eq!(eval("MID(\"甲乙丙\",2,1)").0.as_deref(), Some("乙"));
        assert_eq!(eval("FIND(\"乙\",\"甲乙丙\")").0.as_deref(), Some("2.0000"));
        assert!(eval("TEXTAFTER(\"甲\",\"/\")").1.is_some());
        assert!(compile("SELECT(1)", &[]).is_err());
        assert!(compile("[不存在]", &[]).is_err());
        assert!(compile("1.00001", &[]).is_err());
    }
    #[test]
    fn function_parameters_nulls_and_numeric_fields_are_not_silently_coerced() {
        for (expression, expected) in [
            ("LEFT(\"甲乙丙\",2)", "甲乙"),
            ("RIGHT(\"甲乙丙\",2)", "乙丙"),
            ("LEN(\"甲乙丙\")", "3.0000"),
            ("TRIM(\" 甲乙 \" )", "甲乙"),
            ("SUBSTITUTE(\"甲/乙/甲\",\"甲\",\"丙\")", "丙/乙/丙"),
            ("TEXTAFTER(\"甲/乙/丙\",\"/\",2)", "丙"),
            ("IF(1<2,\"是\",\"否\")", "是"),
            ("\"A\"&\"|\"&\"B\"", "A|B"),
        ] {
            assert_eq!(eval(expression), (Some(expected.into()), None));
        }
        assert!(eval("LEFT(\"甲\",-1)").1.is_some());
        assert!(eval("MID(\"甲\",0,1)").1.is_some());
        let fields = vec![
            FieldDescriptor {
                name: "文本".into(),
                data_type: "VARCHAR".into(),
                nullable: true,
                origin: "s".into(),
                confirmed: true,
            },
            FieldDescriptor {
                name: "精度".into(),
                data_type: "DECIMAL(28,5)".into(),
                nullable: true,
                origin: "s".into(),
                confirmed: true,
            },
        ];
        let c = Connection::open_in_memory().unwrap();
        let blank = compile("IF(ISBLANK([文本]),\"空\",[文本])", &fields).unwrap();
        let value: String = c
            .query_row(
                &format!("SELECT ({}) FROM (SELECT NULL::VARCHAR AS 文本)", blank.sql),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "空");
        let precise = compile("[精度]", &fields).unwrap();
        let (v,e):(Option<String>,Option<String>)=c.query_row(&format!("SELECT CAST(({}) AS VARCHAR),({}) FROM (SELECT 1.00001::DECIMAL(28,5) AS 精度)",precise.sql,precise.error),[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(v, None);
        assert!(e.is_some());
        assert!(compile("[文本]+1", &fields).is_err());
    }
}
