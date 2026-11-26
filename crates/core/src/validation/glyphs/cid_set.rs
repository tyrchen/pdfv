//! Complete CID inventories for PDF/A-2, PDF/A-3, and PDF/UA-1.

use read_fonts::ps::cff::{CffFontRef, charset::CharsetKind};

use super::{
    CidMap, CidToGid, Coverage, ProgramBytes, cff, font_descriptor_dictionary, font_program_stream,
    sfnt,
};
use crate::{
    Dictionary, ParsedDocument, ResourceLimits, Result, StreamObject, validation::semantic_names,
};

const CID_BITMAP_BYTES: usize = 8192;

#[derive(Debug)]
pub(super) enum Evaluation {
    Compared(Coverage),
    RejectedProgram,
}

#[derive(Debug)]
struct Inventory {
    bits: Box<[u8]>,
}

impl Inventory {
    fn new() -> Self {
        Self {
            bits: vec![0; CID_BITMAP_BYTES].into_boxed_slice(),
        }
    }

    fn add(&mut self, cid: u16) -> Option<()> {
        let slot = self.bits.get_mut(usize::from(cid / 8))?;
        let mask = 0x80 >> (cid % 8);
        if *slot & mask != 0 {
            return None;
        }
        *slot |= mask;
        Some(())
    }

    fn matches(&self, bits: &[u8]) -> bool {
        let mut declared = bits.iter().copied();
        let mut actual = self.bits.iter().copied();
        // CID 0 is optional in the declaration.
        if declared.next().unwrap_or(0) & 0x7f != actual.next().unwrap_or(0) & 0x7f {
            return false;
        }
        for byte in actual {
            if declared.next().unwrap_or(0) != byte {
                return false;
            }
        }
        declared.all(|byte| byte == 0)
    }
}

pub(super) fn complete(
    document: &ParsedDocument,
    font: &Dictionary,
    cid_set: &StreamObject,
    limits: &ResourceLimits,
) -> Result<Evaluation> {
    let truetype = semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("CIDFontType2"));
    let program = if truetype {
        font_descriptor_dictionary(document, font)
            .and_then(|descriptor| sfnt::program_stream(document, descriptor))
    } else {
        font_program_stream(document, font)
    };
    let Some(program) = program else {
        return Ok(Evaluation::RejectedProgram);
    };
    let bytes = match sfnt::read_bytes(program, limits)? {
        ProgramBytes::Decoded(bytes) => bytes,
        ProgramBytes::Malformed => return Ok(Evaluation::RejectedProgram),
        ProgramBytes::Unavailable => return Ok(Evaluation::Compared(Coverage::Unavailable)),
    };
    let inventory = if truetype {
        let Some(metrics) = sfnt::Metrics::read(&bytes) else {
            return Ok(Evaluation::RejectedProgram);
        };
        match CidMap::read(document, font, limits)? {
            CidMap::Available(map) => truetype_inventory(metrics.num_glyphs(), &map),
            CidMap::Malformed => return Ok(Evaluation::Compared(Coverage::Missing)),
            CidMap::Unavailable => return Ok(Evaluation::Compared(Coverage::Unavailable)),
        }
    } else {
        let Some(font) = cff::embedded_cid_font(document, program, &bytes) else {
            return Ok(Evaluation::RejectedProgram);
        };
        cff_inventory(&font)
    };
    let Some(inventory) = inventory else {
        return Ok(Evaluation::Compared(Coverage::Unavailable));
    };
    let coverage = match sfnt::read_bytes(cid_set, limits)? {
        ProgramBytes::Decoded(bits) if inventory.matches(&bits) => Coverage::Verified,
        ProgramBytes::Decoded(_) | ProgramBytes::Malformed => Coverage::Missing,
        ProgramBytes::Unavailable => Coverage::Unavailable,
    };
    Ok(Evaluation::Compared(coverage))
}

fn truetype_inventory(glyphs: u16, mapping: &CidToGid) -> Option<Inventory> {
    let CidToGid::Map(map) = mapping else {
        return identity_inventory(glyphs);
    };
    let mut result = Inventory::new();
    let (pairs, _) = map.as_chunks::<2>();
    for (cid, &pair) in pairs.iter().enumerate().skip(1) {
        let cid = u16::try_from(cid).ok()?;
        let gid = u16::from_be_bytes(pair);
        if gid != 0 && gid < glyphs {
            result.add(cid)?;
        }
    }
    Some(result)
}

fn cff_inventory(font: &CffFontRef<'_>) -> Option<Inventory> {
    let num_glyphs = u16::try_from(font.num_glyphs()).ok()?;
    if num_glyphs == 0 {
        return None;
    }
    if !font.is_cid() {
        return identity_inventory(num_glyphs);
    }
    let charset = font.charset()?;
    if !matches!(charset.kind(), CharsetKind::Custom(_)) {
        return None;
    }
    let mut result = Inventory::new();
    let mut glyphs = 0_u32;
    for (gid, cid) in charset.iter() {
        if gid.to_u32() != glyphs {
            return None;
        }
        result.add(cid.to_u16())?;
        glyphs = glyphs.checked_add(1)?;
    }
    (glyphs == u32::from(num_glyphs)).then_some(result)
}

fn identity_inventory(num_glyphs: u16) -> Option<Inventory> {
    let mut result = Inventory::new();
    for cid in 1..num_glyphs {
        result.add(cid)?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use read_fonts::ps::cff::CffFontRef;

    use super::{CidToGid, cff_inventory, truetype_inventory};

    #[test]
    fn test_should_require_all_cids_including_unused_subset_glyphs() {
        let inventory = truetype_inventory(6, &CidToGid::Identity);
        assert!(inventory.is_some_and(|inventory| {
            inventory.matches(&[0x7c])
                && inventory.matches(&[0xfc, 0])
                && !inventory.matches(&[0xf8])
                && !inventory.matches(&[0xfe])
                && !inventory.matches(&[0xfc, 1])
        }));
    }

    #[test]
    fn test_should_use_mapped_cids_and_skip_gid_zero() {
        let map = CidToGid::Map(vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 5, 0, 9]);
        let inventory = truetype_inventory(6, &map);
        assert!(inventory.is_some_and(|inventory| {
            inventory.matches(&[0x58])
                && !inventory.matches(&[0x78])
                && !inventory.matches(&[0x5c])
                && !inventory.matches(&[0x48])
        }));
    }

    #[test]
    fn test_should_use_glyph_indices_for_name_keyed_cff() {
        let font = [
            1, 0, 4, 1, // Header
            0, 1, 1, 1, 2, b'T', // Name INDEX
            0, 1, 1, 1, 3, 160, 17, // Top DICT, CharStrings at byte 21
            0, 0, 0, 0, // Empty String and Global Subr INDEXes
            0, 3, 1, 1, 2, 3, 4, 14, 14, 14, // Three endchar glyph programs
        ];
        let inventory = CffFontRef::new_cff(&font, 0, Some(1000))
            .ok()
            .and_then(|font| cff_inventory(&font));
        assert!(inventory.is_some_and(|inventory| {
            inventory.matches(&[0x60]) && !inventory.matches(&[0x40])
        }));
    }
}
