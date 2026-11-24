//! Device colour spaces at their points of use in content streams.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ptr,
};

use super::{
    AnnotationModel, ContentStreamModel, EffectiveResources, GenericModel, ModelGraph,
    ModelObjectRef, OperatorFact, ResourceFamily, array_values, color_space_component_count,
    color_space_family, effective_resources_for_page_key, output_intent_icc_header,
    page_dictionary, push_linked, resolve_dictionary_value, resource_value, scoped_resource_value,
    semantic_names,
};
use crate::{
    CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ParsedDocument, PdfName, Result,
};

pub(super) const FAMILIES: &[&str] = &["deviceGray", "deviceRgb", "deviceCmyk"];
pub(super) const PROPERTIES: &[&str] = &[
    "gOutputCS",
    "gDocumentOutputCS",
    "gPageOutputCS",
    "gTransparencyCS",
];

#[derive(Clone, Debug)]
pub(super) struct OutputSpaces {
    pub(super) document: ModelValue,
    pub(super) page: ModelValue,
    pub(super) transparency: ModelValue,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Device {
    Gray,
    Rgb,
    Cmyk,
}

impl Device {
    const fn family(self) -> &'static str {
        match self {
            Self::Gray => "deviceGray",
            Self::Rgb => "deviceRgb",
            Self::Cmyk => "deviceCmyk",
        }
    }

    const fn default_name(self) -> &'static str {
        match self {
            Self::Gray => "DefaultGray",
            Self::Rgb => "DefaultRGB",
            Self::Cmyk => "DefaultCMYK",
        }
    }

    const fn components(self) -> u64 {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }

    fn from_name(name: &PdfName, inline: bool) -> Option<Self> {
        match name.as_bytes() {
            b"DeviceGray" => Some(Self::Gray),
            b"DeviceRGB" => Some(Self::Rgb),
            b"DeviceCMYK" => Some(Self::Cmyk),
            b"G" if inline => Some(Self::Gray),
            b"RGB" if inline => Some(Self::Rgb),
            b"CMYK" if inline => Some(Self::Cmyk),
            _ => None,
        }
    }
}

pub(super) fn append<'a>(
    model: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    summary: &super::ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<BTreeMap<u64, GraphicsState>> {
    let resources = model.effective_resources(graph.document)?;
    let defaults = [Device::Gray, Device::Rgb, Device::Cmyk]
        .into_iter()
        .filter(|device| has_default(graph, &resources, *device))
        .collect::<BTreeSet<_>>();
    let mut state = model
        .initial_color_state
        .unwrap_or_else(|| GraphicsState::initial(model.is_form, &defaults));
    state.transparency = transparency_space(graph, model, &resources)?.or(state.transparency);
    let uses = used_devices(graph, &resources, summary, state, &defaults)?;
    if model.object_type.as_str() == super::language::CONTENT_FAMILY {
        return Ok(uses.invocations);
    }
    let mut output = None;
    for (device, location) in uses.devices {
        if !graph.materializes(device.family()) || defaults.contains(&device) {
            continue;
        }
        if output.is_none() {
            output = Some(output_spaces(graph, model.page_key, state.transparency)?);
        }
        let mut model = GenericModel::new_value(
            graph.document,
            device.family(),
            ModelValue::Null,
            0,
            format!(
                "{}/deviceColor[{:?}]",
                location_path(location, &model.context_prefix),
                state.transparency
            ),
        );
        model.supertypes.clear();
        model.device_output.clone_from(&output);
        push_linked(objects, ModelObjectRef::Generic(model), max_objects)?;
    }
    Ok(uses.invocations)
}

pub(super) fn append_3d_annotation<'a>(
    annotation: &AnnotationModel<'_>,
    graph: &ModelGraph<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !FAMILIES.iter().any(|family| graph.materializes(family))
        || !annotation
            .resolved_name(annotation.dictionary.get("Subtype"))?
            .is_some_and(|name| name.matches("3D"))
    {
        return Ok(());
    }
    let resolve = |value| {
        semantic_names::resolve_reference(graph.document, value, graph.limits.max_object_depth)
    };
    let mut target = resolve(annotation.dictionary.get("3DD"))?;
    if let Some((_, CosObject::Dictionary(reference))) = target
        && annotation
            .resolved_name(reference.get("Type"))?
            .is_some_and(|name| name.matches("3DRef"))
    {
        target = resolve(reference.get("3DD"))?;
    }
    let Some((key, CosObject::Stream(stream))) = target else {
        return Ok(());
    };
    if !annotation
        .resolved_name(stream.dictionary.get("Type"))?
        .is_some_and(|name| name.matches("3D"))
    {
        return Ok(());
    }
    let Some((_, value)) = resolve(stream.dictionary.get("ColorSpace"))? else {
        return Ok(());
    };
    let mut resources =
        effective_resources_for_page_key(graph.document, annotation.page_key, graph.limits)?;
    if let Some(nearest) = resources.dictionaries.pop() {
        resources.dictionaries.clear();
        resources.dictionaries.push(nearest);
    }
    let device = selected_3d_space(graph, &resources, value)?;
    let Some(device) = device.filter(|device| graph.materializes(device.family())) else {
        return Ok(());
    };
    let mut model = GenericModel::new_value(
        graph.document,
        device.family(),
        ModelValue::Null,
        0,
        format!(
            "root/page[{}]/annotation[{}]/stream3D/colorSpace",
            annotation.page_ordinal, annotation.ordinal
        ),
    );
    model.key = key;
    model.offset = key.and_then(|key| graph.document.objects.get(&key).map(|object| object.offset));
    model.supertypes.clear();
    let page = page_dictionary(graph.document, annotation.page_key);
    let transparency = owner_transparency_space(graph, page, false, &resources)?;
    model.device_output = Some(output_spaces(
        graph,
        Some(annotation.page_key),
        transparency,
    )?);
    push_linked(objects, ModelObjectRef::Generic(model), max_objects)
}

fn selected_3d_space<'a>(
    graph: &ModelGraph<'a>,
    resources: &EffectiveResources<'a>,
    mut value: &'a CosObject,
) -> Result<Option<Device>> {
    let mut seen = HashSet::new();
    let mut scope = ColourLookup::Entry;
    let resolve = |value| {
        semantic_names::resolve_reference(graph.document, value, graph.limits.max_object_depth)
            .map(|resolved| resolved.map(|(_, value)| value))
    };
    let lookup =
        |name: &PdfName| scoped_resource_value(graph, resources, ResourceFamily::ColorSpace, name);
    for _ in 0..=graph.limits.max_object_depth {
        let Some(resolved) = resolve(Some(value))? else {
            return Ok(None);
        };
        if !seen.insert((ptr::from_ref(resolved), scope)) {
            return Ok(None);
        }
        match resolved {
            CosObject::Name(name) => {
                let device = Device::from_name(name, false);
                if matches!(scope, ColourLookup::Entry | ColourLookup::Resource)
                    && let Some(device) = device
                {
                    let default_name = PdfName::try_from(device.default_name().to_owned())?;
                    if let Some(default) = lookup(&default_name)? {
                        scope = ColourLookup::Default;
                        value = default;
                        continue;
                    }
                }
                if scope != ColourLookup::Direct
                    && let Some(resource) = lookup(name)?
                {
                    if scope == ColourLookup::Entry {
                        scope = ColourLookup::Resource;
                    }
                    value = resource;
                } else {
                    return Ok(device);
                }
            }
            CosObject::Array(elements) => {
                if let [entry] = elements.as_slice() {
                    value = entry;
                } else {
                    let Some(CosObject::Name(family)) = resolve(elements.first())? else {
                        return Ok(None);
                    };
                    let alternate = match family.as_bytes() {
                        b"Indexed" => elements.get(1),
                        b"Separation" | b"DeviceN" => elements.get(2),
                        _ => None,
                    };
                    let Some(alternate) = alternate else {
                        return Ok(None);
                    };
                    value = alternate;
                    if scope == ColourLookup::Entry {
                        scope = ColourLookup::Direct;
                    }
                }
            }
            _ => return Ok(None),
        }
    }
    Err(crate::ValidationError::LimitExceeded {
        limit: "model_reference_depth",
    }
    .into())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ColourLookup {
    Entry,
    Resource,
    Default,
    Direct,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct GraphicsState {
    fill: Option<Device>,
    stroke: Option<Device>,
    fill_pattern: bool,
    stroke_pattern: bool,
    pub(super) text_mode: Option<u8>,
    transparency: Option<BlendingSpace>,
    pub(super) device_n_state: Option<super::device_n::ColourUseState>,
    pub(super) unicode_colour_names: [Option<usize>; 2],
}

impl GraphicsState {
    fn initial(is_form: bool, defaults: &BTreeSet<Device>) -> Self {
        let device = (!is_form && !defaults.contains(&Device::Gray)).then_some(Device::Gray);
        Self {
            fill: device,
            stroke: device,
            fill_pattern: false,
            stroke_pattern: false,
            text_mode: (!is_form).then_some(0),
            transparency: None,
            device_n_state: None,
            unicode_colour_names: [None; 2],
        }
    }

    fn painted(self, operator: &str) -> [Option<Device>; 2] {
        let (fill, stroke) = match operator {
            "f" | "F" | "f*" => (true, false),
            "S" | "s" => (false, true),
            "B" | "B*" | "b" | "b*" => (true, true),
            "text" => match self.text_mode {
                Some(0 | 4) => (true, false),
                Some(1 | 5) => (false, true),
                Some(2 | 6) => (true, true),
                _ => (false, false),
            },
            _ => (false, false),
        };
        [self.fill.filter(|_| fill), self.stroke.filter(|_| stroke)]
    }

    fn follows_pattern_invocation(self, operator: &str, require_pattern: bool) -> bool {
        match operator {
            "scn" => self.fill_pattern || !require_pattern,
            "SCN" => self.stroke_pattern || !require_pattern,
            _ => false,
        }
    }
}

#[derive(Debug, Default)]
struct ContentColourUses<'a> {
    devices: BTreeMap<Device, &'a ObjectLocation>,
    invocations: BTreeMap<u64, GraphicsState>,
}

fn used_devices<'a>(
    graph: &ModelGraph<'_>,
    resources: &EffectiveResources<'_>,
    summary: &'a super::ContentStreamSummary,
    mut state: GraphicsState,
    defaults: &BTreeSet<Device>,
) -> Result<ContentColourUses<'a>> {
    let mut uses = ContentColourUses::default();
    let mut selectors = BTreeMap::new();
    for usage in &summary.resource_uses {
        if usage.family == ResourceFamily::ColorSpace {
            selectors.insert(usage.location.offset, &usage.name);
        } else if matches!(
            usage.family,
            ResourceFamily::XObject | ResourceFamily::Shading
        ) && let Some(device) =
            device_from_resource(graph.document, resources, usage.family, &usage.name)
        {
            uses.devices.entry(device).or_insert(&usage.location);
        }
    }
    let mut saved = Vec::new();
    for fact in &summary.facts {
        match fact {
            OperatorFact::GraphicsState { op, .. } if op.as_str() == "q" => {
                if u32::try_from(saved.len()).unwrap_or(u32::MAX)
                    >= graph.limits.max_graphics_state_depth
                {
                    return Err(crate::ValidationError::LimitExceeded {
                        limit: "max_graphics_state_depth",
                    }
                    .into());
                }
                saved.push(state);
            }
            OperatorFact::GraphicsState { op, .. } if op.as_str() == "Q" => {
                if let Some(previous) = saved.pop() {
                    state = previous;
                }
            }
            OperatorFact::Color { op, location, .. } => {
                let operator = op.as_str();
                let selected = colour_selector(
                    graph.document,
                    resources,
                    operator,
                    selectors.get(&location.offset).copied(),
                );
                if let Some(device) = selected {
                    uses.devices.entry(device).or_insert(location);
                }
                let active = selected.filter(|device| !defaults.contains(device));
                let name = selectors.get(&location.offset).copied();
                update_selection(graph, resources, operator, name, active, &mut state)?;
                if state.follows_pattern_invocation(operator, graph.needs_pattern_invocations())
                    && let Some(offset) = location.offset
                {
                    uses.invocations.insert(offset, state);
                }
            }
            OperatorFact::InlineImage {
                color_space,
                image_mask,
                location,
                ..
            } => {
                let device = if *image_mask {
                    state.fill
                } else {
                    color_space
                        .as_ref()
                        .and_then(|name| inline_color(graph.document, resources, name))
                };
                if let Some(device) = device {
                    uses.devices.entry(device).or_insert(location);
                }
            }
            OperatorFact::XObjectInvoke { name, location } => {
                if let Some(offset) = location.offset {
                    uses.invocations.insert(offset, state);
                }
                if let Some(device) = state.fill
                    && is_image_mask(graph.document, resources, name)
                {
                    uses.devices.entry(device).or_insert(location);
                }
            }
            OperatorFact::TextState {
                op, rendering_mode, ..
            } if op.as_str() == "Tr" => {
                state.text_mode = rendering_mode
                    .and_then(|mode| u8::try_from(mode).ok())
                    .filter(|mode| *mode <= 7);
            }
            OperatorFact::TextShow { location, .. } => {
                record_paint(&mut uses.devices, state.painted("text"), location);
            }
            OperatorFact::Path { op, location } => {
                record_paint(&mut uses.devices, state.painted(op.as_str()), location);
            }
            _ => {}
        }
    }
    Ok(uses)
}

fn update_selection(
    graph: &ModelGraph<'_>,
    resources: &EffectiveResources<'_>,
    operator: &str,
    name: Option<&PdfName>,
    active: Option<Device>,
    state: &mut GraphicsState,
) -> Result<()> {
    let patterned = graph.needs_pattern_invocations()
        && matches!(operator, "cs" | "CS")
        && if let Some(name) = name {
            is_pattern_space(graph, resources, name)?
        } else {
            false
        };
    match operator {
        "g" | "rg" | "k" | "cs" => {
            state.fill = active;
            state.fill_pattern = patterned;
        }
        "G" | "RG" | "K" | "CS" => {
            state.stroke = active;
            state.stroke_pattern = patterned;
        }
        _ => {}
    }
    Ok(())
}

fn is_pattern_space(
    graph: &ModelGraph<'_>,
    resources: &EffectiveResources<'_>,
    name: &PdfName,
) -> Result<bool> {
    if name.matches("Pattern") {
        return Ok(true);
    }
    let value = scoped_resource_value(graph, resources, ResourceFamily::ColorSpace, name)?;
    let first = match semantic_names::resolve_reference(
        graph.document,
        value,
        graph.limits.max_object_depth,
    )? {
        Some((_, CosObject::Array(parts))) => parts.first(),
        value => value.map(|(_, value)| value),
    };
    Ok(
        matches!(semantic_names::resolve_reference(graph.document, first, graph.limits.max_object_depth)?,
        Some((_, CosObject::Name(name))) if name.matches("Pattern")),
    )
}

fn inline_color(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    name: &PdfName,
) -> Option<Device> {
    Device::from_name(name, true)
        .or_else(|| device_from_resource(document, resources, ResourceFamily::ColorSpace, name))
}

fn colour_selector(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    operator: &str,
    name: Option<&PdfName>,
) -> Option<Device> {
    match operator {
        "g" | "G" => Some(Device::Gray),
        "rg" | "RG" => Some(Device::Rgb),
        "k" | "K" => Some(Device::Cmyk),
        "cs" | "CS" => device_from_resource(document, resources, ResourceFamily::ColorSpace, name?),
        _ => None,
    }
}

fn is_image_mask(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    name: &PdfName,
) -> bool {
    resources
        .dictionaries
        .iter()
        .rev()
        .find_map(|dictionary| resource_value(document, dictionary, ResourceFamily::XObject, name))
        .and_then(|value| resolve_dictionary_value(document, Some(value)))
        .is_some_and(|dictionary| {
            semantic_names::name(document, dictionary.get("Subtype"))
                .is_some_and(|name| name.matches("Image"))
                && matches!(
                    semantic_names::resolve(document, dictionary.get("ImageMask")),
                    Some(CosObject::Boolean(true))
                )
        })
}

fn record_paint<'a>(
    uses: &mut BTreeMap<Device, &'a ObjectLocation>,
    devices: [Option<Device>; 2],
    location: &'a ObjectLocation,
) {
    for device in devices.into_iter().flatten() {
        uses.entry(device).or_insert(location);
    }
}

fn location_path<'a>(location: &'a ObjectLocation, fallback: &'a str) -> &'a str {
    location
        .path
        .as_ref()
        .map_or(fallback, crate::BoundedText::as_str)
}

fn device_from_resource(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    family: ResourceFamily,
    name: &PdfName,
) -> Option<Device> {
    if family == ResourceFamily::ColorSpace
        && let Some(device) = Device::from_name(name, false)
    {
        return Some(device);
    }
    let value = resources
        .dictionaries
        .iter()
        .rev()
        .find_map(|dictionary| resource_value(document, dictionary, family, name))?;
    let value = if family == ResourceFamily::ColorSpace {
        value
    } else {
        let dictionary = resolve_dictionary_value(document, Some(value))?;
        let space = dictionary.get("ColorSpace")?;
        match space {
            CosObject::Name(name) if Device::from_name(name, false).is_none() => resources
                .dictionaries
                .iter()
                .rev()
                .find_map(|dictionary| {
                    resource_value(document, dictionary, ResourceFamily::ColorSpace, name)
                })
                .unwrap_or(space),
            _ => space,
        }
    };
    device_from_value(document, value)
}

fn device_from_value<'a>(document: &'a ParsedDocument, mut value: &'a CosObject) -> Option<Device> {
    for _ in 0..=8 {
        match semantic_names::resolve(document, Some(value))? {
            CosObject::Name(name) => return Device::from_name(name, false),
            CosObject::Array(parts) => {
                let CosObject::Name(family) = parts.first()? else {
                    return None;
                };
                value = match family.as_bytes() {
                    b"Indexed" | b"Pattern" => parts.get(1)?,
                    b"Separation" | b"DeviceN" => parts.get(2)?,
                    _ => return None,
                };
            }
            _ => return None,
        }
    }
    None
}

fn default_space(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    device: Device,
) -> Option<Dictionary> {
    let value = resources
        .dictionaries
        .last()
        .and_then(|resources| resolve_dictionary_value(document, resources.get("ColorSpace")))?
        .get(device.default_name())?;
    super::normalize_color_space_resource(document, value).map(|(_, _, space)| space)
}

fn has_default(graph: &ModelGraph<'_>, resources: &EffectiveResources<'_>, device: Device) -> bool {
    let Some(space) = default_space(graph.document, resources, device) else {
        return false;
    };
    let family = color_space_family(&space);
    let device_n_permitted = device == Device::Cmyk
        && family == "DeviceN"
        && graph.active_flavour.as_ref().is_some_and(|flavour| {
            flavour.family.as_str() == "pdfa" && matches!(flavour.part.get(), 2 | 3)
        });
    if device_n_permitted {
        return matches!(
            semantic_names::resolve(graph.document, space.get("Components")),
            Some(CosObject::Array(names)) if names.len() == 4 && names.iter().all(|name| matches!(name, CosObject::Name(_)))
        );
    }
    matches!(family.as_str(), "CalGray" | "CalRGB" | "Lab" | "ICCBased")
        && color_space_component_count(&space) == Some(device.components())
}

#[derive(Clone, Copy, Debug)]
enum BlendingSpace {
    CalRgb,
    CalGray,
    Lab,
    Rgb,
    Gray,
    Cmyk,
}

impl BlendingSpace {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "CalRGB" => Some(Self::CalRgb),
            "CalGray" => Some(Self::CalGray),
            "Lab" => Some(Self::Lab),
            "RGB " => Some(Self::Rgb),
            "GRAY" => Some(Self::Gray),
            "CMYK" => Some(Self::Cmyk),
            _ => None,
        }
    }

    const fn property(self) -> &'static str {
        match self {
            Self::CalRgb => "CalRGB",
            Self::CalGray => "CalGray",
            Self::Lab => "Lab",
            Self::Rgb => "RGB ",
            Self::Gray => "GRAY",
            Self::Cmyk => "CMYK",
        }
    }
}

fn transparency_space(
    graph: &ModelGraph<'_>,
    stream: &ContentStreamModel<'_>,
    resources: &EffectiveResources<'_>,
) -> Result<Option<BlendingSpace>> {
    let document = graph.document;
    let owner = if stream.is_form {
        Some(&stream.stream.dictionary)
    } else {
        stream
            .page_key
            .and_then(|key| page_dictionary(document, key))
    };
    owner_transparency_space(graph, owner, stream.is_form, resources)
}

fn owner_transparency_space(
    graph: &ModelGraph<'_>,
    owner: Option<&Dictionary>,
    is_form: bool,
    resources: &EffectiveResources<'_>,
) -> Result<Option<BlendingSpace>> {
    let document = graph.document;
    let group = owner.and_then(|owner| resolve_dictionary_value(document, owner.get("Group")));
    let Some(group) = group.filter(|group| {
        semantic_names::name(document, group.get("S"))
            .is_some_and(|name| name.matches("Transparency"))
            && (!is_form
                || matches!(
                    semantic_names::resolve(document, group.get("I")),
                    Some(CosObject::Boolean(true))
                ))
    }) else {
        return Ok(None);
    };
    let Some(value) = group.get("CS") else {
        return Ok(None);
    };
    let value = match value {
        CosObject::Name(name) => resources
            .dictionaries
            .iter()
            .rev()
            .find_map(|dictionary| {
                resource_value(document, dictionary, ResourceFamily::ColorSpace, name)
            })
            .unwrap_or(value),
        _ => value,
    };
    let normalized = if let Some(device) = device_from_value(document, value)
        && matches!(
            semantic_names::resolve(document, Some(value)),
            Some(CosObject::Name(_))
        ) {
        default_space(document, resources, device)
    } else {
        super::normalize_color_space_resource(document, value).map(|(_, _, space)| space)
    };
    let Some(space) = normalized else {
        return Ok(None);
    };
    let family = color_space_family(&space);
    if family != "ICCBased" {
        return Ok(BlendingSpace::parse(&family));
    }
    let Some(CosObject::Stream(stream)) =
        semantic_names::resolve(document, space.get("ICCProfile"))
    else {
        return Ok(None);
    };
    let bytes = stream.decoded_bytes(graph.limits)?;
    Ok(
        super::parse_icc_header(&bytes)
            .and_then(|header| BlendingSpace::parse(&header.color_space)),
    )
}

fn output_spaces(
    graph: &ModelGraph<'_>,
    page: Option<ObjectKey>,
    transparency: Option<BlendingSpace>,
) -> Result<OutputSpaces> {
    let page = page.and_then(|key| page_dictionary(graph.document, key));
    let doc_space = output_color_space(graph, super::catalog_dictionary(graph.document))?;
    let page_space = output_color_space(graph, page)?;
    Ok(OutputSpaces {
        document: super::optional_string_model_value(doc_space),
        page: super::optional_string_model_value(page_space),
        transparency: transparency.map_or(ModelValue::Null, |space| {
            ModelValue::String(crate::BoundedText::unchecked(space.property()))
        }),
    })
}

fn output_color_space(
    graph: &ModelGraph<'_>,
    owner: Option<&Dictionary>,
) -> Result<Option<String>> {
    let intents = owner.and_then(|owner| owner.get("OutputIntents"));
    for intent in array_values(semantic_names::resolve(graph.document, intents)) {
        if let Some(dictionary) = resolve_dictionary_value(graph.document, Some(intent))
            && semantic_names::name(graph.document, dictionary.get("S"))
                .is_some_and(|name| name.matches("GTS_PDFA1"))
            && let Some(header) =
                output_intent_icc_header(graph.document, dictionary, graph.limits)?
        {
            return Ok(Some(header.color_space));
        }
    }
    Ok(None)
}
