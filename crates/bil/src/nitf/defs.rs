// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The field definitions of NITF's headers and of the extensions BIL reads, and the walk that
//! reads a header through them.
//!
//! NITF's headers are fields one after another, each of a stated length. Some are present only
//! when an earlier field has a given value, some repeat as often as an earlier field says, and
//! some take their length from an earlier field. Section 14.1 describes such headers by an
//! ordered layout, and a layout here is one: a list of items read in order, each a field with
//! its length and the condition for its presence, or a run of items repeated a counted number of
//! times.
//!
//! The fields follow MIL-STD-2500A for NITF 2.0 and MIL-STD-2500C for NITF 2.1 and NSIF 1.0,
//! and STDI-0002 for the extensions. Native names are the standard's; a field in a counted run
//! carries its place in square brackets, counted from 1, one number for each run, as section
//! 14.1 says.

use crate::fields::{Encoding, FieldDef, Handling, ValueType};

/// A field's length in bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Length {
    Fixed(usize),
    /// The value of an earlier field, less a fixed number of bytes.
    FromField(&'static str, usize),
}

/// When a field is present.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Presence {
    Always,
    /// Unless an earlier field's stored bytes are one of these.
    Unless(&'static str, &'static [&'static [u8]]),
    /// Only when an earlier field's stored bytes are one of these.
    When(&'static str, &'static [&'static [u8]]),
    /// Only when an earlier field's value is above zero.
    Positive(&'static str),
}

/// How many times a run of items repeats.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Count {
    Fixed(usize),
    /// An earlier field's value.
    Field(&'static str),
    /// An earlier field's value, or where that is 0, another's: NITF 2.1's NBANDS and XBANDS.
    FieldElse(&'static str, &'static str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Item {
    Field {
        def: FieldDef,
        length: Length,
        presence: Presence,
    },
    Repeat {
        count: Count,
        items: &'static [Item],
    },
}

const fn field(name: &'static str, len: usize, value_type: ValueType, encoding: Encoding) -> Item {
    Item::Field {
        def: FieldDef {
            name,
            value_type,
            encoding,
            unit: None,
            handling: Handling::Ordinary,
            provides: None,
        },
        length: Length::Fixed(len),
        presence: Presence::Always,
    }
}

/// Text, in ISO 8859-1 (NITF's BCS-A and ECS-A character sets are both within it).
const fn text(name: &'static str, len: usize) -> Item {
    field(name, len, ValueType::Text, Encoding::Latin1Text)
}

/// A whole number in ASCII digits (BCS-N).
const fn int(name: &'static str, len: usize) -> Item {
    field(name, len, ValueType::Integer, Encoding::AsciiDecimal)
}

/// A number with a decimal point or an exponent.
const fn float(name: &'static str, len: usize) -> Item {
    field(name, len, ValueType::Float, Encoding::AsciiFloat)
}

/// Bytes, as they are.
const fn bytes(name: &'static str, len: usize) -> Item {
    field(name, len, ValueType::Bytes, Encoding::Binary)
}

const fn date(name: &'static str, encoding: Encoding) -> Item {
    field(name, 14, ValueType::DateTime, encoding)
}

/// A security marking: carried by every writer (section 14.3, rule 7).
const fn marking(name: &'static str, len: usize) -> Item {
    with_handling(text(name, len), Handling::Marking)
}

const fn with_handling(item: Item, handling: Handling) -> Item {
    match item {
        Item::Field {
            mut def,
            length,
            presence,
        } => {
            def.handling = handling;
            Item::Field {
                def,
                length,
                presence,
            }
        }
        other => other,
    }
}

const fn unit(item: Item, unit: &'static str) -> Item {
    match item {
        Item::Field {
            mut def,
            length,
            presence,
        } => {
            def.unit = Some(unit);
            Item::Field {
                def,
                length,
                presence,
            }
        }
        other => other,
    }
}

const fn provides(item: Item, entry: &'static str) -> Item {
    match item {
        Item::Field {
            mut def,
            length,
            presence,
        } => {
            def.provides = Some(entry);
            Item::Field {
                def,
                length,
                presence,
            }
        }
        other => other,
    }
}

const fn when(item: Item, presence: Presence) -> Item {
    match item {
        Item::Field { def, length, .. } => Item::Field {
            def,
            length,
            presence,
        },
        other => other,
    }
}

const fn sized(item: Item, length: Length) -> Item {
    match item {
        Item::Field { def, presence, .. } => Item::Field {
            def,
            length,
            presence,
        },
        other => other,
    }
}

/// A length in bytes, as the standard states for the segment and header lengths.
const fn length(name: &'static str, len: usize) -> Item {
    unit(int(name, len), "byte")
}

// ------------------------------------------------------------------ the file header

/// User-defined and extended header data, which hold extensions: a length, then where it is
/// above 0, the overflow segment's number and the data.
macro_rules! extension_data {
    ($len:literal, $ofl:literal, $data:literal) => {
        [
            length($len, 5),
            when(int($ofl, 3), Presence::Positive($len)),
            when(
                sized(bytes($data, 0), Length::FromField($len, 3)),
                Presence::Positive($len),
            ),
        ]
    };
}

const FILE_SEGMENTS_21: &[Item] = &[
    int("NUMI", 3),
    Item::Repeat {
        count: Count::Field("NUMI"),
        items: &[length("LISH", 6), length("LI", 10)],
    },
    int("NUMS", 3),
    Item::Repeat {
        count: Count::Field("NUMS"),
        items: &[length("LSSH", 4), length("LS", 6)],
    },
    // Reserved for future use: always 000.
    int("NUMX", 3),
    int("NUMT", 3),
    Item::Repeat {
        count: Count::Field("NUMT"),
        items: &[length("LTSH", 4), length("LT", 5)],
    },
    int("NUMDES", 3),
    Item::Repeat {
        count: Count::Field("NUMDES"),
        items: &[length("LDSH", 4), length("LD", 9)],
    },
    int("NUMRES", 3),
    Item::Repeat {
        count: Count::Field("NUMRES"),
        items: &[length("LRESH", 4), length("LRE", 7)],
    },
];

/// The NITF 2.1 and NSIF 1.0 file header, MIL-STD-2500C table A-1.
pub(crate) static FILE_HEADER_21: &[Item] = &[
    text("FHDR", 4),
    text("FVER", 5),
    int("CLEVEL", 2),
    text("STYPE", 4),
    text("OSTAID", 10),
    date("FDT", Encoding::Ccyymmddhhmmss),
    provides(text("FTITLE", 80), "common.title"),
    marking("FSCLAS", 1),
    marking("FSCLSY", 2),
    marking("FSCODE", 11),
    marking("FSCTLH", 2),
    marking("FSREL", 20),
    marking("FSDCTP", 2),
    marking("FSDCDT", 8),
    marking("FSDCXM", 4),
    marking("FSDG", 1),
    marking("FSDGDT", 8),
    marking("FSCLTX", 43),
    marking("FSCATP", 1),
    marking("FSCAUT", 40),
    marking("FSCRSN", 1),
    marking("FSSRDT", 8),
    marking("FSCTLN", 15),
    int("FSCOP", 5),
    int("FSCPYS", 5),
    int("ENCRYP", 1),
    bytes("FBKGC", 3),
    with_handling(text("ONAME", 24), Handling::Identity),
    with_handling(text("OPHONE", 18), Handling::Identity),
    length("FL", 12),
    length("HL", 6),
    Item::Repeat {
        count: Count::Fixed(1),
        items: FILE_SEGMENTS_21,
    },
];

/// The NITF 2.0 file header, MIL-STD-2500A table A-1. It has labels where 2.1 has a reserved
/// count, other security fields, and no background color.
pub(crate) static FILE_HEADER_20: &[Item] = &[
    text("FHDR", 4),
    text("FVER", 5),
    int("CLEVEL", 2),
    text("STYPE", 4),
    text("OSTAID", 10),
    date("FDT", Encoding::Ddhhmmsszmonyy),
    provides(text("FTITLE", 80), "common.title"),
    marking("FSCLAS", 1),
    marking("FSCODE", 40),
    marking("FSCTLH", 40),
    marking("FSREL", 40),
    marking("FSCAUT", 20),
    marking("FSCTLN", 20),
    marking("FSDWNG", 6),
    when(
        marking("FSDEVT", 40),
        Presence::When("FSDWNG", &[b"999998"]),
    ),
    int("FSCOP", 5),
    int("FSCPYS", 5),
    int("ENCRYP", 1),
    with_handling(text("ONAME", 27), Handling::Identity),
    with_handling(text("OPHONE", 18), Handling::Identity),
    length("FL", 12),
    length("HL", 6),
    int("NUMI", 3),
    Item::Repeat {
        count: Count::Field("NUMI"),
        items: &[length("LISH", 6), length("LI", 10)],
    },
    int("NUMS", 3),
    Item::Repeat {
        count: Count::Field("NUMS"),
        items: &[length("LSSH", 4), length("LS", 6)],
    },
    int("NUML", 3),
    Item::Repeat {
        count: Count::Field("NUML"),
        items: &[length("LLSH", 4), length("LL", 3)],
    },
    int("NUMT", 3),
    Item::Repeat {
        count: Count::Field("NUMT"),
        items: &[length("LTSH", 4), length("LT", 5)],
    },
    int("NUMDES", 3),
    Item::Repeat {
        count: Count::Field("NUMDES"),
        items: &[length("LDSH", 4), length("LD", 9)],
    },
    int("NUMRES", 3),
    Item::Repeat {
        count: Count::Field("NUMRES"),
        items: &[length("LRESH", 4), length("LRE", 7)],
    },
];

/// The end of both versions' file headers.
pub(crate) static FILE_HEADER_TAIL: [Item; 6] = {
    let a = extension_data!("UDHDL", "UDHOFL", "UDHD");
    let b = extension_data!("XHDL", "XHDLOFL", "XHD");
    [a[0], a[1], a[2], b[0], b[1], b[2]]
};

// ------------------------------------------------------------------ the image subheader

/// One band's fields, with its lookup tables.
const BAND: &[Item] = &[
    text("IREPBAND", 2),
    text("ISUBCAT", 6),
    text("IFC", 1),
    text("IMFLT", 3),
    int("NLUTS", 1),
    when(int("NELUT", 5), Presence::Positive("NLUTS")),
    Item::Repeat {
        count: Count::Field("NLUTS"),
        items: &[sized(bytes("LUTD", 0), Length::FromField("NELUT", 0))],
    },
];

/// The fields from ISYNC to the end of the image subheader, the same in both versions.
const IMAGE_TAIL: &[Item] = &[
    int("ISYNC", 1),
    text("IMODE", 1),
    int("NBPR", 4),
    int("NBPC", 4),
    unit(int("NPPBH", 4), "pixel"),
    unit(int("NPPBV", 4), "pixel"),
    unit(int("NBPP", 2), "bit"),
    int("IDLVL", 3),
    int("IALVL", 3),
    text("ILOC", 10),
    text("IMAG", 4),
    length("UDIDL", 5),
    when(int("UDOFL", 3), Presence::Positive("UDIDL")),
    when(
        sized(bytes("UDID", 0), Length::FromField("UDIDL", 3)),
        Presence::Positive("UDIDL"),
    ),
    length("IXSHDL", 5),
    when(int("IXSOFL", 3), Presence::Positive("IXSHDL")),
    when(
        sized(bytes("IXSHD", 0), Length::FromField("IXSHDL", 3)),
        Presence::Positive("IXSHDL"),
    ),
];

/// The NITF 2.1 and NSIF 1.0 image subheader, MIL-STD-2500C table A-3.
pub(crate) static IMAGE_SUBHEADER_21: &[Item] = &[
    text("IM", 2),
    text("IID1", 10),
    provides(
        date("IDATIM", Encoding::Ccyymmddhhmmss),
        "common.collection_time",
    ),
    text("TGTID", 17),
    text("IID2", 80),
    marking("ISCLAS", 1),
    marking("ISCLSY", 2),
    marking("ISCODE", 11),
    marking("ISCTLH", 2),
    marking("ISREL", 20),
    marking("ISDCTP", 2),
    marking("ISDCDT", 8),
    marking("ISDCXM", 4),
    marking("ISDG", 1),
    marking("ISDGDT", 8),
    marking("ISCLTX", 43),
    marking("ISCATP", 1),
    marking("ISCAUT", 40),
    marking("ISCRSN", 1),
    marking("ISSRDT", 8),
    marking("ISCTLN", 15),
    int("ENCRYP", 1),
    text("ISORCE", 42),
    unit(int("NROWS", 8), "pixel"),
    unit(int("NCOLS", 8), "pixel"),
    text("PVTYPE", 3),
    text("IREP", 8),
    text("ICAT", 8),
    unit(int("ABPP", 2), "bit"),
    text("PJUST", 1),
    provides(text("ICORDS", 1), "common.reference_system"),
    when(
        provides(text("IGEOLO", 60), "common.footprint"),
        Presence::Unless("ICORDS", &[b" "]),
    ),
    int("NICOM", 1),
    Item::Repeat {
        count: Count::Field("NICOM"),
        items: &[text("ICOM", 80)],
    },
    text("IC", 2),
    when(text("COMRAT", 4), Presence::Unless("IC", &[b"NC", b"NM"])),
    int("NBANDS", 1),
    when(int("XBANDS", 5), Presence::When("NBANDS", &[b"0"])),
    Item::Repeat {
        count: Count::FieldElse("NBANDS", "XBANDS"),
        items: BAND,
    },
    Item::Repeat {
        count: Count::Fixed(1),
        items: IMAGE_TAIL,
    },
];

/// The NITF 2.0 image subheader, MIL-STD-2500A table A-3. In NITF 2.0, ICORDS "N" means no
/// coordinates, where NITF 2.1 uses a space.
pub(crate) static IMAGE_SUBHEADER_20: &[Item] = &[
    text("IM", 2),
    text("IID", 10),
    provides(
        date("IDATIM", Encoding::Ddhhmmsszmonyy),
        "common.collection_time",
    ),
    text("TGTID", 17),
    text("ITITLE", 80),
    marking("ISCLAS", 1),
    marking("ISCODE", 40),
    marking("ISCTLH", 40),
    marking("ISREL", 40),
    marking("ISCAUT", 20),
    marking("ISCTLN", 20),
    marking("ISDWNG", 6),
    when(
        marking("ISDEVT", 40),
        Presence::When("ISDWNG", &[b"999998"]),
    ),
    int("ENCRYP", 1),
    text("ISORCE", 42),
    unit(int("NROWS", 8), "pixel"),
    unit(int("NCOLS", 8), "pixel"),
    text("PVTYPE", 3),
    text("IREP", 8),
    text("ICAT", 8),
    unit(int("ABPP", 2), "bit"),
    text("PJUST", 1),
    provides(text("ICORDS", 1), "common.reference_system"),
    when(
        provides(text("IGEOLO", 60), "common.footprint"),
        Presence::Unless("ICORDS", &[b"N"]),
    ),
    int("NICOM", 1),
    Item::Repeat {
        count: Count::Field("NICOM"),
        items: &[text("ICOM", 80)],
    },
    text("IC", 2),
    when(text("COMRAT", 4), Presence::Unless("IC", &[b"NC", b"NM"])),
    int("NBANDS", 1),
    Item::Repeat {
        count: Count::Field("NBANDS"),
        items: BAND,
    },
    Item::Repeat {
        count: Count::Fixed(1),
        items: IMAGE_TAIL,
    },
];

// ------------------------------------------------------------------ data extension subheaders

/// A data extension segment that holds extensions overflowing from a header: NITF 2.1 names it
/// TRE_OVERFLOW, NITF 2.0 Registered or Controlled Extensions. The tags are padded to 25 bytes.
pub(crate) const OVERFLOW_21: &[&[u8]] = &[b"TRE_OVERFLOW             "];
pub(crate) const OVERFLOW_20: &[&[u8]] =
    &[b"Registered Extensions    ", b"Controlled Extensions    "];

/// The NITF 2.1 and NSIF 1.0 data extension subheader, MIL-STD-2500C table A-8.
pub(crate) static DES_SUBHEADER_21: &[Item] = &[
    text("DE", 2),
    text("DESID", 25),
    int("DESVER", 2),
    marking("DESCLAS", 1),
    marking("DESCLSY", 2),
    marking("DESCODE", 11),
    marking("DESCTLH", 2),
    marking("DESREL", 20),
    marking("DESDCTP", 2),
    marking("DESDCDT", 8),
    marking("DESDCXM", 4),
    marking("DESDG", 1),
    marking("DESDGDT", 8),
    marking("DESCLTX", 43),
    marking("DESCATP", 1),
    marking("DESCAUT", 40),
    marking("DESCRSN", 1),
    marking("DESSRDT", 8),
    marking("DESCTLN", 15),
    when(text("DESOFLW", 6), Presence::When("DESID", OVERFLOW_21)),
    when(int("DESITEM", 3), Presence::When("DESID", OVERFLOW_21)),
    length("DESSHL", 4),
    sized(bytes("DESSHF", 0), Length::FromField("DESSHL", 0)),
];

/// The NITF 2.0 data extension subheader, MIL-STD-2500A table A-8.
pub(crate) static DES_SUBHEADER_20: &[Item] = &[
    text("DE", 2),
    text("DESTAG", 25),
    int("DESVER", 2),
    marking("DESCLAS", 1),
    marking("DESCODE", 40),
    marking("DESCTLH", 40),
    marking("DESREL", 40),
    marking("DESCAUT", 20),
    marking("DESCTLN", 20),
    marking("DESDWNG", 6),
    when(
        marking("DESDEVT", 40),
        Presence::When("DESDWNG", &[b"999998"]),
    ),
    when(text("DESOFLW", 6), Presence::When("DESTAG", OVERFLOW_20)),
    when(int("DESITEM", 3), Presence::When("DESTAG", OVERFLOW_20)),
    length("DESSHL", 4),
    sized(bytes("DESSHF", 0), Length::FromField("DESSHL", 0)),
];

// ------------------------------------------------------------------ extensions

/// RPC00A and RPC00B: rational polynomial coefficients (STDI-0002, appendix E). The two differ
/// only in the order of the coefficients' terms, which the geocoding step, not the fields,
/// accounts for.
pub(crate) static RPC00: &[Item] = &[
    int("SUCCESS", 1),
    unit(float("ERR_BIAS", 7), "m"),
    unit(float("ERR_RAND", 7), "m"),
    unit(int("LINE_OFF", 6), "pixel"),
    unit(int("SAMP_OFF", 5), "pixel"),
    unit(float("LAT_OFF", 8), "deg"),
    unit(float("LONG_OFF", 9), "deg"),
    unit(int("HEIGHT_OFF", 5), "m"),
    unit(int("LINE_SCALE", 6), "pixel"),
    unit(int("SAMP_SCALE", 5), "pixel"),
    unit(float("LAT_SCALE", 8), "deg"),
    unit(float("LONG_SCALE", 9), "deg"),
    unit(int("HEIGHT_SCALE", 5), "m"),
    Item::Repeat {
        count: Count::Fixed(20),
        items: &[float("LINE_NUM_COEFF", 12)],
    },
    Item::Repeat {
        count: Count::Fixed(20),
        items: &[float("LINE_DEN_COEFF", 12)],
    },
    Item::Repeat {
        count: Count::Fixed(20),
        items: &[float("SAMP_NUM_COEFF", 12)],
    },
    Item::Repeat {
        count: Count::Fixed(20),
        items: &[float("SAMP_DEN_COEFF", 12)],
    },
];

/// The longest extension tag NITF holds: its CETAG field has six characters, padded on the
/// right with spaces. A definition's code for an extension may be shorter, never longer
/// (section 14.1, "Codes up to the format's length").
pub const EXTENSION_TAG_LENGTH: usize = 6;

/// The extensions BIL defines, each by the code or family of codes its locator gives.
static EXTENSIONS: &[(&str, &[Item])] = &[("RPC00*", RPC00)];

/// Whether a definition's code for an NITF extension can match a tag: one to six printable
/// characters other than spaces, besides any wildcards, `*`. The error says why not. It is for
/// adding definitions, which section 14.1 refuses when their code breaks the limit; BIL has no
/// call to add them yet.
pub fn check_extension_code(code: &str) -> Result<(), String> {
    let fixed = code.chars().filter(|&c| c != '*').count();
    if !code.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(format!(
            "{code:?}: an extension tag is printable characters, with no spaces"
        ));
    }
    // A code of wildcards alone would claim every extension (section 14.1).
    if fixed == 0 {
        return Err(format!(
            "{code:?}: an extension tag needs a character besides `*`"
        ));
    }
    if fixed > EXTENSION_TAG_LENGTH {
        return Err(format!(
            "{code:?}: an extension tag has at most {EXTENSION_TAG_LENGTH} characters"
        ));
    }
    Ok(())
}

/// Whether `tag` is in the family `code` names: `*` stands for any run of characters,
/// including none, as section 14.1 says. One pass over each, returning to the last `*` on a
/// mismatch, so the work is bounded by the lengths' product however many wildcards a code holds.
fn matches(code: &[u8], tag: &[u8]) -> bool {
    let (mut c, mut t) = (0, 0);
    // The last `*` seen, and the place in the tag it was tried against.
    let mut star: Option<(usize, usize)> = None;
    while t < tag.len() {
        if c < code.len() && code[c] == b'*' {
            star = Some((c, t));
            c += 1;
        } else if c < code.len() && code[c] == tag[t] {
            c += 1;
            t += 1;
        } else if let Some((sc, st)) = star {
            // Let the `*` take one more character, and try again after it.
            star = Some((sc, st + 1));
            c = sc + 1;
            t = st + 1;
        } else {
            return false;
        }
    }
    code[c..].iter().all(|&b| b == b'*')
}

/// The layout of an extension, by its tag, where BIL has one. Where several definitions match,
/// a code with no wildcard is preferred, then the code with the most characters fixed, as
/// section 14.1 says.
pub(crate) fn extension_layout(tag: &str) -> Option<&'static [Item]> {
    EXTENSIONS
        .iter()
        .filter(|(code, _)| matches(code.as_bytes(), tag.as_bytes()))
        .max_by_key(|(code, _)| {
            (
                !code.contains('*'),
                code.chars().filter(|&c| c != '*').count(),
            )
        })
        .map(|&(_, layout)| layout)
}

/// The definition of the field with this native name in any of the layouts, where there is
/// one, with the number of counted runs it lies inside: how many repeat numbers its name
/// carries, as the walk gives them.
pub(crate) fn definition(layouts: &[&[Item]], native: &str) -> Option<(FieldDef, usize)> {
    fn find(items: &[Item], native: &str, depth: usize) -> Option<(FieldDef, usize)> {
        items.iter().find_map(|item| match item {
            Item::Field { def, .. } => (def.name == native).then_some((*def, depth)),
            // A fixed run of one is a group of items, which the walk does not number.
            Item::Repeat {
                count: Count::Fixed(1),
                items,
            } => find(items, native, depth),
            Item::Repeat { items, .. } => find(items, native, depth + 1),
        })
    }
    layouts.iter().find_map(|l| find(l, native, 0))
}

// ------------------------------------------------------------------ the walk

/// One field the walk found: its name with any repeat numbers, where its bytes lie, and its
/// definition.
#[derive(Clone, Debug)]
pub(crate) struct Walked {
    pub part: String,
    pub native: &'static str,
    pub at: usize,
    pub len: usize,
    pub def: FieldDef,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WalkError {
    /// The bytes end before the header does; at least this many are needed.
    Truncated(usize),
    /// A field that decides the layout does not hold a usable value.
    Corrupt(String),
}

struct Walk<'a> {
    bytes: &'a [u8],
    at: usize,
    found: Vec<Walked>,
    stop: Option<&'static str>,
    stopped: bool,
}

impl Walk<'_> {
    /// The stored bytes of the latest field with this native name: within a repeat, the one of
    /// the current pass.
    fn stored(&self, name: &str) -> Option<&[u8]> {
        self.found
            .iter()
            .rev()
            .find(|w| w.native == name)
            .map(|w| &self.bytes[w.at..w.at + w.len])
    }

    /// The whole number an earlier field holds, which the layout depends on. Blank counts as 0,
    /// as NITF writers leave optional counts blank.
    fn number(&self, name: &str) -> Result<usize, WalkError> {
        let stored = self
            .stored(name)
            .ok_or_else(|| WalkError::Corrupt(format!("{name} is needed and absent")))?;
        let text = std::str::from_utf8(stored).unwrap_or("").trim();
        if text.is_empty() {
            return Ok(0);
        }
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(WalkError::Corrupt(format!(
                "{name} is not a number: {:?}",
                String::from_utf8_lossy(stored)
            )));
        }
        text.parse()
            .map_err(|_| WalkError::Corrupt(format!("{name} is out of range")))
    }

    fn items(&mut self, items: &[Item], suffix: &str) -> Result<(), WalkError> {
        for item in items {
            if self.stopped {
                return Ok(());
            }
            match *item {
                Item::Field {
                    def,
                    length,
                    presence,
                } => {
                    let present = match presence {
                        Presence::Always => true,
                        Presence::Unless(f, values) => {
                            let v = self.stored(f).unwrap_or(&[]);
                            !values.contains(&v)
                        }
                        Presence::When(f, values) => {
                            let v = self.stored(f).unwrap_or(&[]);
                            values.contains(&v)
                        }
                        Presence::Positive(f) => self.number(f)? > 0,
                    };
                    if !present {
                        continue;
                    }
                    let len = match length {
                        Length::Fixed(n) => n,
                        Length::FromField(f, minus) => {
                            self.number(f)?.checked_sub(minus).ok_or_else(|| {
                                WalkError::Corrupt(format!("{f} is less than {minus}"))
                            })?
                        }
                    };
                    let end = self
                        .at
                        .checked_add(len)
                        .ok_or(WalkError::Truncated(usize::MAX))?;
                    if end > self.bytes.len() {
                        return Err(WalkError::Truncated(end));
                    }
                    self.found.push(Walked {
                        part: format!("{}{suffix}", def.name),
                        native: def.name,
                        at: self.at,
                        len,
                        def,
                    });
                    self.at = end;
                    if self.stop == Some(def.name) {
                        self.stopped = true;
                    }
                }
                Item::Repeat { count, items } => {
                    let n = match count {
                        Count::Fixed(n) => n,
                        Count::Field(f) => self.number(f)?,
                        Count::FieldElse(f, g) => match self.number(f)? {
                            0 => self.number(g)?,
                            n => n,
                        },
                    };
                    // A fixed run of one is a group of items, not a repeat: no number.
                    if matches!(count, Count::Fixed(1)) {
                        self.items(items, suffix)?;
                        continue;
                    }
                    for i in 1..=n {
                        // Every pass consumes bytes or none; a pass that consumes none, such as
                        // empty lookup tables, cannot run on forever, since n is at most the
                        // 99,999 of a five-digit count.
                        self.items(items, &format!("{suffix}[{i}]"))?;
                        if self.stopped {
                            return Ok(());
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Reads `bytes` through a layout, from the start, and returns every field found and the
/// number of bytes read. With `stop`, the walk ends after the first field of that name.
pub(crate) fn walk(
    bytes: &[u8],
    layout: &[&[Item]],
    stop: Option<&'static str>,
) -> Result<(Vec<Walked>, usize), WalkError> {
    let mut w = Walk {
        bytes,
        at: 0,
        found: Vec::new(),
        stop,
        stopped: false,
    };
    for items in layout {
        w.items(items, "")?;
    }
    Ok((w.found, w.at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_up_to_six_characters_and_families() {
        for code in EXTENSIONS.iter().map(|(c, _)| *c) {
            assert_eq!(check_extension_code(code), Ok(()), "{code}");
        }
        for code in ["VNDR", "VNDRTA", "RPC00*", "A*B", "*A**"] {
            assert_eq!(check_extension_code(code), Ok(()), "{code}");
        }
        for code in ["VNDRTRE", "", "*", "**", "VN DR", "ABCDEFG*"] {
            assert!(check_extension_code(code).is_err(), "{code}");
        }
        assert!(extension_layout("RPC00A").is_some());
        assert!(extension_layout("RPC00B").is_some());
        assert!(extension_layout("RPC00").is_some());
        assert!(extension_layout("RPC0").is_none());
        assert!(extension_layout("XRPC00B").is_none());
        assert!(matches(b"A*B*", b"AXXBY") && !matches(b"A*B", b"AXXBY"));
        assert!(matches(b"*B", b"AB") && matches(b"A*", b"A") && !matches(b"A*C", b"AB"));
        // Many wildcards cost no more than a few.
        let many = [b"*".repeat(200), b"X".to_vec()].concat();
        assert!(!matches(&many, &b"RPC00B".repeat(50)));
        // No two of BIL's own codes tie under section 14.1's ranking.
        let rank = |c: &str| (!c.contains('*'), c.chars().filter(|&x| x != '*').count());
        for (i, (a, _)) in EXTENSIONS.iter().enumerate() {
            for (b, _) in &EXTENSIONS[i + 1..] {
                assert!(rank(a) != rank(b), "{a} and {b} tie");
            }
        }
    }

    #[test]
    fn rpc00_is_1041_bytes() {
        let bytes = vec![b'0'; 1041];
        let (found, end) = walk(&bytes, &[RPC00], None).unwrap();
        assert_eq!(end, 1041);
        assert_eq!(found.len(), 13 + 80);
        assert_eq!(found[13].part, "LINE_NUM_COEFF[1]");
        assert_eq!(found.last().unwrap().part, "SAMP_DEN_COEFF[20]");
    }

    #[test]
    fn conditions_repeats_and_lengths() {
        // NLUTS 2, NELUT 3: two tables of three bytes, numbered.
        let band = b"M       N   200003abcdef";
        let (found, end) = walk(band, &[BAND], None).unwrap();
        assert_eq!(end, band.len());
        let parts: Vec<&str> = found.iter().map(|w| w.part.as_str()).collect();
        assert_eq!(
            parts,
            ["IREPBAND", "ISUBCAT", "IFC", "IMFLT", "NLUTS", "NELUT", "LUTD[1]", "LUTD[2]"]
        );
        assert_eq!(&band[found[7].at..found[7].at + 3], b"def");
        // No tables: no NELUT.
        let (found, _) = walk(b"M       N   0", &[BAND], None).unwrap();
        assert_eq!(found.len(), 5);
        // Cut short.
        assert_eq!(
            walk(b"M       N   200003abc", &[BAND], None).unwrap_err(),
            WalkError::Truncated(24)
        );
        // A count that is not a number.
        assert!(matches!(
            walk(b"M       N   x", &[BAND], None).unwrap_err(),
            WalkError::Corrupt(_)
        ));
    }
}
