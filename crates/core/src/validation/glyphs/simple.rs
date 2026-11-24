//! Named encodings and Differences for simple PDF fonts.

use std::collections::BTreeMap;

use read_fonts::ps::PredefinedEncoding;

use super::simple_font_differences;
use crate::{
    CosObject, Dictionary, ParsedDocument, PdfName,
    validation::{resolve_dictionary_value, semantic_names},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Base {
    Embedded,
    Standard,
    WinAnsi,
    MacRoman,
    MacExpert,
}

#[derive(Debug)]
pub(super) struct Encoding {
    pub(super) base: Base,
    differences: BTreeMap<u8, PdfName>,
}

impl Encoding {
    pub(super) fn read(document: &ParsedDocument, font: &Dictionary) -> Option<Self> {
        let value = font.get("Encoding");
        let dictionary = resolve_dictionary_value(document, value);
        let base_value = dictionary.map_or(value, |dictionary| dictionary.get("BaseEncoding"));
        let base = match semantic_names::resolve(document, base_value) {
            Some(CosObject::Name(name)) => match name.as_bytes() {
                b"StandardEncoding" => Base::Standard,
                b"WinAnsiEncoding" => Base::WinAnsi,
                b"MacRomanEncoding" => Base::MacRoman,
                b"MacExpertEncoding" => Base::MacExpert,
                _ => return None,
            },
            Some(CosObject::Null) => Base::Embedded,
            None if base_value.is_none() => Base::Embedded,
            _ => return None,
        };
        let differences =
            if dictionary.is_some_and(|dictionary| dictionary.get("Differences").is_some()) {
                simple_font_differences(document, font)?
            } else {
                BTreeMap::new()
            };
        Some(Self { base, differences })
    }

    /// Returns a PDF glyph name, or delegates the code to the embedded encoding.
    pub(super) fn name(&self, code: u8) -> Option<&[u8]> {
        if let Some(name) = self.differences.get(&code) {
            return Some(name.as_bytes());
        }
        base_name(self.base, code).map(str::as_bytes)
    }
}

fn base_name(base: Base, code: u8) -> Option<&'static str> {
    match base {
        Base::Embedded => None,
        Base::Standard => Some(PredefinedEncoding::Standard.name(code)),
        Base::WinAnsi => WIN_ANSI.get(usize::from(code)).copied(),
        Base::MacRoman => MAC_ROMAN.get(usize::from(code)).copied(),
        Base::MacExpert => MAC_EXPERT.get(usize::from(code)).copied(),
    }
}

pub(super) fn mac_os_code(name: &str) -> Option<u8> {
    // ISO 32000-1, Table 115 supplements MacRoman for the Macintosh cmap.
    match name {
        "notequal" => Some(173),
        "infinity" => Some(176),
        "lessequal" => Some(178),
        "greaterequal" => Some(179),
        "partialdiff" => Some(182),
        "summation" => Some(183),
        "product" => Some(184),
        "pi" => Some(185),
        "integral" => Some(186),
        "Omega" => Some(189),
        "radical" => Some(195),
        "approxequal" => Some(197),
        "Delta" => Some(198),
        "lozenge" => Some(215),
        "Euro" => Some(219),
        "apple" => Some(240),
        ".notdef" | "currency" => None,
        _ => MAC_ROMAN
            .iter()
            .position(|value| *value == name)
            .and_then(|code| u8::try_from(code).ok()),
    }
}

// Glyph-name mappings from ISO 32000-1 Annex D.
#[rustfmt::skip]
const MAC_ROMAN: [&str; 256] = [
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "quotesingle",
    "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period", "slash",
    "zero", "one", "two", "three", "four", "five", "six", "seven",
    "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question",
    "at", "A", "B", "C", "D", "E", "F", "G",
    "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W",
    "X", "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore",
    "grave", "a", "b", "c", "d", "e", "f", "g",
    "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w",
    "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde", ".notdef",
    "Adieresis", "Aring", "Ccedilla", "Eacute", "Ntilde", "Odieresis", "Udieresis", "aacute",
    "agrave", "acircumflex", "adieresis", "atilde", "aring", "ccedilla", "eacute", "egrave",
    "ecircumflex", "edieresis", "iacute", "igrave", "icircumflex", "idieresis", "ntilde", "oacute",
    "ograve", "ocircumflex", "odieresis", "otilde", "uacute", "ugrave", "ucircumflex", "udieresis",
    "dagger", "degree", "cent", "sterling", "section", "bullet", "paragraph", "germandbls",
    "registered", "copyright", "trademark", "acute", "dieresis", ".notdef", "AE", "Oslash",
    ".notdef", "plusminus", ".notdef", ".notdef", "yen", "mu", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", "ordfeminine", "ordmasculine", ".notdef", "ae", "oslash",
    "questiondown", "exclamdown", "logicalnot", ".notdef", "florin", ".notdef", ".notdef", "guillemotleft",
    "guillemotright", "ellipsis", "space", "Agrave", "Atilde", "Otilde", "OE", "oe",
    "endash", "emdash", "quotedblleft", "quotedblright", "quoteleft", "quoteright", "divide", ".notdef",
    "ydieresis", "Ydieresis", "fraction", "currency", "guilsinglleft", "guilsinglright", "fi", "fl",
    "daggerdbl", "periodcentered", "quotesinglbase", "quotedblbase", "perthousand", "Acircumflex", "Ecircumflex", "Aacute",
    "Edieresis", "Egrave", "Iacute", "Icircumflex", "Idieresis", "Igrave", "Oacute", "Ocircumflex",
    ".notdef", "Ograve", "Uacute", "Ucircumflex", "Ugrave", "dotlessi", "circumflex", "tilde",
    "macron", "breve", "dotaccent", "ring", "cedilla", "hungarumlaut", "ogonek", "caron",
];

#[rustfmt::skip]
const MAC_EXPERT: [&str; 256] = [
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "space", "exclamsmall", "Hungarumlautsmall", "centoldstyle", "dollaroldstyle", "dollarsuperior", "ampersandsmall", "Acutesmall",
    "parenleftsuperior", "parenrightsuperior", "twodotenleader", "onedotenleader", "comma", "hyphen", "period", "fraction",
    "zerooldstyle", "oneoldstyle", "twooldstyle", "threeoldstyle", "fouroldstyle", "fiveoldstyle", "sixoldstyle", "sevenoldstyle",
    "eightoldstyle", "nineoldstyle", "colon", "semicolon", ".notdef", "threequartersemdash", ".notdef", "questionsmall",
    ".notdef", ".notdef", ".notdef", ".notdef", "Ethsmall", ".notdef", ".notdef", "onequarter",
    "onehalf", "threequarters", "oneeighth", "threeeighths", "fiveeighths", "seveneighths", "onethird", "twothirds",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "ff", "fi",
    "fl", "ffi", "ffl", "parenleftinferior", ".notdef", "parenrightinferior", "Circumflexsmall", "hypheninferior",
    "Gravesmall", "Asmall", "Bsmall", "Csmall", "Dsmall", "Esmall", "Fsmall", "Gsmall",
    "Hsmall", "Ismall", "Jsmall", "Ksmall", "Lsmall", "Msmall", "Nsmall", "Osmall",
    "Psmall", "Qsmall", "Rsmall", "Ssmall", "Tsmall", "Usmall", "Vsmall", "Wsmall",
    "Xsmall", "Ysmall", "Zsmall", "colonmonetary", "onefitted", "rupiah", "Tildesmall", ".notdef",
    ".notdef", "asuperior", "centsuperior", ".notdef", ".notdef", ".notdef", ".notdef", "Aacutesmall",
    "Agravesmall", "Acircumflexsmall", "Adieresissmall", "Atildesmall", "Aringsmall", "Ccedillasmall", "Eacutesmall", "Egravesmall",
    "Ecircumflexsmall", "Edieresissmall", "Iacutesmall", "Igravesmall", "Icircumflexsmall", "Idieresissmall", "Ntildesmall", "Oacutesmall",
    "Ogravesmall", "Ocircumflexsmall", "Odieresissmall", "Otildesmall", "Uacutesmall", "Ugravesmall", "Ucircumflexsmall", "Udieresissmall",
    ".notdef", "eightsuperior", "fourinferior", "threeinferior", "sixinferior", "eightinferior", "seveninferior", "Scaronsmall",
    ".notdef", "centinferior", "twoinferior", ".notdef", "Dieresissmall", ".notdef", "Caronsmall", "osuperior",
    "fiveinferior", ".notdef", "commainferior", "periodinferior", "Yacutesmall", ".notdef", "dollarinferior", ".notdef",
    ".notdef", "Thornsmall", ".notdef", "nineinferior", "zeroinferior", "Zcaronsmall", "AEsmall", "Oslashsmall",
    "questiondownsmall", "oneinferior", "Lslashsmall", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", "Cedillasmall", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", "OEsmall",
    "figuredash", "hyphensuperior", ".notdef", ".notdef", ".notdef", ".notdef", "exclamdownsmall", ".notdef",
    "Ydieresissmall", ".notdef", "onesuperior", "twosuperior", "threesuperior", "foursuperior", "fivesuperior", "sixsuperior",
    "sevensuperior", "ninesuperior", "zerosuperior", ".notdef", "esuperior", "rsuperior", "tsuperior", ".notdef",
    ".notdef", "isuperior", "ssuperior", "dsuperior", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", "lsuperior", "Ogoneksmall", "Brevesmall", "Macronsmall", "bsuperior", "nsuperior", "msuperior",
    "commasuperior", "periodsuperior", "Dotaccentsmall", "Ringsmall", ".notdef", ".notdef", ".notdef", ".notdef",
];

#[rustfmt::skip]
const WIN_ANSI: [&str; 256] = [
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "quotesingle",
    "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period", "slash",
    "zero", "one", "two", "three", "four", "five", "six", "seven",
    "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question",
    "at", "A", "B", "C", "D", "E", "F", "G",
    "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W",
    "X", "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore",
    "grave", "a", "b", "c", "d", "e", "f", "g",
    "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w",
    "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde", "bullet",
    "Euro", "bullet", "quotesinglbase", "florin", "quotedblbase", "ellipsis", "dagger", "daggerdbl",
    "circumflex", "perthousand", "Scaron", "guilsinglleft", "OE", "bullet", "Zcaron", "bullet",
    "bullet", "quoteleft", "quoteright", "quotedblleft", "quotedblright", "bullet", "endash", "emdash",
    "tilde", "trademark", "scaron", "guilsinglright", "oe", "bullet", "zcaron", "Ydieresis",
    "space", "exclamdown", "cent", "sterling", "currency", "yen", "brokenbar", "section",
    "dieresis", "copyright", "ordfeminine", "guillemotleft", "logicalnot", "hyphen", "registered", "macron",
    "degree", "plusminus", "twosuperior", "threesuperior", "acute", "mu", "paragraph", "periodcentered",
    "cedilla", "onesuperior", "ordmasculine", "guillemotright", "onequarter", "onehalf", "threequarters", "questiondown",
    "Agrave", "Aacute", "Acircumflex", "Atilde", "Adieresis", "Aring", "AE", "Ccedilla",
    "Egrave", "Eacute", "Ecircumflex", "Edieresis", "Igrave", "Iacute", "Icircumflex", "Idieresis",
    "Eth", "Ntilde", "Ograve", "Oacute", "Ocircumflex", "Otilde", "Odieresis", "multiply",
    "Oslash", "Ugrave", "Uacute", "Ucircumflex", "Udieresis", "Yacute", "Thorn", "germandbls",
    "agrave", "aacute", "acircumflex", "atilde", "adieresis", "aring", "ae", "ccedilla",
    "egrave", "eacute", "ecircumflex", "edieresis", "igrave", "iacute", "icircumflex", "idieresis",
    "eth", "ntilde", "ograve", "oacute", "ocircumflex", "otilde", "odieresis", "divide",
    "oslash", "ugrave", "uacute", "ucircumflex", "udieresis", "yacute", "thorn", "ydieresis",
];
