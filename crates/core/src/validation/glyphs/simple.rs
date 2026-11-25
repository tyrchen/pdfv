//! Named encodings and Differences for simple PDF fonts.

use std::collections::BTreeMap;

use read_fonts::ps::PredefinedEncoding;

use super::{font_declares_program, font_descriptor_dictionary, sfnt, simple_font_differences};
use crate::{
    CosObject, Dictionary, ParsedDocument, PdfName,
    validation::{STANDARD_14_FONT_NAMES, resolve_dictionary_value, semantic_names},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Base {
    Embedded,
    Standard,
    WinAnsi,
    MacRoman,
    MacExpert,
    Symbol,
    ZapfDingbats,
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
            Some(CosObject::Null) => default_base(document, font, dictionary.is_some()),
            None if base_value.is_none() => default_base(document, font, dictionary.is_some()),
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

fn default_base(
    document: &ParsedDocument,
    font: &Dictionary,
    has_encoding_dictionary: bool,
) -> Base {
    let subtype = semantic_names::name(document, font.get("Subtype"));
    if has_encoding_dictionary
        && subtype.is_some_and(|name| name.matches("TrueType"))
        && font_descriptor_dictionary(document, font)
            .is_some_and(|descriptor| !sfnt::is_symbolic(document, descriptor))
    {
        return Base::Standard;
    }
    if !subtype.is_some_and(|name| name.matches("Type1")) || font_declares_program(document, font) {
        return Base::Embedded;
    }
    let Some(name) = semantic_names::name(document, font.get("BaseFont")) else {
        return Base::Embedded;
    };
    match name.as_bytes() {
        b"Symbol" => Base::Symbol,
        b"ZapfDingbats" => Base::ZapfDingbats,
        _ if STANDARD_14_FONT_NAMES
            .iter()
            .any(|standard| name.matches(standard)) =>
        {
            Base::Standard
        }
        _ => Base::Embedded,
    }
}

pub(super) fn base_name(base: Base, code: u8) -> Option<&'static str> {
    match base {
        Base::Embedded => None,
        Base::Standard => Some(PredefinedEncoding::Standard.name(code)),
        Base::WinAnsi => WIN_ANSI.get(usize::from(code)).copied(),
        Base::MacRoman => MAC_ROMAN.get(usize::from(code)).copied(),
        Base::MacExpert => MAC_EXPERT.get(usize::from(code)).copied(),
        Base::Symbol => SYMBOL.get(usize::from(code)).copied(),
        Base::ZapfDingbats => ZAPF_DINGBATS.get(usize::from(code)).copied(),
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

// Built-in glyph names from ISO 32000-1, Tables D.5 and D.6.
#[rustfmt::skip]
const SYMBOL: [&str; 256] = [
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "space", "exclam", "universal", "numbersign", "existential", "percent", "ampersand", "suchthat",
    "parenleft", "parenright", "asteriskmath", "plus", "comma", "minus", "period", "slash",
    "zero", "one", "two", "three", "four", "five", "six", "seven",
    "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question",
    "congruent", "Alpha", "Beta", "Chi", "Delta", "Epsilon", "Phi", "Gamma",
    "Eta", "Iota", "theta1", "Kappa", "Lambda", "Mu", "Nu", "Omicron",
    "Pi", "Theta", "Rho", "Sigma", "Tau", "Upsilon", "sigma1", "Omega",
    "Xi", "Psi", "Zeta", "bracketleft", "therefore", "bracketright", "perpendicular", "underscore",
    "radicalex", "alpha", "beta", "chi", "delta", "epsilon", "phi", "gamma",
    "eta", "iota", "phi1", "kappa", "lambda", "mu", "nu", "omicron",
    "pi", "theta", "rho", "sigma", "tau", "upsilon", "omega1", "omega",
    "xi", "psi", "zeta", "braceleft", "bar", "braceright", "similar", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "Euro", "Upsilon1", "minute", "lessequal", "fraction", "infinity", "florin", "club",
    "diamond", "heart", "spade", "arrowboth", "arrowleft", "arrowup", "arrowright", "arrowdown",
    "degree", "plusminus", "second", "greaterequal", "multiply", "proportional", "partialdiff", "bullet",
    "divide", "notequal", "equivalence", "approxequal", "ellipsis", "arrowvertex", "arrowhorizex", "carriagereturn",
    "aleph", "Ifraktur", "Rfraktur", "weierstrass", "circlemultiply", "circleplus", "emptyset", "intersection",
    "union", "propersuperset", "reflexsuperset", "notsubset", "propersubset", "reflexsubset", "element", "notelement",
    "angle", "gradient", "registerserif", "copyrightserif", "trademarkserif", "product", "radical", "dotmath",
    "logicalnot", "logicaland", "logicalor", "arrowdblboth", "arrowdblleft", "arrowdblup", "arrowdblright", "arrowdbldown",
    "lozenge", "angleleft", "registersans", "copyrightsans", "trademarksans", "summation", "parenlefttp", "parenleftex",
    "parenleftbt", "bracketlefttp", "bracketleftex", "bracketleftbt", "bracelefttp", "braceleftmid", "braceleftbt", "braceex",
    ".notdef", "angleright", "integral", "integraltp", "integralex", "integralbt", "parenrighttp", "parenrightex",
    "parenrightbt", "bracketrighttp", "bracketrightex", "bracketrightbt", "bracerighttp", "bracerightmid", "bracerightbt", ".notdef",
];

#[rustfmt::skip]
const ZAPF_DINGBATS: [&str; 256] = [
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    "space", "a1", "a2", "a202", "a3", "a4", "a5", "a119",
    "a118", "a117", "a11", "a12", "a13", "a14", "a15", "a16",
    "a105", "a17", "a18", "a19", "a20", "a21", "a22", "a23",
    "a24", "a25", "a26", "a27", "a28", "a6", "a7", "a8",
    "a9", "a10", "a29", "a30", "a31", "a32", "a33", "a34",
    "a35", "a36", "a37", "a38", "a39", "a40", "a41", "a42",
    "a43", "a44", "a45", "a46", "a47", "a48", "a49", "a50",
    "a51", "a52", "a53", "a54", "a55", "a56", "a57", "a58",
    "a59", "a60", "a61", "a62", "a63", "a64", "a65", "a66",
    "a67", "a68", "a69", "a70", "a71", "a72", "a73", "a74",
    "a203", "a75", "a204", "a76", "a77", "a78", "a79", "a81",
    "a82", "a83", "a84", "a97", "a98", "a99", "a100", ".notdef",
    "a89", "a90", "a93", "a94", "a91", "a92", "a205", "a85",
    "a206", "a86", "a87", "a88", "a95", "a96", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef", ".notdef",
    ".notdef", "a101", "a102", "a103", "a104", "a106", "a107", "a108",
    "a112", "a111", "a110", "a109", "a120", "a121", "a122", "a123",
    "a124", "a125", "a126", "a127", "a128", "a129", "a130", "a131",
    "a132", "a133", "a134", "a135", "a136", "a137", "a138", "a139",
    "a140", "a141", "a142", "a143", "a144", "a145", "a146", "a147",
    "a148", "a149", "a150", "a151", "a152", "a153", "a154", "a155",
    "a156", "a157", "a158", "a159", "a160", "a161", "a163", "a164",
    "a196", "a165", "a192", "a166", "a167", "a168", "a169", "a170",
    "a171", "a172", "a173", "a162", "a174", "a175", "a176", "a177",
    "a178", "a179", "a193", "a180", "a199", "a181", "a200", "a182",
    ".notdef", "a201", "a183", "a184", "a197", "a185", "a194", "a198",
    "a186", "a195", "a187", "a188", "a189", "a190", "a191", ".notdef",
];

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
