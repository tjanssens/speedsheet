//! Write the source files of the test corpus in `testdata/corpus/`. They hold the things real
//! business files contain and SpeedSheet must not lose or miscalculate: named ranges, data
//! validation, hyperlinks, notes, charts, images, macros and many formulas.
//!
//! These files have no calculated values. `scripts/refresh-corpus.sh` runs this example and lets
//! LibreOffice open, recalculate and save each file, so the committed corpus has cached values
//! from an independent spreadsheet engine (the "oracle" the tests compare against).
//!
//! `cargo run -p speedsheet-core --example gen_corpus -- <out-dir>`

use std::path::{Path, PathBuf};

use rust_xlsxwriter::{Chart, ChartType, Color, DataValidation, DocProperties, Formula, Image, Note, Workbook, XlsxError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "testdata/corpus".into()));
    std::fs::create_dir_all(&out)?;
    features(&out.join("features.xlsx"), None)?;
    let vba = out.join("vbaProject.bin");
    // Not a real VBA project: the tests only check the part survives, Excel would not run it.
    std::fs::write(&vba, b"SpeedSheet test VBA project placeholder")?;
    features(&out.join("macros.xlsm"), Some(&vba))?;
    std::fs::remove_file(&vba)?;
    formulas(&out.join("formulas.xlsx"))?;
    println!("wrote corpus sources to {}", out.display());
    Ok(())
}

/// Everything around the cells that a save must keep.
fn features(path: &Path, vba: Option<&Path>) -> Result<(), XlsxError> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet().set_name("Data")?;
    ws.write(0, 0, "Rate")?;
    ws.write(0, 1, 0.21)?;
    for r in 1..6u32 {
        ws.write(r, 0, format!("Item {r}"))?;
        ws.write(r, 1, (r * 10) as f64)?;
        ws.write_formula(r, 2, Formula::new(format!("=B{}*Rate", r + 1)))?;
    }
    ws.write(0, 3, "Status")?;
    let dv = DataValidation::new().allow_list_strings(&["Open", "Done"])?;
    ws.add_data_validation(1, 3, 5, 3, &dv)?;
    ws.write_url(0, 5, "https://github.com/tjanssens/speedsheet")?;
    ws.insert_note(1, 0, &Note::new("A note on Item 1"))?;
    let mut chart = Chart::new(ChartType::Column);
    chart.add_series().set_categories(("Data", 1, 0, 5, 0)).set_values(("Data", 1, 1, 5, 1));
    ws.insert_chart(7, 0, &chart)?;
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app-icon.png");
    ws.insert_image(7, 8, &Image::new(icon)?.set_scale_width(0.2).set_scale_height(0.2))?;
    // Page setup and sheet settings a user chose on purpose.
    ws.set_landscape().set_paper_size(9).set_print_fit_to_pages(1, 0).set_print_gridlines(true);
    ws.set_tab_color(Color::Red);
    ws.set_header("&CSpeedSheet test");
    wb.define_name("Rate", "=Data!$B$1")?;
    wb.set_properties(&DocProperties::new().set_title("Corpus").set_custom_property("Classification", "Internal"));
    if let Some(vba) = vba {
        wb.add_vba_project(vba)?;
    }
    wb.save(path)
}

/// Formulas the engine supports, for comparing its results with another engine's.
fn formulas(path: &Path) -> Result<(), XlsxError> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet().set_name("Input")?;
    let rows: [(&str, f64, &str); 6] = [
        ("apple", 3.0, "red"),
        ("banana", 5.0, "yellow"),
        ("cherry", 7.0, "red"),
        ("date", 2.5, "brown"),
        ("elder", -4.0, "black"),
        ("fig", 0.0, "purple"),
    ];
    for (r, (name, qty, color)) in rows.iter().enumerate() {
        let r = r as u32;
        ws.write(r, 0, *name)?;
        ws.write(r, 1, *qty)?;
        ws.write(r, 2, *color)?;
    }
    ws.write(7, 0, "Rate")?;
    ws.write(7, 1, 0.21)?;
    wb.define_name("Rate", "=Input!$B$8")?;
    wb.define_name("Qty", "=Input!$B$1:$B$6")?;

    let ws = wb.add_worksheet().set_name("Calc")?;
    let cases = [
        "=SUM(Input!B1:B6)",
        "=AVERAGE(Input!B1:B6)",
        "=MAX(Input!B1:B6)-MIN(Input!B1:B6)",
        "=COUNT(Input!A1:C6)",
        "=COUNTA(Input!A1:C6)",
        "=COUNTIF(Input!C1:C6,\"red\")",
        "=SUMIFS(Input!B1:B6,Input!C1:C6,\"red\")",
        "=VLOOKUP(\"cherry\",Input!A1:C6,2,FALSE)",
        "=HLOOKUP(\"x\",{\"x\",\"y\";1,2},2,FALSE)",
        "=INDEX(Input!A1:C6,4,3)",
        "=MATCH(\"date\",Input!A1:A6,0)",
        "=IF(Input!B2>4,\"big\",\"small\")",
        "=IFERROR(1/Input!B6,\"div0\")",
        "=ROUND(Input!B4*1.234,2)",
        "=ROUNDUP(2.341,1)+ROUNDDOWN(2.349,1)",
        "=INT(-2.5)",
        "=MOD(-7,3)",
        "=ABS(Input!B5)*SIGN(Input!B5)",
        "=SQRT(16)+POWER(2,5)",
        "=PRODUCT(Input!B1:B4)",
        "=SUMPRODUCT(Input!B1:B3,Input!B2:B4)",
        "=MEDIAN(Input!B1:B6)",
        "=SMALL(Input!B1:B6,2)",
        "=UPPER(Input!A1)&LOWER(\"XY\")",
        "=PROPER(\"hello world\")",
        "=LEN(Input!A2)",
        "=MID(Input!A3,2,3)",
        "=RIGHT(Input!A4,2)",
        "=SUBSTITUTE(Input!A2,\"a\",\"o\")",
        "=TRIM(\"  a   b  \")",
        "=REPT(\"ab\",3)",
        "=EXACT(\"a\",\"A\")",
        "=SEARCH(\"rr\",Input!A3)",
        "=CONCATENATE(Input!A1,\"-\",Input!B1)",
        "=TEXT(1234.5,\"0.00\")",
        "=VALUE(\"12.5\")+1",
        "=DATE(2026,11,27)",
        "=YEAR(DATE(2026,11,27))*100+MONTH(DATE(2026,11,27))",
        "=DAY(DATE(2026,2,30))",
        "=WEEKDAY(DATE(2026,11,27))",
        "=HOUR(0.75)+MINUTE(0.7525)",
        "=AND(Input!B1>0,Input!B2>0)",
        "=OR(Input!B5>0,Input!B6>0)",
        "=NOT(ISBLANK(Input!A1))",
        "=ISNUMBER(Input!B1)+ISTEXT(Input!A1)",
        "=ISERROR(1/0)",
        "=CHOOSE(2,\"a\",\"b\",\"c\")",
        "=1/0",
        "=NA()",
        "=Input!B2*Rate",
        "=SUM(Qty)*Rate",
        "=Rate*100",
    ];
    for (r, f) in cases.iter().enumerate() {
        ws.write(r as u32, 0, *f)?;
        ws.write_formula(r as u32, 1, Formula::new(*f))?;
    }
    // A chain that runs upwards: each cell reads the one below it.
    let ws = wb.add_worksheet().set_name("Chain")?;
    let n = 200u32;
    for r in 0..n - 1 {
        ws.write_formula(r, 0, Formula::new(format!("=A{}+1", r + 2)))?;
    }
    ws.write(n - 1, 0, 1.0)?;
    wb.save(path)
}
