use nomifun_api_types::ConversionTarget;
use nomifun_office::ConversionService;
use rust_xlsxwriter::{Format, Workbook};
use std::path::PathBuf;
use tempfile::TempDir;

fn create_simple_xlsx(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("test.xlsx");
    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.set_name("Sheet1").unwrap();
    sheet.write_string(0, 0, "Name").unwrap();
    sheet.write_string(0, 1, "Age").unwrap();
    sheet.write_string(1, 0, "Alice").unwrap();
    sheet.write_number(1, 1, 30.0).unwrap();
    sheet.write_string(2, 0, "Bob").unwrap();
    sheet.write_number(2, 1, 25.0).unwrap();
    wb.save(&path).unwrap();
    path
}

fn create_multi_sheet_xlsx(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("multi.xlsx");
    let mut wb = Workbook::new();

    let s1 = wb.add_worksheet();
    s1.set_name("Users").unwrap();
    s1.write_string(0, 0, "Name").unwrap();
    s1.write_number(0, 1, 1.0).unwrap();

    let s2 = wb.add_worksheet();
    s2.set_name("Products").unwrap();
    s2.write_string(0, 0, "Item").unwrap();
    s2.write_number(0, 1, 9.99).unwrap();

    let s3 = wb.add_worksheet();
    s3.set_name("Empty").unwrap();

    wb.save(&path).unwrap();
    path
}

fn create_xlsx_with_merges(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("merged.xlsx");
    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.set_name("Merged").unwrap();
    let fmt = Format::new();
    sheet.merge_range(0, 0, 1, 2, "Merged Title", &fmt).unwrap();
    sheet.write_string(2, 0, "A").unwrap();
    sheet.write_string(2, 1, "B").unwrap();
    sheet.write_string(2, 2, "C").unwrap();
    wb.save(&path).unwrap();
    path
}

#[allow(clippy::approx_constant)] // 3.14 is test data, not an approximation of PI
fn create_xlsx_with_types(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("types.xlsx");
    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.set_name("Types").unwrap();
    sheet.write_string(0, 0, "text").unwrap();
    sheet.write_number(0, 1, 42.0).unwrap();
    sheet.write_number(0, 2, 3.14).unwrap();
    sheet.write_boolean(0, 3, true).unwrap();
    wb.save(&path).unwrap();
    path
}

// DC-1: Excel → JSON (normal)
#[tokio::test]
async fn dc1_excel_to_json_simple() {
    let dir = TempDir::new().unwrap();
    let path = create_simple_xlsx(&dir);
    let svc = ConversionService::new(None);

    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert_eq!(resp.to, "excel-json");
    assert!(resp.result.success);
    assert!(resp.result.error.is_none());

    let data = resp.result.data.unwrap();
    let sheets = data["sheets"].as_array().unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0]["name"], "Sheet1");

    let rows = sheets[0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0], "Name");
    assert_eq!(rows[0][1], "Age");
    assert_eq!(rows[1][0], "Alice");
    assert_eq!(rows[1][1], 30.0);
    assert_eq!(rows[2][0], "Bob");
    assert_eq!(rows[2][1], 25.0);
}

// DC-2: Excel → JSON (multiple sheets)
#[tokio::test]
async fn dc2_excel_to_json_multi_sheet() {
    let dir = TempDir::new().unwrap();
    let path = create_multi_sheet_xlsx(&dir);
    let svc = ConversionService::new(None);

    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert!(resp.result.success);
    let data = resp.result.data.unwrap();
    let sheets = data["sheets"].as_array().unwrap();
    assert_eq!(sheets.len(), 3);
    assert_eq!(sheets[0]["name"], "Users");
    assert_eq!(sheets[1]["name"], "Products");
    assert_eq!(sheets[2]["name"], "Empty");
}

// DC-3: Excel → JSON (with merged cells)
#[tokio::test]
async fn dc3_excel_to_json_with_merges() {
    let dir = TempDir::new().unwrap();
    let path = create_xlsx_with_merges(&dir);
    let svc = ConversionService::new(None);

    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert!(resp.result.success);
    let data = resp.result.data.unwrap();
    let sheet = &data["sheets"][0];
    assert_eq!(sheet["name"], "Merged");

    let merges = sheet["merges"].as_array().unwrap();
    assert!(!merges.is_empty());

    let merge = &merges[0];
    assert_eq!(merge["s"]["r"], 0);
    assert_eq!(merge["s"]["c"], 0);
    assert_eq!(merge["e"]["r"], 1);
    assert_eq!(merge["e"]["c"], 2);
}

// DC-4: Excel → JSON (file not found)
#[tokio::test]
async fn dc4_excel_to_json_file_not_found() {
    let svc = ConversionService::new(None);
    let resp = svc
        .convert("/nonexistent/file.xlsx", ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert_eq!(resp.to, "excel-json");
    assert!(!resp.result.success);
    assert!(resp.result.data.is_none());
    assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
}

// DC-6: Word → Markdown (pandoc not available)
#[tokio::test]
async fn dc6_word_to_markdown_file_not_found() {
    let svc = ConversionService::new(None);
    let resp = svc
        .convert("/nonexistent/file.docx", ConversionTarget::Markdown)
        .await
        .unwrap();

    assert_eq!(resp.to, "markdown");
    assert!(!resp.result.success);
    assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
}

// DC-8: PPT → JSON (officecli not available)
#[tokio::test]
async fn dc8_ppt_to_json_file_not_found() {
    let svc = ConversionService::new(None);
    let resp = svc
        .convert("/nonexistent/file.pptx", ConversionTarget::PptJson)
        .await
        .unwrap();

    assert_eq!(resp.to, "ppt-json");
    assert!(!resp.result.success);
    assert!(resp.result.error.as_ref().unwrap().contains("file not found"));
}

// DC-8b: PPT → JSON (officecli not installed — configured path invalid and not in PATH)
#[tokio::test]
async fn dc8b_ppt_to_json_officecli_not_installed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("slides.pptx");
    std::fs::write(&path, b"fake pptx content").unwrap();

    // Use a wrapper that ensures officecli is not found
    let svc = ConversionService::new(Some("/nonexistent/officecli".into()));
    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::PptJson)
        .await
        .unwrap();

    // Conversion should fail — either because officecli is not found or because
    // it fails to parse the fake file. Either way, success must be false.
    assert!(!resp.result.success);
    assert!(resp.result.error.is_some());
}

// Excel cell type handling
#[tokio::test]
#[allow(clippy::approx_constant)] // 3.14 is test data, not an approximation of PI
async fn excel_to_json_cell_types() {
    let dir = TempDir::new().unwrap();
    let path = create_xlsx_with_types(&dir);
    let svc = ConversionService::new(None);

    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert!(resp.result.success);
    let data = resp.result.data.unwrap();
    let row = &data["sheets"][0]["data"][0];

    assert_eq!(row[0], "text");
    assert_eq!(row[1], 42.0);
    assert!((row[2].as_f64().unwrap() - 3.14).abs() < 0.01);
    assert_eq!(row[3], true);
}

// Excel empty file
#[tokio::test]
async fn excel_to_json_empty_workbook() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.xlsx");
    let mut wb = Workbook::new();
    wb.add_worksheet().set_name("Empty").unwrap();
    wb.save(&path).unwrap();

    let svc = ConversionService::new(None);
    let resp = svc
        .convert(path.to_str().unwrap(), ConversionTarget::ExcelJson)
        .await
        .unwrap();

    assert!(resp.result.success);
    let data = resp.result.data.unwrap();
    let sheets = data["sheets"].as_array().unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0]["name"], "Empty");
}

// Response always returns success=true at convert() level (error wrapped in result)
#[tokio::test]
async fn convert_always_returns_ok_with_result_wrapper() {
    let svc = ConversionService::new(None);

    let targets = [
        ConversionTarget::Markdown,
        ConversionTarget::ExcelJson,
        ConversionTarget::PptJson,
    ];

    for target in targets {
        let resp = svc.convert("/nonexistent/path", target).await;
        assert!(resp.is_ok());
        assert!(!resp.unwrap().result.success);
    }
}
