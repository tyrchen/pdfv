//! Selection of active indirect objects from the linked cross-reference sections.

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    ops::Range,
};

use super::{
    ByteParser, CosObject, IndirectObject, ObjectStore, StreamObject, Trailer, bounded,
    checked_u64_len, optional_non_negative_offset,
};
use crate::{ObjectKey, ObjectLocation, ParseError, ParseFact, StreamFact, XrefFact};

pub(super) type XrefEntries = BTreeMap<NonZeroU32, XrefEntry>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum XrefEntry {
    Unavailable,
    Malformed,
    Uncompressed { offset: u64, generation: u16 },
    Compressed { stream: ObjectKey, index: u64 },
}

#[derive(Debug)]
pub(super) struct ClassicXref {
    pub(super) entries_range: Range<usize>,
    pub(super) trailer_index: usize,
}

#[derive(Debug, Default)]
struct XrefChain {
    entries: XrefEntries,
    trailer_indices: Vec<usize>,
}

pub(super) fn select_objects(
    mut physical: BTreeMap<u64, IndirectObject>,
    xref: Option<&XrefEntries>,
) -> Result<ObjectStore, ParseError> {
    let mut objects = ObjectStore::default();
    if let Some(entries) = xref {
        for (&number, entry) in entries {
            match *entry {
                XrefEntry::Uncompressed { offset, generation } => {
                    let key = ObjectKey::new(number, generation);
                    let object = physical
                        .remove(&offset)
                        .filter(|object| object.key == key)
                        .ok_or_else(|| ParseError::MissingObject {
                            message: bounded(&format!(
                                "cross-reference for object {number} {generation} points to byte \
                                 {offset}"
                            )),
                        })?;
                    objects.insert(object);
                }
                XrefEntry::Malformed => {
                    return Err(ParseError::Malformed {
                        message: bounded(&format!("invalid cross-reference for object {number}")),
                    });
                }
                XrefEntry::Unavailable | XrefEntry::Compressed { .. } => {}
            }
        }
    } else {
        for object in physical.into_values() {
            objects.insert(object);
        }
    }
    Ok(objects)
}

pub(super) fn verify_compressed_entries(
    objects: &ObjectStore,
    xref: Option<&XrefEntries>,
) -> Result<(), ParseError> {
    if let Some(entries) = xref {
        for (&number, entry) in entries {
            if let XrefEntry::Compressed { stream, index } = entry
                && objects.get(&ObjectKey::new(number, 0)).is_none()
            {
                let owner = stream.number;
                return Err(ParseError::MissingObject {
                    message: bounded(&format!(
                        "cross-reference for object {number} 0 points to member {index} of object \
                         stream {owner} 0"
                    )),
                });
            }
        }
    }
    Ok(())
}

impl ByteParser {
    pub(super) fn resolve_xref(
        &mut self,
        physical: &BTreeMap<u64, IndirectObject>,
        trailers: &mut Vec<Trailer>,
    ) -> Result<Option<XrefEntries>, ParseError> {
        let mut stream_trailers = BTreeMap::new();
        if let Some(chain) = self.walk_xref_chain(physical, trailers, &mut stream_trailers)? {
            self.order_trailers(physical, trailers, &stream_trailers, chain.trailer_indices);
            return Ok(Some(chain.entries));
        }

        // Preserve scan-based recovery when startxref cannot identify a section.
        for object in physical.values() {
            if let Some(stream) = xref_stream(object) {
                self.read_xref_stream(object, stream, trailers, &mut stream_trailers)?;
            }
        }
        trailers.sort_by_key(|trailer| trailer.offset);
        Ok(None)
    }

    fn order_trailers(
        &mut self,
        physical: &BTreeMap<u64, IndirectObject>,
        trailers: &mut Vec<Trailer>,
        stream_trailers: &BTreeMap<u64, usize>,
        primary_indices: Vec<usize>,
    ) {
        let mut physical_trailers = std::mem::take(trailers)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let primary = primary_indices
            .into_iter()
            .rev()
            .filter_map(|index| physical_trailers.get_mut(index)?.take())
            .collect::<Vec<_>>();
        trailers.extend(physical_trailers.into_iter().flatten());
        for object in physical.values() {
            if let Some(stream) = xref_stream(object)
                && !stream_trailers.contains_key(&object.offset)
            {
                trailers.push(Trailer {
                    dictionary: stream.dictionary.clone(),
                    offset: object.offset,
                });
            }
        }
        trailers.sort_by_key(|trailer| trailer.offset);
        self.active_trailer_start = Some(trailers.len()).filter(|start| *start != 0);
        trailers.extend(primary);
    }

    fn walk_xref_chain(
        &mut self,
        physical: &BTreeMap<u64, IndirectObject>,
        trailers: &mut Vec<Trailer>,
        stream_trailers: &mut BTreeMap<u64, usize>,
    ) -> Result<Option<XrefChain>, ParseError> {
        let Some(mut offset) = self.startxref.filter(|offset| *offset != 0) else {
            return Ok(None);
        };
        let classic = std::mem::take(&mut self.classic_xrefs);
        let mut visited = BTreeSet::new();
        let mut chain = XrefChain::default();
        loop {
            if !visited.insert(offset) {
                return Err(ParseError::Malformed {
                    message: bounded(&format!("cycle at cross-reference section byte {offset}")),
                });
            }
            let (trailer_index, entries, supplement) = if let Some(section) = classic.get(&offset)
                && let Some(trailer) = trailers.get(section.trailer_index)
            {
                let supplement = optional_non_negative_offset(&trailer.dictionary, "XRefStm")?;
                (
                    section.trailer_index,
                    self.read_classic_entries_at(section)?,
                    supplement,
                )
            } else if let Some(object) = physical.get(&offset)
                && let Some(stream) = xref_stream(object)
            {
                let (index, entries) =
                    self.read_xref_stream(object, stream, trailers, stream_trailers)?;
                (index, entries, None)
            } else if chain.trailer_indices.is_empty() {
                return Ok(None);
            } else {
                return Err(missing_section(offset));
            };
            let trailer = trailers
                .get(trailer_index)
                .ok_or_else(|| missing_section(offset))?;
            let previous = optional_non_negative_offset(&trailer.dictionary, "Prev")?;
            chain.trailer_indices.push(trailer_index);
            self.merge_xref_entries(&mut chain.entries, entries)?;

            // The classic section takes precedence over its supplement. The supplement's
            // Prev is meaningful only when that stream is itself a primary section.
            if let Some(offset) = supplement
                && !stream_trailers.contains_key(&offset)
            {
                let object = physical
                    .get(&offset)
                    .ok_or_else(|| missing_section(offset))?;
                let stream = xref_stream(object).ok_or_else(|| missing_section(offset))?;
                let (_, entries) =
                    self.read_xref_stream(object, stream, trailers, stream_trailers)?;
                self.merge_xref_entries(&mut chain.entries, entries)?;
            }

            if let Some(previous) = previous.filter(|offset| *offset != 0) {
                offset = previous;
            } else {
                return Ok(Some(chain));
            }
        }
    }

    fn merge_xref_entries(
        &self,
        active: &mut XrefEntries,
        section: XrefEntries,
    ) -> Result<(), ParseError> {
        for (number, entry) in section {
            active.entry(number).or_insert(entry);
        }
        if checked_u64_len(active.len(), "active xref entries")? > self.limits.max_objects {
            return Err(ParseError::LimitExceeded {
                limit: "max_objects",
            });
        }
        Ok(())
    }

    fn read_classic_entries_at(&self, section: &ClassicXref) -> Result<XrefEntries, ParseError> {
        let mut probe = ByteParser::new(self.source.clone(), self.limits.clone(), None);
        probe.pos = section.entries_range.start;
        probe.end = section.entries_range.end;
        let (_, entries, _) = probe.read_classic_xref_entries()?;
        Ok(entries)
    }

    fn read_xref_stream(
        &mut self,
        object: &IndirectObject,
        stream: &StreamObject,
        trailers: &mut Vec<Trailer>,
        stream_trailers: &mut BTreeMap<u64, usize>,
    ) -> Result<(usize, XrefEntries), ParseError> {
        if let Some(&index) = stream_trailers.get(&object.offset) {
            // The entries were already merged when the stream was read as a supplement.
            return Ok((index, XrefEntries::new()));
        }
        let summary = self.parse_xref_stream(object.key, stream)?;
        self.push_fact(ParseFact::Stream {
            object: object.key,
            fact: StreamFact::Decoded {
                bytes: checked_u64_len(summary.decoded_bytes, "decoded xref stream length")?,
            },
        });
        self.push_xref_chain_facts(Some(object.key), object.offset, &stream.dictionary)?;
        self.push_fact(ParseFact::Xref {
            section: ObjectLocation {
                object: Some(object.key),
                offset: Some(object.offset),
                path: None,
            },
            fact: XrefFact::XrefStreamParsed {
                entries: summary.entries,
                compressed_entries: summary.compressed_entries,
            },
        });
        let index = trailers.len();
        trailers.push(Trailer {
            dictionary: stream.dictionary.clone(),
            offset: object.offset,
        });
        stream_trailers.insert(object.offset, index);
        Ok((index, summary.objects))
    }
}

pub(super) fn xref_stream(object: &IndirectObject) -> Option<&StreamObject> {
    match &object.object {
        CosObject::Stream(stream) if matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("XRef")) => {
            Some(stream)
        }
        _ => None,
    }
}

fn missing_section(offset: u64) -> ParseError {
    ParseError::Malformed {
        message: bounded(&format!("missing cross-reference section at byte {offset}")),
    }
}
