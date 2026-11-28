//! Shapes of workbooks that break naive engines: very long dependency chains in both
//! directions, wide fan-in/fan-out, and the local clock.

use speedsheet_core::Workbook;

fn show(wb: &Workbook, r: usize, c: usize) -> String {
    wb.sheets[0].value(r, c).display()
}

/// A1 = A2+1, A2 = A3+1, ... : evaluation starts at the top and has to go all the way down.
fn upward_chain(n: usize) -> Workbook {
    let mut wb = Workbook::new();
    let mut grid: Vec<Vec<String>> = (1..n).map(|r| vec![format!("=A{}+1", r + 1)]).collect();
    grid.push(vec!["1".into()]);
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    wb
}

#[test]
fn upward_chain_short() {
    let mut wb = upward_chain(1_000);
    assert_eq!(show(&wb, 0, 0), "1000");
    wb.set_inputs(0, 999, 0, &[vec!["11".into()]]).unwrap();
    assert_eq!(show(&wb, 0, 0), "1010");
}

/// Feedback bug #5: this gave a false #CYCLE! when recalculation was recursive.
#[test]
fn upward_chain_long() {
    let mut wb = upward_chain(25_000);
    assert_eq!(show(&wb, 0, 0), "25000");
    wb.set_inputs(0, 24_999, 0, &[vec!["11".into()]]).unwrap();
    assert_eq!(show(&wb, 0, 0), "25010");
}

#[test]
fn fan_in_and_fan_out() {
    let n = 5_000;
    let mut wb = Workbook::new();
    // Column A: inputs. B: each reads its A and the total in C1. C1: sum of all of A.
    let mut grid: Vec<Vec<String>> = (0..n).map(|r| vec!["1".into(), format!("=A{}+$C$1", r + 1)]).collect();
    grid[0].push(format!("=SUM(A1:A{n})"));
    wb.set_inputs(0, 0, 0, &grid).unwrap();
    assert_eq!(show(&wb, 0, 2), n.to_string());
    assert_eq!(show(&wb, n - 1, 1), (n + 1).to_string());
    wb.set_inputs(0, 0, 0, &[vec!["101".into()]]).unwrap();
    assert_eq!(show(&wb, 0, 2), (n + 100).to_string());
    assert_eq!(show(&wb, n - 1, 1), (n + 101).to_string());
}

#[test]
fn real_cycle_is_still_detected() {
    let mut wb = Workbook::new();
    wb.set_inputs(0, 0, 0, &[vec!["=B1+1".into(), "=C1+1".into(), "=A1+1".into()]]).unwrap();
    assert_eq!(show(&wb, 0, 0), "#CYCLE!");
}

/// TODAY() must be the date on the user's clock, not in UTC. CI runs this in time zones far
/// from UTC (UTC+14 and UTC-11: at any moment one of them is on another date than UTC).
/// Feedback bug #6.
#[cfg(unix)]
#[test]
fn today_is_local_date() {
    let local = std::process::Command::new("date").arg("+%Y-%m-%d").output().expect("date command");
    let local = String::from_utf8(local.stdout).unwrap().trim().to_string();
    let wb = Workbook::new();
    let today = wb.evaluate(0, "=TEXT(TODAY(),\"yyyy-mm-dd\")").display();
    // Around midnight the two calls can straddle it; allow that one second of slack.
    let again = String::from_utf8(std::process::Command::new("date").arg("+%Y-%m-%d").output().unwrap().stdout).unwrap();
    assert!(today == local || today == again.trim(), "TODAY() = {today}, local date = {local}");
}
