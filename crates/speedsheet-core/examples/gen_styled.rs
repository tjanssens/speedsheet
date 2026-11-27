//! Write a formatted test workbook like a typical members list exported to Excel:
//! `cargo run -p speedsheet-core --example gen_styled -- testdata/styled-members.xlsx`

use rust_xlsxwriter::{
    Color, ConditionalFormatCell, ConditionalFormatCellRule, ConditionalFormatText, ConditionalFormatTextRule, Format,
    FormatAlign, FormatBorder, Workbook,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "testdata/styled-members.xlsx".into());
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Leden 2026")?;

    let title = Format::new().set_bold().set_font_size(12);
    let head = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap()
        .set_border(FormatBorder::Thin)
        .set_border_color(Color::RGB(0x8EA9DB));
    let head_dark = head.clone().set_background_color(Color::RGB(0x1F3864)).set_font_color(Color::White);
    let cell = Format::new().set_border_bottom(FormatBorder::Hair).set_border_bottom_color(Color::RGB(0xBFBFBF));
    let blue = cell.clone().set_font_color(Color::RGB(0x0070C0));
    let wrap = cell.clone().set_text_wrap().set_align(FormatAlign::VerticalCenter);
    let money = cell.clone().set_num_format("[$€-813] #,##0.00;[Red]-[$€-813] #,##0.00");
    let date = cell.clone().set_num_format("dd/mm/yyyy").set_align(FormatAlign::Center);
    let pct = cell.clone().set_num_format("0.0%");
    let total = Format::new().set_bold().set_border_top(FormatBorder::Double).set_num_format("[$€-813] #,##0.00");

    ws.write_with_format(0, 0, "Bijlage 1 : leden 2026", &title)?;
    ws.merge_range(0, 7, 0, 9, "Overzicht per 27/11/2026", &Format::new().set_italic().set_align(FormatAlign::Right))?;
    let headers = [
        "ID\nlid",
        "Naam\nlid",
        "Voornaam\nlid",
        "Straat\nlid",
        "Huisnummer\nlid",
        "Lid sinds",
        "Bijdrage",
        "Aanwezig",
        "Status",
        "Notities",
    ];
    for (c, h) in headers.iter().enumerate() {
        ws.write_with_format(1, c as u16, *h, if c == 9 { &head_dark } else { &head })?;
    }
    let people = [
        (1989680, " Tawfik", "Ilyass", "Patriottenstraat", "1"),
        (1984715, "A de xia", "Kunga Tsering", "Mariakerkelaan", "56"),
        (1954953, "Aabbou", "Ayoub", "Abdijstraat", "72"),
        (233570, "Aabbou", "Sara", "Abdijstraat", "72"),
        (1976039, "Aadmi", "Safwan", "Frans Hensstraat", "44"),
        (2000094, "Aadmi", "Tasnim", "Folke Bernadottelaan", "25"),
        (1983203, "Aafjes", "Raydjen", "Oude gendarmeriestraat", "14a"),
        (1983204, "Aafjes", "Rilana", "Oude Gendarmeriestraat", "14a"),
        (1992591, "Aajour", "Roukia", "Turnhoutsebaan", "203"),
        (232701, "Aamara", "Adam", "Minister Delbekelaan", "12"),
        (277053, "Aamara", "Elyas", "Minister Delbekelaan", "12"),
        (1978433, "Aamara", "Rayan", "Portugeesstraat", "9"),
    ];
    for (i, (id, naam, voornaam, straat, nr)) in people.iter().enumerate() {
        let r = i as u32 + 2;
        ws.write_with_format(r, 0, *id as f64, &cell)?;
        ws.write_with_format(r, 1, *naam, &cell)?;
        ws.write_with_format(r, 2, *voornaam, &blue)?;
        ws.write_with_format(r, 3, *straat, &wrap)?;
        ws.write_with_format(r, 4, *nr, &cell)?;
        ws.write_with_format(r, 5, 44000.0 + (i * 397) as f64, &date)?;
        ws.write_with_format(r, 6, if i == 3 { -12.5 } else { 25.0 + i as f64 * 7.5 }, &money)?;
        ws.write_with_format(r, 7, 0.35 + (i % 7) as f64 * 0.1, &pct)?;
        ws.write_with_format(r, 8, if i % 4 == 0 { "ok" } else { "nieuw" }, &cell)?;
        ws.write_with_format(r, 9, if i == 1 { "Verhuisd in oktober, nieuw adres nog niet bevestigd" } else { "" }, &wrap)?;
        ws.set_row_height(r, 32)?;
    }
    let last = people.len() as u32 + 1;
    ws.write_with_format(last + 1, 5, "Totaal", &Format::new().set_bold().set_align(FormatAlign::Right))?;
    let sum: f64 = (0..people.len()).map(|i| if i == 3 { -12.5 } else { 25.0 + i as f64 * 7.5 }).sum();
    let f = rust_xlsxwriter::Formula::new(format!("=SUM(G3:G{})", last + 1)).set_result(sum.to_string());
    ws.write_formula_with_format(last + 1, 6, f, &total)?;

    ws.set_row_height(1, 34)?;
    for (c, w) in [10.5, 18.0, 16.0, 22.0, 12.0, 11.0, 12.0, 10.0, 9.0, 28.0].iter().enumerate() {
        ws.set_column_width(c as u16, *w)?;
    }
    ws.set_freeze_panes(2, 1)?;
    ws.autofilter(1, 0, last, 9)?;
    let low = Format::new().set_background_color(Color::RGB(0xFFC7CE)).set_font_color(Color::RGB(0x9C0006));
    ws.add_conditional_format(
        2,
        7,
        last,
        7,
        &ConditionalFormatCell::new().set_rule(ConditionalFormatCellRule::LessThan(0.5)).set_format(low),
    )?;
    let ok = Format::new().set_background_color(Color::RGB(0xC6EFCE)).set_font_color(Color::RGB(0x006100));
    ws.add_conditional_format(
        2,
        8,
        last,
        8,
        &ConditionalFormatText::new().set_rule(ConditionalFormatTextRule::Contains("ok".into())).set_format(ok),
    )?;
    wb.save(&path)?;
    println!("wrote {path}");
    Ok(())
}
