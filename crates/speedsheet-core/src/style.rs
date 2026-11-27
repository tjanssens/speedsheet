//! Cell formatting: a compact style table shared by a workbook, read from and written to xlsx.
//!
//! Each sheet stores a style id per cell (0 = default); the ids index `Workbook::styles`.

use std::sync::Arc;

use serde::Serialize;

/// Horizontal alignment. `General` means: numbers right, text left, like Excel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HAlign {
    #[default]
    General,
    Left,
    Center,
    Right,
    Justify,
    Fill,
    CenterAcross,
    Distributed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    #[default]
    Bottom,
    Center,
    Top,
    Justify,
    Distributed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Hash)]
#[serde(rename_all = "camelCase")]
pub enum LineStyle {
    Thin,
    Medium,
    Thick,
    Double,
    Hair,
    Dashed,
    Dotted,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Hash)]
pub struct BorderLine {
    pub style: LineStyle,
    /// 0xRRGGBB
    pub color: u32,
}

/// Everything SpeedSheet keeps of a cell format. Colors are 0xRRGGBB.
#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellStyle {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<Arc<str>>,
    /// Font size in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<u32>,
    pub h_align: HAlign,
    pub v_align: VAlign,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub wrap: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub indent: u8,
    /// Number format code; None = General.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_fmt: Option<Arc<str>>,
    /// Left, right, top, bottom.
    pub borders: [Option<BorderLine>; 4],
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

impl CellStyle {
    pub fn is_default(&self) -> bool {
        *self == CellStyle::default()
    }

    /// The number format to display values with, if it is not General.
    pub fn number_format(&self) -> Option<&str> {
        self.num_fmt.as_deref().filter(|f| !f.eq_ignore_ascii_case("general"))
    }

    pub fn from_calamine(s: &calamine::Style) -> CellStyle {
        use calamine::{BorderStyle, FillPattern, HorizontalAlignment as H, VerticalAlignment as V};
        let rgb = |c: &calamine::Color| ((c.red as u32) << 16) | ((c.green as u32) << 8) | c.blue as u32;
        let mut out = CellStyle::default();
        if let Some(f) = &s.font {
            out.bold = f.is_bold();
            out.italic = f.is_italic();
            out.underline = f.has_underline();
            out.strike = f.has_strikethrough();
            out.font_name = f.name.as_deref().map(Arc::from);
            out.font_size = f.size.map(|v| v as f32);
            // Black is the automatic text color; keeping it as None lets dark mode invert it.
            out.color = f.color.as_ref().map(rgb).filter(|&c| c != 0);
        }
        if let Some(fill) = &s.fill {
            if fill.pattern != FillPattern::None {
                // For a solid fill the foreground color is the cell color.
                out.fill = fill.get_color().map(|c| rgb(&c));
            }
        }
        if let Some(a) = &s.alignment {
            out.h_align = match a.horizontal {
                H::Left => HAlign::Left,
                H::Center => HAlign::Center,
                H::CenterContinuous => HAlign::CenterAcross,
                H::Right => HAlign::Right,
                H::Justify => HAlign::Justify,
                H::Distributed => HAlign::Distributed,
                H::Fill => HAlign::Fill,
                _ => HAlign::General,
            };
            out.v_align = match a.vertical {
                V::Top => VAlign::Top,
                V::Center => VAlign::Center,
                V::Justify => VAlign::Justify,
                V::Distributed => VAlign::Distributed,
                _ => VAlign::Bottom,
            };
            out.wrap = a.wrap_text;
            out.indent = a.indent.unwrap_or(0);
        }
        if let Some(n) = &s.number_format {
            if n.format_id != Some(0) && !n.format_code.eq_ignore_ascii_case("general") {
                out.num_fmt = Some(Arc::from(n.format_code.as_str()));
            }
        }
        if let Some(b) = &s.borders {
            let conv = |b: &calamine::Border| -> Option<BorderLine> {
                let style = match b.style {
                    BorderStyle::None => return None,
                    BorderStyle::Thin => LineStyle::Thin,
                    BorderStyle::Medium => LineStyle::Medium,
                    BorderStyle::Thick => LineStyle::Thick,
                    BorderStyle::Double => LineStyle::Double,
                    BorderStyle::Hair => LineStyle::Hair,
                    BorderStyle::Dashed => LineStyle::Dashed,
                    BorderStyle::Dotted => LineStyle::Dotted,
                    BorderStyle::MediumDashed => LineStyle::MediumDashed,
                    BorderStyle::DashDot => LineStyle::DashDot,
                    BorderStyle::MediumDashDot => LineStyle::MediumDashDot,
                    BorderStyle::DashDotDot => LineStyle::DashDotDot,
                    BorderStyle::MediumDashDotDot => LineStyle::MediumDashDotDot,
                    BorderStyle::SlantDashDot => LineStyle::SlantDashDot,
                    _ => LineStyle::Thin,
                };
                Some(BorderLine { style, color: b.color.as_ref().map(rgb).unwrap_or(0) })
            };
            out.borders = [conv(&b.left), conv(&b.right), conv(&b.top), conv(&b.bottom)];
        }
        out
    }

    /// The equivalent rust_xlsxwriter format. Dates without a number format get `date_fmt`.
    pub(crate) fn to_xlsx(&self) -> rust_xlsxwriter::Format {
        use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, FormatPattern, FormatUnderline};
        let mut f = Format::new();
        if self.bold {
            f = f.set_bold();
        }
        if self.italic {
            f = f.set_italic();
        }
        if self.underline {
            f = f.set_underline(FormatUnderline::Single);
        }
        if self.strike {
            f = f.set_font_strikethrough();
        }
        if let Some(n) = &self.font_name {
            f = f.set_font_name(n.as_ref());
        }
        if let Some(s) = self.font_size {
            f = f.set_font_size(s as f64);
        }
        if let Some(c) = self.color {
            f = f.set_font_color(Color::RGB(c));
        }
        if let Some(c) = self.fill {
            f = f.set_pattern(FormatPattern::Solid).set_background_color(Color::RGB(c));
        }
        f = match self.h_align {
            HAlign::General => f,
            HAlign::Left => f.set_align(FormatAlign::Left),
            HAlign::Center => f.set_align(FormatAlign::Center),
            HAlign::Right => f.set_align(FormatAlign::Right),
            HAlign::Justify => f.set_align(FormatAlign::Justify),
            HAlign::Fill => f.set_align(FormatAlign::Fill),
            HAlign::CenterAcross => f.set_align(FormatAlign::CenterAcross),
            HAlign::Distributed => f.set_align(FormatAlign::Distributed),
        };
        f = match self.v_align {
            VAlign::Bottom => f,
            VAlign::Center => f.set_align(FormatAlign::VerticalCenter),
            VAlign::Top => f.set_align(FormatAlign::Top),
            VAlign::Justify => f.set_align(FormatAlign::VerticalJustify),
            VAlign::Distributed => f.set_align(FormatAlign::VerticalDistributed),
        };
        if self.wrap {
            f = f.set_text_wrap();
        }
        if self.indent > 0 {
            f = f.set_indent(self.indent);
        }
        if let Some(n) = self.number_format() {
            f = f.set_num_format(n);
        }
        let line = |b: &BorderLine| match b.style {
            LineStyle::Thin => FormatBorder::Thin,
            LineStyle::Medium => FormatBorder::Medium,
            LineStyle::Thick => FormatBorder::Thick,
            LineStyle::Double => FormatBorder::Double,
            LineStyle::Hair => FormatBorder::Hair,
            LineStyle::Dashed => FormatBorder::Dashed,
            LineStyle::Dotted => FormatBorder::Dotted,
            LineStyle::MediumDashed => FormatBorder::MediumDashed,
            LineStyle::DashDot => FormatBorder::DashDot,
            LineStyle::MediumDashDot => FormatBorder::MediumDashDot,
            LineStyle::DashDotDot => FormatBorder::DashDotDot,
            LineStyle::MediumDashDotDot => FormatBorder::MediumDashDotDot,
            LineStyle::SlantDashDot => FormatBorder::SlantDashDot,
        };
        let [l, r, t, b] = &self.borders;
        if let Some(x) = l {
            f = f.set_border_left(line(x)).set_border_left_color(Color::RGB(x.color));
        }
        if let Some(x) = r {
            f = f.set_border_right(line(x)).set_border_right_color(Color::RGB(x.color));
        }
        if let Some(x) = t {
            f = f.set_border_top(line(x)).set_border_top_color(Color::RGB(x.color));
        }
        if let Some(x) = b {
            f = f.set_border_bottom(line(x)).set_border_bottom_color(Color::RGB(x.color));
        }
        f
    }
}
