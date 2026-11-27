//! Generate a large test workbook: `cargo run --release -p speedsheet-core --example gen_large -- [rows] [path]`
//! Default: 500 000 rows into testdata/large-500k.xlsx (10 columns, one formula column).

use rust_xlsxwriter::{Format, Formula, Workbook};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let rows: u32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(500_000);
    let path = args.next().unwrap_or_else(|| "testdata/large-500k.xlsx".into());
    let cities = ["Gent", "Antwerpen", "Brussel", "Leuven", "Brugge", "Hasselt", "Mechelen", "Kortrijk", "Namur", "Liège"];
    let cats = ["Hardware", "Software", "Service", "Training", "Support"];
    let names = ["Jansen", "Peeters", "Maes", "Jacobs", "Mertens", "Willems", "Claes", "Goossens", "Wouters", "De Smet"];
    let mut wb = Workbook::new();
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let ws = wb.add_worksheet_with_constant_memory()?;
    ws.set_name("Orders")?;
    for (c, h) in ["Id", "Date", "Customer", "City", "Category", "Price", "Qty", "Total", "Paid", "Note"].iter().enumerate() {
        ws.write_string(0, c as u16, *h)?;
    }
    let mut seed: u64 = 0x5eed;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for r in 1..=rows {
        let x = rnd();
        let price = ((x % 100_000) as f64) / 100.0;
        let qty = (x >> 20) % 50 + 1;
        ws.write_number(r, 0, r as f64)?;
        ws.write_number_with_format(r, 1, 45000.0 + ((x >> 8) % 1500) as f64, &date)?;
        ws.write_string(r, 2, format!("{} {}", names[(x >> 3) as usize % names.len()], (x >> 30) % 1000))?;
        ws.write_string(r, 3, cities[(x >> 12) as usize % cities.len()])?;
        ws.write_string(r, 4, cats[(x >> 16) as usize % cats.len()])?;
        ws.write_number(r, 5, price)?;
        ws.write_number(r, 6, qty as f64)?;
        let total = price * qty as f64;
        ws.write_formula(r, 7, Formula::new(format!("=F{}*G{}", r + 1, r + 1)).set_result(format!("{}", total)))?;
        ws.write_boolean(r, 8, x % 3 != 0)?;
        if x % 7 == 0 {
            ws.write_string(r, 9, "urgent")?;
        }
    }
    wb.save(&path)?;
    println!("wrote {rows} rows to {path}");
    Ok(())
}
