//! Cell formatting and sheet layout: read from xlsx, shown in blocks, kept on save.

use rust_xlsxwriter::{
    Color, ConditionalFormatCell, ConditionalFormatCellRule, Format, FormatAlign, FormatBorder, Workbook as XWorkbook,
};
use speedsheet_core::layout::{Area, CfTest};
use speedsheet_core::style::{HAlign, LineStyle, VAlign};
use speedsheet_core::{io, ops, Value, Workbook};

/// A members list like the ones exported from administration software.
fn styled_file(path: &std::path::Path) {
    let mut x = XWorkbook::new();
    let ws = x.add_worksheet();
    ws.set_name("Leden").unwrap();
    let title = Format::new().set_bold().set_font_size(12);
    let head = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F3864))
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap()
        .set_border(FormatBorder::Thin)
        .set_border_color(Color::RGB(0x808080));
    let blue = Format::new().set_font_color(Color::RGB(0x0070C0)).set_border_bottom(FormatBorder::Thin);
    let money = Format::new().set_num_format("#,##0.00 \"EUR\"").set_italic();
    let date = Format::new().set_num_format("dd/mm/yyyy");
    ws.merge_range(0, 0, 0, 2, "Bijlage 1 : leden 2026", &title).unwrap();
    for (c, h) in ["ID\nlid", "Naam\nlid", "Voornaam\nlid", "Bijdrage", "Score", "Geheim"].iter().enumerate() {
        ws.write_string_with_format(1, c as u16, *h, &head).unwrap();
    }
    for r in 0..5u32 {
        ws.write_number(r + 2, 0, 1989680.0 + r as f64).unwrap();
        ws.write_string(r + 2, 1, ["Tawfik", "A de xia", "Aabbou", "Aadmi", "Aafjes"][r as usize]).unwrap();
        ws.write_string_with_format(r + 2, 2, ["Ilyass", "Kunga", "Ayoub", "Safwan", "Raydjen"][r as usize], &blue).unwrap();
        ws.write_number_with_format(r + 2, 3, 1234.5 * (r + 1) as f64, &money).unwrap();
        ws.write_number(r + 2, 4, (r * 30) as f64).unwrap();
        ws.write_string(r + 2, 5, "x").unwrap();
        ws.set_row_height(r + 2, 30).unwrap();
    }
    ws.write_number_with_format(8, 3, 46000.0, &date).unwrap();
    ws.set_row_height(1, 45).unwrap();
    ws.set_column_width(0, 12).unwrap();
    ws.set_column_width(1, 20).unwrap();
    ws.set_column_hidden(5).unwrap();
    ws.set_freeze_panes(2, 0).unwrap();
    ws.autofilter(1, 0, 6, 4).unwrap();
    let red = Format::new().set_background_color(Color::RGB(0xFFC7CE)).set_font_color(Color::RGB(0x9C0006));
    let cf = ConditionalFormatCell::new().set_rule(ConditionalFormatCellRule::GreaterThan(50)).set_format(red);
    ws.add_conditional_format(2, 4, 6, 4, &cf).unwrap();
    x.save(path).unwrap();
}

fn check(wb: &Workbook) {
    let s = &wb.sheets[0];
    let style = |r: usize, c: usize| &wb.styles[s.style_id(r, c) as usize];
    // Header: bold white on dark blue, centered, wrapped, with borders.
    let h = style(1, 1);
    assert!(h.bold, "{h:?}");
    assert_eq!(h.color, Some(0xFFFFFF));
    assert_eq!(h.fill, Some(0x1F3864));
    assert_eq!(h.h_align, HAlign::Center);
    assert_eq!(h.v_align, VAlign::Center);
    assert!(h.wrap);
    assert_eq!(h.borders[0].map(|b| (b.style, b.color)), Some((LineStyle::Thin, 0x808080)));
    // Title is bold and merged over three columns.
    assert!(style(0, 0).bold);
    assert_eq!(style(0, 0).font_size, Some(12.0));
    assert_eq!(s.merges, vec![Area { r0: 0, c0: 0, r1: 0, c1: 2 }]);
    // Plain cells keep the default style.
    assert_eq!(s.style_id(2, 1), 0);
    assert_eq!(style(2, 2).color, Some(0x0070C0));
    assert!(style(2, 2).borders[3].is_some());
    // Number formats are applied for display.
    let b = ops::block(wb, 0, 0, 8, 0, 5);
    let text = |r: usize, c: usize| b.text[r * b.cols as usize + c].clone();
    assert_eq!(text(2, 3), "1,234.50 EUR");
    assert_eq!(text(3, 3), "2,469.00 EUR");
    assert_eq!(text(8, 3), "09/12/2025");
    assert_eq!(text(1, 0), "ID\nlid");
    assert_eq!(b.styles[b.cols as usize + 1], s.style_id(1, 1));
    // Conditional format: score > 50 gets the dxf.
    let cf = b.cf.as_ref().expect("conditional formats");
    let at = |r: usize| cf[r * b.cols as usize + 4];
    assert_eq!((at(2), at(3), at(4), at(5)), (0, 0, 1, 1));
    let dxf = &wb.dxfs[at(4) as usize - 1];
    assert_eq!(dxf.fill, Some(0xFFC7CE));
    assert_eq!(dxf.color, Some(0x9C0006));
    assert!(matches!(&s.cond[0].rules[0].test, CfTest::Cell { op, a, .. } if op == "gt" && a == "50"));
    // Layout.
    assert_eq!(s.col_width(0), Some(89.0));
    assert_eq!(s.col_width(1), Some(145.0));
    assert_eq!(s.col_width(5), Some(0.0));
    assert_eq!(s.row_height(1), Some(60.0));
    assert_eq!(s.row_height(2), Some(40.0));
    assert_eq!(s.row_height(0), None);
    assert_eq!(s.default_row_height, Some(20.0));
    assert_eq!(s.freeze, Some((2, 0)));
    assert_eq!(s.autofilter, Some(Area { r0: 1, c0: 0, r1: 6, c1: 4 }));
}

#[test]
fn reads_and_keeps_formatting() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("styled.xlsx");
    styled_file(&p);
    let mut wb = io::open(&p).unwrap();
    check(&wb);

    // Saving and reopening keeps everything.
    let out = dir.path().join("saved.xlsx");
    io::save(&mut wb, &out, 0).unwrap();
    let back = io::open(&out).unwrap();
    check(&back);
}

#[test]
fn autofilter_header_and_sort_keep_formats() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("styled.xlsx");
    styled_file(&p);
    let mut wb = io::open(&p).unwrap();
    // Sorting by name starts below the autofilter header (row 2) and moves formats with rows.
    let blue_row = |wb: &Workbook| (2..7).find(|&r| wb.styles[wb.sheets[0].style_id(r, 2) as usize].color.is_some());
    assert_eq!(blue_row(&wb), Some(2));
    wb.sort(0, 1, false, false).unwrap();
    assert_eq!(wb.sheets[0].value(0, 0).display(), "Bijlage 1 : leden 2026");
    assert_eq!(wb.sheets[0].value(1, 1).display(), "Naam\nlid");
    assert_eq!(wb.sheets[0].value(2, 1).display(), "Tawfik");
    // The formats moved along with the data.
    let r = (2..7).find(|&r| wb.sheets[0].value(r, 2).display() == "Ilyass").unwrap();
    assert!(wb.styles[wb.sheets[0].style_id(r, 2) as usize].color.is_some());
    wb.undo();
    assert_eq!(blue_row(&wb), Some(2));

    // Filters keep the rows above and including the autofilter header.
    let f = speedsheet_core::Filter {
        header: false,
        conds: vec![speedsheet_core::FilterCond { col: 1, op: speedsheet_core::FilterOp::Contains { value: "aa".into() } }],
    };
    wb.set_filter(0, Some(f)).unwrap();
    let v = wb.sheets[0].view.clone().unwrap();
    assert_eq!(&v[..2], &[0, 1]);
    assert!(v[2..].iter().all(|&r| wb.sheets[0].value(r as usize, 1).display().to_lowercase().contains("aa")));
    assert_eq!(wb.sheets[0].value(2, 4), &Value::Number(0.0));
}
