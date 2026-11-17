use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldSummary {
    pub name: String,
    pub data_type: String,
    pub non_null_count: i64,
    pub null_count: i64,
    pub sum: Option<String>,
    pub amount_counts: Option<AmountCounts>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AmountCounts {
    pub blank: i64,
    pub failed: i64,
    pub filled_zero: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultSummary {
    pub batch: String,
    pub node_id: String,
    pub category: String,
    pub rows: i64,
    pub fields: Vec<FieldSummary>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormulaCheck {
    pub result_type: String,
    pub referenced_fields: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldDescriptor {pub name:String,pub data_type:String,pub nullable:bool,pub origin:String,pub confirmed:bool}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDiagnostic {pub node_id:String,pub parameter:String,pub code:String,pub message:String}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputContract {pub outputs:BTreeMap<String,Vec<FieldDescriptor>>,pub diagnostics:Vec<NodeDiagnostic>,pub exception_mode:String}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    #[serde(default = "one")]
    pub version: u32,
    pub label: String,
    pub position: Point,
    #[serde(default)]
    pub config: Value,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub source_port: Option<String>,
    #[serde(default)]
    pub target_port: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub schema_version: u32,
    pub name: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceBinding {
    pub path: String,
    #[serde(default)]
    pub sheet: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub layout: Option<TableLayout>,
    #[serde(default)]
    pub text: Option<TextOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_baseline: Option<ReadingBaseline>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadingBaseline {
    pub version: u32,
    pub sheet: String,
    pub layout: TableLayout,
    pub header_cells: Vec<Vec<String>>,
    pub header_merges: Vec<(usize, usize, u32, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadingIssue {
    pub code: String,
    pub message: String,
    pub row: Option<usize>,
    pub col: Option<u32>,
    pub related_cols: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadingCheck {
    pub compatible: bool,
    pub baseline: Option<ReadingBaseline>,
    pub diagnostics: Vec<ReadingIssue>,
    pub report: Option<StructureReport>,
    pub full_checked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextOptions {
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    #[serde(default = "one_usize")]
    pub header_record: usize,
}

fn default_encoding() -> String {
    "utf-8".into()
}
fn default_delimiter() -> String {
    ",".into()
}
fn one_usize() -> usize {
    1
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            encoding: default_encoding(),
            delimiter: default_delimiter(),
            header_record: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableLayout {
    pub first_col: u32,
    pub last_col: u32,
    pub header_rows: Vec<usize>,
    pub data_start_row: usize,
    #[serde(default)]
    pub data_end_row: Option<usize>,
    #[serde(default)]
    pub fill_merged_data_cols: Vec<u32>,
    /// Explicit user corrections keyed by physical (one-based) column.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_names: BTreeMap<u32, String>,
    /// New suggestions reject unseen populated columns unless exclusion is explicitly accepted.
    #[serde(default)]
    pub strict_columns: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructureField {
    pub physical_col: u32,
    pub parts: Vec<String>,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructureCandidate {
    pub layout: TableLayout,
    pub fields: Vec<StructureField>,
    pub issues: Vec<String>,
    pub ready: bool,
    pub score: i32,
    pub diagnostics: Vec<ReadingIssue>,
    pub baseline: ReadingBaseline,
    pub preview: SourcePreview,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructureReport {
    pub raw: RawSourcePreview,
    pub candidates: Vec<StructureCandidate>,
    pub active: Option<StructureCandidate>,
    pub notices: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompanyConfig {
    pub schema_version: u32,
    pub company: String,
    pub sources: BTreeMap<String, SourceBinding>,
}

/// A local editing project. Bindings use source node IDs, so a single file does
/// not require a separate company configuration or a hard-coded JE/TB role.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectFile {
    pub schema_version: u32,
    pub workflow: Workflow,
    #[serde(default)]
    pub company_name: Option<String>,
    #[serde(default)]
    pub bindings: BTreeMap<String, SourceBinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeReview {
    pub node_id: String,
    pub label: String,
    pub rows: i64,
    pub amount_total: Option<String>,
    pub exceptions: i64,
    pub excluded: i64,
    pub table: String,
    pub matched: Option<i64>,
    pub left_only: Option<i64>,
    pub right_only: Option<i64>,
    pub unresolved: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub run_dir: String,
    pub database: String,
    pub reviews: Vec<NodeReview>,
    pub input_fingerprints: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportReceipt {
    pub path: String,
    pub rows: i64,
    pub node_id: String,
    pub category: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourcePreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub physical_rows: Vec<usize>,
    pub truncated: bool,
    pub sheet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawSourcePreview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub physical_rows: Vec<usize>,
    pub truncated: bool,
    pub sheet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TracePart {
    pub label: String,
    pub preview: Preview,
}
