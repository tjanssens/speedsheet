//! Property-based fuzzing of everything that parses text: formulas, user input and reference
//! shifting. A panic here would crash the app on a keystroke or on opening a file.
//! The nightly workflow runs these with many more cases (PROPTEST_CASES).

use proptest::prelude::*;
use speedsheet_core::formula::parser::{parse, shift_formula, tokenize};
use speedsheet_core::input::parse_input;
use speedsheet_core::{Cell, Value, Workbook};

/// Strings made of the characters formulas are built from, so the parser gets deep.
fn formulaish() -> impl Strategy<Value = String> {
    prop::string::string_regex(r#"[A-Z$0-9:!,;.+\-*/^&=<>%(){}"' #_a-z]{0,40}"#).unwrap()
}

fn cell_ref(rows: std::ops::Range<u32>) -> impl Strategy<Value = String> {
    (prop::sample::select(vec!["C", "D", "Z", "AA", "XFB"]), rows, any::<bool>(), any::<bool>())
        .prop_map(|(c, r, ac, ar)| format!("{}{c}{}{r}", if ac { "$" } else { "" }, if ar { "$" } else { "" }))
}

fn formula_with_refs() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        cell_ref(100..200),
        (cell_ref(100..150), cell_ref(150..200)).prop_map(|(a, b)| format!("SUM({a}:{b})")),
        cell_ref(100..200).prop_map(|a| format!("Sheet2!{a}")),
        cell_ref(100..200).prop_map(|a| format!("'My Sheet'!{a}")),
        Just("\"A1\"".to_string()),
        (0..1000i32).prop_map(|n| n.to_string()),
    ];
    prop::collection::vec((piece, prop::sample::select(vec!["+", "-", "*", "&", ","])), 1..6).prop_map(|parts| {
        let mut s = String::new();
        for (i, (p, op)) in parts.iter().enumerate() {
            if i > 0 {
                s.push_str(op);
            }
            s.push_str(p);
        }
        s
    })
}

/// Failing cases are stored next to this file (`fuzz.proptest-regressions`, committed) and
/// replayed first on every run.
fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(prop::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn parser_never_panics(src in formulaish()) {
        let _ = tokenize(&src);
        let _ = parse(&src);
    }

    #[test]
    fn parser_never_panics_on_any_text(src in any::<String>()) {
        let _ = tokenize(&src);
        let _ = parse(&src);
    }

    #[test]
    fn evaluating_never_panics(src in formulaish()) {
        let mut wb = Workbook::new();
        wb.set_inputs(0, 0, 0, &[vec!["1".into(), "x".into(), "=A1*2".into()]]).unwrap();
        let _ = wb.evaluate(0, &src);
        // Also as a stored formula, which goes through the dependency index and recalculation.
        let _ = wb.set_inputs(0, 1, 0, &[vec![format!("={src}")]]);
    }

    /// Number formats come from files and from TEXT(); none may crash the grid.
    #[test]
    fn number_formats_never_panic(
        code in prop_oneof![
            prop::string::string_regex(r#"[0#?.,%Ee+\-/ :;@"*_\[\]hmsdyAPMGeneral$€é]{0,24}"#).unwrap(),
            any::<String>(),
        ],
        n in prop_oneof![any::<f64>(), -1e7..1e7f64, 0.0..60000.0f64],
    ) {
        let _ = speedsheet_core::numfmt::format_value(&Value::Number(n), &code);
        let _ = speedsheet_core::numfmt::format_value(&Value::Date(n), &code);
        let _ = speedsheet_core::numfmt::format_value(&Value::text("abc"), &code);
        let _ = speedsheet_core::numfmt::is_date_format(&code);
    }

    /// Copying a formula down and back up gives the original formula back.
    #[test]
    fn shift_and_shift_back_is_identity(src in formula_with_refs(), dr in -50i64..50, dc in -2i64..=2) {
        // Columns C..XFB and rows 100..200 stay on the sheet; a #REF! could not come back.
        let there = shift_formula(&src, dr, dc);
        prop_assume!(!there.contains("#REF!"));
        prop_assert_eq!(shift_formula(&there, -dr, -dc), src);
    }

    /// Editing a cell and pressing Enter without changes must not change it.
    #[test]
    fn edit_text_round_trips(v in prop_oneof![
        any::<f64>().prop_filter("finite", |n| n.is_finite()).prop_map(Value::Number),
        (-1e6..1e6f64).prop_map(Value::Number),
        "[ -~]{1,12}".prop_map(|s| Value::text(s.as_str())),
        any::<bool>().prop_map(Value::Bool),
    ]) {
        let cell = Cell::from_value(v);
        let text = cell.input_text();
        let back = parse_input(&text);
        prop_assert_eq!(&back, &cell, "input text {:?}", text);
    }
}
