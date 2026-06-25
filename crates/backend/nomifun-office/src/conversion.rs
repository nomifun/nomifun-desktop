use std::path::Path;

use calamine::{DataType, Reader, Sheets, open_workbook_auto};
use nomifun_api_types::{
    CellCoord, CellRange, ConversionResultDto, ConversionTarget, DocumentConversionResponse, ExcelSheetData,
    ExcelWorkbookData,
};
use nomifun_runtime::Builder as CmdBuilder;
use serde_json::Value;
use tracing::warn;

use crate::error::OfficeError;

pub struct ConversionService {
    officecli_path: Option<String>,
}

impl ConversionService {
    pub fn new(officecli_path: Option<String>) -> Self {
        Self { officecli_path }
    }

    pub async fn convert(
        &self,
        file_path: &str,
        target: ConversionTarget,
    ) -> Result<DocumentConversionResponse, OfficeError> {
        let to_str = match target {
            ConversionTarget::Markdown => "markdown",
            ConversionTarget::ExcelJson => "excel-json",
            ConversionTarget::PptJson => "ppt-json",
        };

        let result = match target {
            ConversionTarget::Markdown => self.word_to_markdown(file_path).await,
            ConversionTarget::ExcelJson => self.excel_to_json(file_path),
            ConversionTarget::PptJson => self.ppt_to_json(file_path).await,
        };

        let result_dto = match result {
            Ok(data) => ConversionResultDto {
                success: true,
                data: Some(data),
                error: None,
            },
            Err(e) => ConversionResultDto {
                success: false,
                data: None,
                error: Some(e.to_string()),
            },
        };

        Ok(DocumentConversionResponse {
            to: to_str.to_string(),
            result: result_dto,
        })
    }

    async fn word_to_markdown(&self, file_path: &str) -> Result<Value, OfficeError> {
        validate_file_exists(file_path)?;

        let pandoc = find_executable("pandoc");
        let pandoc_path = pandoc.ok_or_else(|| {
            OfficeError::ToolNotFound(
                "pandoc not installed. Install it via: brew install pandoc (macOS) \
                 or apt-get install pandoc (Linux)"
                    .into(),
            )
        })?;

        let mut builder = CmdBuilder::clean_cli(&pandoc_path);
        builder.args(["-f", "docx", "-t", "markdown", "--wrap=none", file_path]);
        let output = builder
            .output()
            .await
            .map_err(|e| OfficeError::Conversion(format!("failed to run pandoc: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OfficeError::Conversion(format!("pandoc failed: {stderr}")));
        }

        let markdown = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(Value::String(markdown))
    }

    fn excel_to_json(&self, file_path: &str) -> Result<Value, OfficeError> {
        validate_file_exists(file_path)?;

        let mut workbook: Sheets<_> = open_workbook_auto(file_path)
            .map_err(|e| OfficeError::Conversion(format!("failed to open workbook: {e}")))?;

        let sheet_names = workbook.sheet_names().to_vec();
        let mut sheets = Vec::with_capacity(sheet_names.len());

        for name in &sheet_names {
            let range = workbook
                .worksheet_range(name)
                .map_err(|e| OfficeError::Conversion(format!("failed to read sheet '{name}': {e}")))?;

            let data = convert_range_to_2d_array(&range);
            let merges = extract_merge_regions(&mut workbook, name);

            sheets.push(ExcelSheetData {
                name: name.clone(),
                data,
                merges,
                images: None,
            });
        }

        let workbook_data = ExcelWorkbookData { sheets };
        serde_json::to_value(workbook_data).map_err(OfficeError::Json)
    }

    async fn ppt_to_json(&self, file_path: &str) -> Result<Value, OfficeError> {
        validate_file_exists(file_path)?;

        let officecli = resolve_officecli(&self.officecli_path).await?;

        let mut builder = CmdBuilder::clean_cli(&officecli);
        builder.args(["ppt2json", file_path]);
        let output = builder
            .output()
            .await
            .map_err(|e| OfficeError::Conversion(format!("failed to run officecli ppt2json: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OfficeError::Conversion(format!("officecli ppt2json failed: {stderr}")));
        }

        let json: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| OfficeError::Conversion(format!("failed to parse officecli ppt2json output: {e}")))?;

        Ok(json)
    }
}

fn validate_file_exists(file_path: &str) -> Result<(), OfficeError> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err(OfficeError::Conversion(format!("file not found: {file_path}")));
    }
    if !path.is_file() {
        return Err(OfficeError::Conversion(format!("not a file: {file_path}")));
    }
    Ok(())
}

fn convert_range_to_2d_array(range: &calamine::Range<calamine::Data>) -> Vec<Vec<Value>> {
    let (rows, cols) = range.get_size();
    let mut data = Vec::with_capacity(rows);

    for r in 0..rows {
        let mut row = Vec::with_capacity(cols);
        for c in 0..cols {
            let cell = &range[(r, c)];
            let value = cell_to_json_value(cell);
            row.push(value);
        }
        data.push(row);
    }

    data
}

fn cell_to_json_value(cell: &calamine::Data) -> Value {
    if cell.is_empty() {
        return Value::Null;
    }
    if let Some(b) = cell.get_bool() {
        return Value::Bool(b);
    }
    if let Some(i) = cell.get_int() {
        return Value::Number(i.into());
    }
    if let Some(f) = cell.get_float() {
        return serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Some(s) = cell.as_string() {
        return Value::String(s);
    }
    Value::Null
}

fn extract_merge_regions<RS: std::io::Read + std::io::Seek>(
    workbook: &mut Sheets<RS>,
    sheet_name: &str,
) -> Option<Vec<CellRange>> {
    let xlsx = match workbook {
        Sheets::Xlsx(wb) => wb,
        _ => return None,
    };

    if xlsx.load_merged_regions().is_err() {
        warn!("failed to load merged regions");
        return None;
    }

    let regions = xlsx.merged_regions_by_sheet(sheet_name);
    if regions.is_empty() {
        return None;
    }

    let ranges: Vec<CellRange> = regions
        .into_iter()
        .map(|(_, _, dim)| CellRange {
            s: CellCoord {
                r: dim.start.0 as usize,
                c: dim.start.1 as usize,
            },
            e: CellCoord {
                r: dim.end.0 as usize,
                c: dim.end.1 as usize,
            },
        })
        .collect();

    Some(ranges)
}

fn find_executable(name: &str) -> Option<String> {
    which::which(name).ok().map(|p| p.to_string_lossy().into_owned())
}

async fn resolve_officecli(configured_path: &Option<String>) -> Result<String, OfficeError> {
    if let Some(path) = configured_path
        && Path::new(path).exists()
    {
        return Ok(path.clone());
    }

    if let Some(found) = find_executable("officecli") {
        return Ok(found);
    }

    Err(OfficeError::ToolNotFound(
        "officecli not installed. Install it to enable PPT → JSON conversion".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_file_exists_nonexistent() {
        let result = validate_file_exists("/nonexistent/file.xlsx");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn validate_file_exists_is_directory() {
        let result = validate_file_exists("/tmp");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not a file"));
    }

    #[test]
    fn cell_to_json_value_empty() {
        let cell = calamine::Data::Empty;
        assert_eq!(cell_to_json_value(&cell), Value::Null);
    }

    #[test]
    fn cell_to_json_value_bool() {
        let cell = calamine::Data::Bool(true);
        assert_eq!(cell_to_json_value(&cell), Value::Bool(true));
    }

    #[test]
    fn cell_to_json_value_int() {
        let cell = calamine::Data::Int(42);
        assert_eq!(cell_to_json_value(&cell), serde_json::json!(42));
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 is test data, not an approximation of PI
    fn cell_to_json_value_float() {
        let cell = calamine::Data::Float(3.14);
        let val = cell_to_json_value(&cell);
        assert!(val.is_number());
        let n = val.as_f64().unwrap();
        assert!((n - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn cell_to_json_value_string() {
        let cell = calamine::Data::String("hello".to_string());
        assert_eq!(cell_to_json_value(&cell), Value::String("hello".into()));
    }

    #[test]
    fn convert_range_empty() {
        let range = calamine::Range::<calamine::Data>::new((0, 0), (0, 0));
        let data = convert_range_to_2d_array(&range);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].len(), 1);
    }

    #[test]
    fn conversion_service_new() {
        let svc = ConversionService::new(None);
        assert!(svc.officecli_path.is_none());

        let svc = ConversionService::new(Some("/usr/local/bin/officecli".into()));
        assert_eq!(svc.officecli_path.as_deref(), Some("/usr/local/bin/officecli"));
    }

    #[tokio::test]
    async fn convert_excel_file_not_found() {
        let svc = ConversionService::new(None);
        let resp = svc
            .convert("/nonexistent/file.xlsx", ConversionTarget::ExcelJson)
            .await
            .unwrap();
        assert!(!resp.result.success);
        assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
        assert_eq!(resp.to, "excel-json");
    }

    #[tokio::test]
    async fn convert_word_file_not_found() {
        let svc = ConversionService::new(None);
        let resp = svc
            .convert("/nonexistent/file.docx", ConversionTarget::Markdown)
            .await
            .unwrap();
        assert!(!resp.result.success);
        assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
        assert_eq!(resp.to, "markdown");
    }

    #[tokio::test]
    async fn convert_ppt_file_not_found() {
        let svc = ConversionService::new(None);
        let resp = svc
            .convert("/nonexistent/file.pptx", ConversionTarget::PptJson)
            .await
            .unwrap();
        assert!(!resp.result.success);
        assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
        assert_eq!(resp.to, "ppt-json");
    }
}
