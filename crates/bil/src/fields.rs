// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Metadata fields and their definitions (specification, section 14.1).
//!
//! A field has a name, a list of parts beginning with the namespace; stored bytes; and, where a
//! definition names it, a value type, an encoding, a unit, a handling and the dictionary item it
//! provides. Its value is decoded from the stored bytes when it is first read (section 14.3,
//! rule 4), and held.

use std::fmt;
use std::sync::OnceLock;

/// Where a field's name comes from (section 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    SelfDescribed,
    Specified,
    Derived,
    Companion,
    Caller,
}

/// What a writer does with a field (sections 11 and 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handling {
    Ordinary,
    Marking,
    Identity,
}

/// The type of a field's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Text,
    Integer,
    Float,
    Boolean,
    DateTime,
    Bytes,
}

/// How a field's stored bytes become its value. [`Encoding::name`] gives the name a definition
/// in a file uses (section 14.1, "The form of a field definition").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Text in ISO 8859-1, of which ASCII is a part, padded on the right with spaces.
    Latin1Text,
    /// A whole number written in ASCII digits, with an optional sign and spaces around it.
    AsciiDecimal,
    /// A number written in ASCII, with a decimal point or an exponent.
    AsciiFloat,
    /// The bytes as they are.
    Binary,
    /// A date and time as `CCYYMMDDhhmmss`, in UTC.
    Ccyymmddhhmmss,
    /// A date and time as `DDhhmmssZMONYY`, in UTC: day, time, the letter Z, the month's three
    /// letters and the year's last two digits.
    Ddhhmmsszmonyy,
}

impl Encoding {
    /// The encoding's name in a definition kept in a file.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Latin1Text => "latin1_text",
            Encoding::AsciiDecimal => "ascii_decimal",
            Encoding::AsciiFloat => "ascii_float",
            Encoding::Binary => "binary",
            Encoding::Ccyymmddhhmmss => "ccyymmddhhmmss",
            Encoding::Ddhhmmsszmonyy => "ddhhmmsszmonyy",
        }
    }
}

/// A date and time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: f64,
    /// The offset from UTC the file stated, in minutes, where the zone is known; then the
    /// fields above are in UTC. `None` means local time with no known zone, never taken as UTC
    /// (section 14.1).
    pub utc_offset_minutes: Option<i32>,
}

/// A field's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    DateTime(DateTime),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    /// A known field that holds no value, such as a NITF field left blank: delivered as
    /// `Missing` (section 14.4, `read_field`).
    Missing,
}

/// A field definition (section 14.1): what a code or a position in a format means. The
/// locator, which says where the stored bytes are, belongs to the format and its driver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDef {
    /// The native name, exactly as the format's specification spells it.
    pub name: &'static str,
    pub value_type: ValueType,
    pub encoding: Encoding,
    pub unit: Option<&'static str>,
    pub handling: Handling,
    /// The dictionary item this field provides, where it is plainly the same (section 14.2).
    pub provides: Option<&'static str>,
}

/// A field's name: a namespace, then the native name, as a list of parts (section 14.1). A part
/// that repeats carries its number, counted from 1, in square brackets.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldName(pub Vec<String>);

impl FieldName {
    /// A name from its parts.
    pub fn new<S: Into<String>>(parts: impl IntoIterator<Item = S>) -> FieldName {
        FieldName(parts.into_iter().map(Into::into).collect())
    }

    /// The dotted form: parts joined by dots, a dot inside a part written `\.`.
    pub fn dotted(&self) -> String {
        self.0
            .iter()
            .map(|p| p.replace('.', "\\."))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// A name from its dotted form, the inverse of [`FieldName::dotted`].
    pub fn parse(dotted: &str) -> FieldName {
        let mut parts = vec![String::new()];
        let mut chars = dotted.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&'.') => {
                    chars.next();
                    parts.last_mut().unwrap().push('.');
                }
                '.' => parts.push(String::new()),
                _ => parts.last_mut().unwrap().push(c),
            }
        }
        FieldName(parts)
    }

    /// Whether this name begins with all the parts of `prefix`.
    pub fn starts_with(&self, prefix: &FieldName) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }

    /// This name with one more part.
    pub fn child(&self, part: impl Into<String>) -> FieldName {
        let mut parts = self.0.clone();
        parts.push(part.into());
        FieldName(parts)
    }
}

impl fmt::Display for FieldName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.dotted())
    }
}

/// A field: its name, its stored bytes, and its definition where one names it. Without a
/// definition the value is the stored bytes ("Nothing is hidden", section 14.1).
#[derive(Debug)]
pub struct Field {
    pub name: FieldName,
    pub stored: Vec<u8>,
    pub definition: Option<FieldDef>,
    pub origin: Origin,
    value: OnceLock<Value>,
}

impl Clone for Field {
    fn clone(&self) -> Field {
        let value = OnceLock::new();
        if let Some(v) = self.value.get() {
            let _ = value.set(v.clone());
        }
        Field {
            name: self.name.clone(),
            stored: self.stored.clone(),
            definition: self.definition,
            origin: self.origin,
            value,
        }
    }
}

impl Field {
    pub fn new(
        name: FieldName,
        stored: Vec<u8>,
        definition: Option<FieldDef>,
        origin: Origin,
    ) -> Field {
        Field {
            name,
            stored,
            definition,
            origin,
            value: OnceLock::new(),
        }
    }

    /// A field whose value the file types itself, as a TIFF tag's entry does: the value is held
    /// from the start, beside the stored bytes it came from.
    pub fn with_value(name: FieldName, stored: Vec<u8>, value: Value, origin: Origin) -> Field {
        let field = Field::new(name, stored, None, origin);
        let _ = field.value.set(value);
        field
    }

    /// A defined field with no value here: its value is `Missing` (section 3.3).
    pub fn missing(name: FieldName, definition: Option<FieldDef>, origin: Origin) -> Field {
        let field = Field::new(name, Vec::new(), definition, origin);
        let _ = field.value.set(Value::Missing);
        field
    }

    /// The value, decoded from the stored bytes on first use and then held. Bytes that do not
    /// decode as the definition says are delivered as `Bytes`, so that nothing is lost.
    pub fn value(&self) -> &Value {
        self.value.get_or_init(|| match &self.definition {
            Some(d) => decode(&self.stored, d.value_type, d.encoding)
                .unwrap_or_else(|| Value::Bytes(self.stored.clone())),
            None => Value::Bytes(self.stored.clone()),
        })
    }

    pub fn unit(&self) -> Option<&'static str> {
        self.definition.and_then(|d| d.unit)
    }

    pub fn handling(&self) -> Handling {
        self.definition.map_or(Handling::Ordinary, |d| d.handling)
    }
}

/// A group of fields: one unit of the format, extracted together (section 14.3, rule 2).
#[derive(Clone, Debug)]
pub struct FieldGroup {
    pub name: FieldName,
    pub fields: Vec<Field>,
    /// `Complete`, or `Partial` where some items of the group could not be read (section 3.3);
    /// `problem` then says what.
    pub status: crate::types::Status,
    pub problem: Option<String>,
}

impl FieldGroup {
    /// The field whose name is this group's name with `part` added.
    pub fn get(&self, part: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.name.0.len() == self.name.0.len() + 1 && f.name.0.last().unwrap() == part)
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b == b' ')
}

fn digits(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Decodes stored bytes as `value_type` in `encoding`. `None` means the bytes do not follow the
/// encoding; blank bytes in any encoding but `Binary` are `Missing`.
pub fn decode(stored: &[u8], value_type: ValueType, encoding: Encoding) -> Option<Value> {
    if encoding != Encoding::Binary && blank(stored) {
        return Some(Value::Missing);
    }
    let text = || latin1(stored).trim().to_string();
    let value = match (encoding, value_type) {
        (Encoding::Binary, _) => Value::Bytes(stored.to_vec()),
        (Encoding::Latin1Text, ValueType::Text) => {
            Value::Text(latin1(stored).trim_end_matches(' ').to_string())
        }
        (Encoding::AsciiDecimal, ValueType::Integer) => Value::Integer(text().parse().ok()?),
        (Encoding::AsciiDecimal | Encoding::AsciiFloat, ValueType::Float) => {
            let t = text();
            // Plain decimal notation only: no "inf", "nan" or hexadecimal, which Rust's parser
            // would accept and no format here writes.
            if !t
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'E' | b'e'))
            {
                return None;
            }
            Value::Float(t.parse().ok()?)
        }
        (Encoding::Ccyymmddhhmmss, ValueType::DateTime) => {
            if stored.len() != 14 {
                return None;
            }
            date_time(
                digits(&stored[0..4])? as i32,
                digits(&stored[4..6])?,
                digits(&stored[6..8])?,
                &stored[8..14],
            )?
        }
        (Encoding::Ddhhmmsszmonyy, ValueType::DateTime) => {
            if stored.len() != 14 || stored[8] != b'Z' {
                return None;
            }
            const MONTHS: [&[u8; 3]; 12] = [
                b"JAN", b"FEB", b"MAR", b"APR", b"MAY", b"JUN", b"JUL", b"AUG", b"SEP", b"OCT",
                b"NOV", b"DEC",
            ];
            let month = MONTHS.iter().position(|m| m[..] == stored[9..12])? as u32 + 1;
            // Two-digit years: 1950 to 2049, the usual window for files of this form.
            let yy = digits(&stored[12..14])? as i32;
            let year = if yy < 50 { 2000 + yy } else { 1900 + yy };
            let mut time = [0u8; 6];
            time.copy_from_slice(&stored[2..8]);
            date_time(year, month, digits(&stored[0..2])?, &time)?
        }
        _ => return None,
    };
    Some(value)
}

/// A UTC date and time from its parts, with `hhmmss` as six ASCII digits. `None` where any part
/// is out of range.
fn date_time(year: i32, month: u32, day: u32, hhmmss: &[u8]) -> Option<Value> {
    let (hour, minute, second) = (
        digits(&hhmmss[0..2])?,
        digits(&hhmmss[2..4])?,
        digits(&hhmmss[4..6])?,
    );
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    // A leap second is 60.
    if !(1..=12).contains(&month)
        || day == 0
        || day > days[month as usize - 1]
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(Value::DateTime(DateTime {
        year,
        month: month as u8,
        day: day as u8,
        hour: hour as u8,
        minute: minute as u8,
        second: f64::from(second),
        utc_offset_minutes: Some(0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_names_round_trip() {
        let n = FieldName::new(["hdf5", "calibration", "band.1"]);
        assert_eq!(n.dotted(), "hdf5.calibration.band\\.1");
        assert_eq!(FieldName::parse(&n.dotted()), n);
        let r = FieldName::parse("nitf.image_subheader.IREPBAND[2]");
        assert_eq!(r.0, ["nitf", "image_subheader", "IREPBAND[2]"]);
    }

    #[test]
    fn values_decode() {
        use Encoding::*;
        use ValueType::*;
        assert_eq!(
            decode(b"  42", Integer, AsciiDecimal),
            Some(Value::Integer(42))
        );
        assert_eq!(
            decode(b"-0042", Integer, AsciiDecimal),
            Some(Value::Integer(-42))
        );
        assert_eq!(decode(b"    ", Integer, AsciiDecimal), Some(Value::Missing));
        assert_eq!(decode(b"4x", Integer, AsciiDecimal), None);
        assert_eq!(
            decode(b"+1.5E+02", Float, AsciiFloat),
            Some(Value::Float(150.0))
        );
        assert_eq!(decode(b"inf", Float, AsciiFloat), None);
        assert_eq!(
            decode(b"Title   ", Text, Latin1Text),
            Some(Value::Text("Title".into()))
        );
        assert_eq!(
            decode(&[0xE9, b' '], Text, Latin1Text),
            Some(Value::Text("é".into()))
        );
        let Some(Value::DateTime(t)) = decode(b"20261014093015", DateTime, Ccyymmddhhmmss) else {
            panic!()
        };
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (2026, 10, 14, 9, 30, 15.0)
        );
        assert_eq!(t.utc_offset_minutes, Some(0));
        let Some(Value::DateTime(t)) = decode(b"14093015ZOCT97", DateTime, Ddhhmmsszmonyy) else {
            panic!()
        };
        assert_eq!((t.year, t.month, t.day, t.hour), (1997, 10, 14, 9));
        assert_eq!(decode(b"20260230093015", DateTime, Ccyymmddhhmmss), None);
        assert_eq!(decode(b"2026----------", DateTime, Ccyymmddhhmmss), None);
    }

    #[test]
    fn a_field_without_definition_is_its_bytes() {
        let f = Field::new(
            FieldName::new(["nitf", "VNDRTA", "CEDATA"]),
            vec![1, 2, 3],
            None,
            Origin::Specified,
        );
        assert_eq!(f.value(), &Value::Bytes(vec![1, 2, 3]));
        assert_eq!(f.handling(), Handling::Ordinary);
    }
}
