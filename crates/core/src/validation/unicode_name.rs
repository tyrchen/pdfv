//! UTF-8 checks on font names, colourants, and structure type names.

mod colour;

use std::{collections::HashSet, ptr};

use super::{
    ContentStreamModel, EffectiveResources, GenericModel, ModelGraph, ModelObjectRef, PageModel,
    catalog_value, descendant_font_dictionary,
    glyphs::{FontSelection, ProgramFontId},
    inherited_page_value, push_linked, semantic_names, unknown_property,
};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, PdfName,
    PropertyName, Result, ValidationError, accessibility::AccessibilityGraph,
};

pub(crate) const FAMILY: &str = "cosUnicodeName";
pub(super) const PROPERTIES: &[&str] = &["isValidUtf8", "unicodeValue"];

#[derive(Clone, Debug)]
pub(super) struct Evidence {
    valid_utf8: bool,
    text: BoundedText,
}

impl Evidence {
    fn from_name(name: &PdfName) -> Self {
        let bytes = name.as_bytes();
        let decoded = String::from_utf8_lossy(bytes);
        Self {
            valid_utf8: matches!(decoded, std::borrow::Cow::Borrowed(_)),
            text: BoundedText::unchecked(decoded),
        }
    }

    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "isValidUtf8" => Ok(ModelValue::Bool(self.valid_utf8)),
            "unicodeValue" => Ok(ModelValue::String(self.text.clone())),
            _ => unknown_property(name),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Scope {
    resources: HashSet<usize>,
    character_resources: HashSet<String>,
    fonts: HashSet<ProgramFontId>,
    colours: colour::Scope,
}

pub(super) fn enabled(graph: &ModelGraph<'_>) -> bool {
    graph.active_flavour.is_some() && graph.materializes(FAMILY)
}

fn borrow_scope<'g>(graph: &'g ModelGraph<'_>) -> Result<std::cell::RefMut<'g, Scope>> {
    graph.unicode_names.try_borrow_mut().map_err(|_| {
        ValidationError::SubsystemUnavailable {
            subsystem: "unicode_names",
        }
        .into()
    })
}

pub(super) fn append_page<'a>(
    graph: &ModelGraph<'a>,
    page: &PageModel<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !enabled(graph) {
        return Ok(());
    }
    let value = inherited_page_value(graph.document, page.key, "Resources", graph.limits)?;
    if let Some((key, CosObject::Dictionary(resources))) = resolve(graph, value)?
        && borrow_scope(graph)?
            .resources
            .insert(ptr::from_ref(resources).addr())
    {
        let owner = owner_location(
            graph,
            key.or(Some(page.key)),
            format!("root/page[{}]/Resources", page.ordinal),
        );
        append_resources(graph, resources, &owner, objects, max_objects)?;
    }
    colour::append_page_group(graph, page, objects, max_objects)
}

pub(super) fn append_content<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'_>,
    summary: &super::ContentStreamSummary,
    invocations: &mut std::collections::BTreeMap<u64, super::device_color::GraphicsState>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !enabled(graph) || content.page_key.is_none() {
        return Ok(());
    }
    if content.is_form {
        append_local_resources(graph, content, objects, max_objects)?;
    }
    colour::append_content(graph, content, summary, invocations, objects, max_objects)
}

fn append_local_resources<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'_>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let resource_path = || format!("{}/Resources", content.context_prefix);
    if let Some((key, CosObject::Dictionary(resources))) =
        resolve(graph, content.stream.dictionary.get("Resources"))?
    {
        if borrow_scope(graph)?
            .resources
            .insert(ptr::from_ref(resources).addr())
        {
            let owner = owner_location(graph, key.or(Some(content.key)), resource_path());
            append_resources(graph, resources, &owner, objects, max_objects)?;
        }
    } else if let Some(font) = &content.character_resources
        && let Some(resources) = &font.dictionary
        && borrow_scope(graph)?
            .character_resources
            .insert(font.font.clone())
    {
        let owner = owner_location(graph, Some(content.key), resource_path());
        append_resources(graph, resources, &owner, objects, max_objects)?;
    }
    Ok(())
}

fn append_resources<'a>(
    graph: &ModelGraph<'a>,
    resources: &Dictionary,
    owner: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let mut collector = Collector::new(graph, objects, max_objects, owner);
    if let Some((_, CosObject::Dictionary(fonts))) = resolve(graph, resources.get("Font"))? {
        colour::charge(graph, fonts.len())?;
        for (_, value) in fonts.iter() {
            if let Some((_, CosObject::Dictionary(font))) = resolve(graph, Some(value))? {
                append_font_slot(graph, font, &mut collector)?;
            }
        }
    }
    if let Some((_, CosObject::Dictionary(spaces))) = resolve(graph, resources.get("ColorSpace"))? {
        let resources = EffectiveResources {
            dictionaries: vec![resources],
            cyclic: false,
        };
        for (_, value) in spaces.iter() {
            colour::append_resource(graph, &resources, value, &mut collector)?;
        }
    }
    Ok(())
}

fn append_font_slot(
    graph: &ModelGraph<'_>,
    font: &Dictionary,
    collector: &mut Collector<'_, '_>,
) -> Result<()> {
    let Some(subtype) = resolved_name(graph, font.get("Subtype"))? else {
        return Ok(());
    };
    let dictionary = match subtype.as_bytes() {
        b"Type0" => descendant_font_dictionary(graph.document, font).unwrap_or(font),
        b"Type1" | b"MMType1" | b"TrueType" | b"Type3" | b"CIDFontType0" | b"CIDFontType2" => font,
        _ => return Ok(()),
    };
    if let Some(name) = resolved_name(graph, dictionary.get("BaseFont"))? {
        collector.name(name)?;
    }
    Ok(())
}

pub(super) fn append_used_font<'a>(
    graph: &ModelGraph<'a>,
    font: &FontSelection,
    dictionary: &Dictionary,
    content: &ContentStreamModel<'_>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let identity = font.identity();
    if !borrow_scope(graph)?
        .fonts
        .insert(font.dictionary_identity())
    {
        return Ok(());
    }
    let owner = owner_location(
        graph,
        font.object_key().or(Some(content.key)),
        format!("{}/font[{identity}]", content.context_prefix),
    );
    let mut collector = Collector::new(graph, objects, max_objects, &owner);
    let field = if resolved_name(graph, dictionary.get("Subtype"))?
        .is_some_and(|name| name.matches("Type3"))
    {
        "Name"
    } else {
        "BaseFont"
    };
    if let Some(name) = resolved_name(graph, dictionary.get(field))? {
        collector.name(name)?;
    }
    if let Some(descendant) = descendant_font_dictionary(graph.document, dictionary)
        && let Some(name) = resolved_name(graph, descendant.get("BaseFont"))?
    {
        collector.name(name)?;
    }
    Ok(())
}

pub(super) fn append_structure<'a>(
    graph: &ModelGraph<'a>,
    structure: &AccessibilityGraph,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !enabled(graph) {
        return Ok(());
    }
    if structure.truncated {
        return Err(ValidationError::LimitExceeded {
            limit: "structure_name_evidence",
        }
        .into());
    }
    if let Some((root_key, CosObject::Dictionary(root))) =
        resolve(graph, catalog_value(graph.document, "StructTreeRoot"))?
        && let Some((map_key, CosObject::Dictionary(map))) = resolve(graph, root.get("RoleMap"))?
    {
        let owner = owner_location(
            graph,
            map_key.or(root_key),
            "root/StructTreeRoot/RoleMap".to_owned(),
        );
        let mut collector = Collector::new(graph, objects, max_objects, &owner);
        for (key, value) in map.iter() {
            if let Some(name) = resolved_name(graph, Some(value))? {
                collector.name(key)?;
                collector.name(name)?;
            }
        }
    }
    for node in &structure.nodes {
        if let Some(valid_utf8) = node.role_is_utf8 {
            Collector::new(graph, objects, max_objects, &node.location).push(Evidence {
                valid_utf8,
                text: BoundedText::unchecked(&node.role),
            })?;
        }
    }
    Ok(())
}

fn owner_location(graph: &ModelGraph<'_>, key: Option<ObjectKey>, path: String) -> ObjectLocation {
    ObjectLocation {
        object: key,
        offset: key
            .and_then(|key| graph.document.objects.get(&key))
            .map(|object| object.offset),
        path: Some(BoundedText::unchecked(path)),
    }
}

fn resolve<'a>(
    graph: &ModelGraph<'a>,
    value: Option<&'a CosObject>,
) -> Result<Option<(Option<ObjectKey>, &'a CosObject)>> {
    semantic_names::resolve_reference(graph.document, value, graph.limits.max_object_depth)
}

fn resolved_name<'a>(
    graph: &ModelGraph<'a>,
    value: Option<&'a CosObject>,
) -> Result<Option<&'a PdfName>> {
    Ok(match resolve(graph, value)? {
        Some((_, CosObject::Name(name))) => Some(name),
        _ => None,
    })
}

struct Collector<'scope, 'a> {
    graph: &'scope ModelGraph<'a>,
    objects: &'scope mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
    owner: &'scope ObjectLocation,
    ordinal: usize,
}

impl<'scope, 'a> Collector<'scope, 'a> {
    fn new(
        graph: &'scope ModelGraph<'a>,
        objects: &'scope mut Vec<ModelObjectRef<'a>>,
        max_objects: usize,
        owner: &'scope ObjectLocation,
    ) -> Self {
        Self {
            graph,
            objects,
            max_objects,
            owner,
            ordinal: 0,
        }
    }

    fn name(&mut self, name: &PdfName) -> Result<()> {
        self.push(Evidence::from_name(name))
    }

    fn push(&mut self, evidence: Evidence) -> Result<()> {
        let context = self.owner.path.as_ref().map_or("root", BoundedText::as_str);
        let ordinal = self.ordinal;
        self.ordinal = self.ordinal.saturating_add(1);
        self.graph.charge_text_bytes(evidence.text.as_str().len())?;
        let mut model = GenericModel::new_value(
            self.graph.document,
            FAMILY,
            ModelValue::Null,
            ordinal,
            format!("{context}/unicodeName[{ordinal}]"),
        );
        model.supertypes.clear();
        model.key = self.owner.object;
        model.offset = self.owner.offset;
        model.unicode_name = Some(evidence);
        push_linked(
            self.objects,
            ModelObjectRef::Generic(model),
            self.max_objects,
        )
    }
}
