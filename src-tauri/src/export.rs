//! Writes a result grid to CSV, JSON or Excel.

use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Json,
    Xlsx,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Xlsx => "xlsx",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::Xlsx => "Excel",
        }
    }
}

/// Binary cells (`{"$bin": n, "hex": …}`) export as their hex preview.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(o) if o.contains_key("$bin") => Some(format!(
            "0x{}",
            o.get("hex").and_then(Value::as_str).unwrap_or("")
        )),
        v => Some(v.to_string()),
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

pub fn write(path: &Path, format: Format, columns: &[String], rows: &[Vec<Value>]) -> Result<()> {
    match format {
        Format::Csv => {
            // A BOM so Excel opens UTF-8 text correctly.
            let mut out = String::from("\u{feff}");
            let line = |cells: Vec<String>| cells.join(",") + "\r\n";
            out += &line(columns.iter().map(|c| csv_field(c)).collect());
            for row in rows {
                out += &line(
                    row.iter()
                        .map(|v| text(v).map(|s| csv_field(&s)).unwrap_or_default())
                        .collect(),
                );
            }
            std::fs::write(path, out)?;
        }
        Format::Json => {
            let objects: Vec<Value> = rows
                .iter()
                .map(|row| {
                    let mut m = Map::new();
                    for (c, v) in columns.iter().zip(row) {
                        let v = match v {
                            Value::Object(_) => text(v).map(Value::String).unwrap_or(Value::Null),
                            v => v.clone(),
                        };
                        m.insert(c.clone(), v);
                    }
                    Value::Object(m)
                })
                .collect();
            std::fs::write(path, serde_json::to_string_pretty(&objects)?)?;
        }
        Format::Xlsx => {
            use rust_xlsxwriter::{Format as Style, Workbook};
            let mut book = Workbook::new();
            let sheet = book.add_worksheet();
            let bold = Style::new().set_bold();
            for (c, name) in columns.iter().enumerate() {
                sheet.write_string_with_format(0, c as u16, name, &bold)?;
            }
            for (r, row) in rows.iter().enumerate() {
                let r = r as u32 + 1;
                for (c, v) in row.iter().enumerate() {
                    let c = c as u16;
                    match v {
                        Value::Null => {}
                        Value::Bool(b) => {
                            sheet.write_boolean(r, c, *b)?;
                        }
                        Value::Number(n) => match n.as_f64() {
                            Some(f) => {
                                sheet.write_number(r, c, f)?;
                            }
                            None => {
                                sheet.write_string(r, c, n.to_string())?;
                            }
                        },
                        v => {
                            sheet.write_string(r, c, text(v).unwrap_or_default())?;
                        }
                    }
                }
            }
            sheet.set_freeze_panes(1, 0)?;
            sheet.autofit();
            book.save(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_csv_fields_that_need_it() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
