//! `DeviceN` colour spaces exposed by PDF content and transparency groups.

use std::{
    collections::{BTreeMap, HashSet},
    ptr,
    sync::Arc,
};

use super::{
    ContentStreamModel, ContentStreamSummary, EffectiveResources, GenericModel, ModelGraph,
    ModelObjectRef, OperatorFact, PageModel, ResourceFamily, device_color::GraphicsState,
    effective_page_resources, effective_resources_for_page_key, push_linked, scoped_resource_value,
    semantic_names, unknown_property, usize_to_f64,
};
use crate::{
    CosObject, Dictionary, ModelValue, ObjectKey, PdfName, PropertyName, Result, ValidationError,
};

pub(crate) const FAMILY: &str = "deviceNColorSpace";
pub(super) const PROPERTIES: &[&str] = &["nrComponents", "areColorantsPresent"];

#[derive(Clone, Copy, Debug)]
pub(super) struct Evidence {
    components: usize,
    colorants_present: bool,
}

impl Evidence {
    pub(super) fn property(self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "nrComponents" => usize_to_f64(self.components).map(ModelValue::Number),
            "areColorantsPresent" => Ok(ModelValue::Bool(self.colorants_present)),
            _ => unknown_property(name),
        }
    }
}

pub(super) fn enabled(graph: &ModelGraph<'_>) -> bool {
    graph.active_flavour.is_some() && graph.materializes(FAMILY)
}

pub(super) fn append_page<'a>(
    graph: &ModelGraph<'a>,
    page: &PageModel<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if enabled(graph)
        && let Some(value) = page.dictionary.get("Group")
    {
        let mut resources = effective_page_resources(graph.document, page)?;
        if let Some(nearest) = resources.dictionaries.pop() {
            resources.dictionaries.clear();
            resources.dictionaries.push(nearest);
        }
        let path = format!("root/page[{}]/group", page.ordinal);
        Scanner::new(graph, &resources, objects, max_objects).group(value, &path)?;
    }
    Ok(())
}

pub(super) fn append_content<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'_>,
    summary: &ContentStreamSummary,
    invocations: &mut BTreeMap<u64, GraphicsState>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !enabled(graph) || content.page_key.is_none() {
        return Ok(());
    }
    if summary.scoped_operands_truncated {
        return Err(ValidationError::LimitExceeded {
            limit: "colour_space_operands",
        }
        .into());
    }
    let resources = content.effective_resources(graph.document)?;
    let path = content.context_prefix.as_str();
    Scanner::new(graph, &resources, objects, max_objects).scan_content(
        content,
        summary,
        invocations,
        path,
    )
}

fn retained_inline_colour(
    graph: &ModelGraph<'_>,
    stream: ObjectKey,
    offset: u64,
    array: &Arc<CosObject>,
) -> Result<Arc<CosObject>> {
    let mut spaces = graph.inline_colour_spaces.try_borrow_mut().map_err(|_| {
        ValidationError::SubsystemUnavailable {
            subsystem: "inline_colour_spaces",
        }
    })?;
    if let Some(canonical) = spaces.get(&(stream, offset)) {
        return Ok(Arc::clone(canonical));
    }
    charge(graph, 1)?;
    spaces.insert((stream, offset), Arc::clone(array));
    Ok(Arc::clone(array))
}

fn charge(graph: &ModelGraph<'_>, visits: usize) -> Result<()> {
    let limit = || ValidationError::LimitExceeded {
        limit: "colour_space_contexts",
    };
    let visits = u64::try_from(visits).map_err(|_| limit())?;
    let count = graph
        .device_n_visits
        .get()
        .checked_add(visits)
        .filter(|count| *count <= graph.limits.max_objects)
        .ok_or_else(limit)?;
    graph.device_n_visits.set(count);
    Ok(())
}

impl<'scope> Scanner<'scope, '_> {
    fn scan_content(
        &mut self,
        content: &'scope ContentStreamModel<'_>,
        summary: &ContentStreamSummary,
        invocations: &mut BTreeMap<u64, GraphicsState>,
        path: &str,
    ) -> Result<()> {
        if matches!(self.resolve(content.stream.dictionary.get("Subtype"))?,
        Some((_, CosObject::Name(name))) if name.matches("Form"))
            && let Some(value) = content.stream.dictionary.get("Group")
        {
            self.group(value, path)?;
        }
        let mut state = self.initial_state(content)?;
        let mut saved = Vec::new();
        let mut uses = summary.resource_uses.iter().peekable();
        let mut selected = HashSet::new();
        let mut device_selectors = HashSet::new();
        let mut recorded_gray = false;
        for fact in &summary.facts {
            let offset = fact.location().offset;
            let resource = uses.next_if(|used| used.location.offset == offset);
            match fact.op_name() {
                "q" => saved.push(state),
                "Q" => {
                    if let Some(previous) = saved.pop() {
                        state = previous;
                    }
                }
                "d1" => state.enabled = false,
                _ => {}
            }
            if let Some(resource) = resource
                && matches!(
                    resource.family,
                    ResourceFamily::ColorSpace
                        | ResourceFamily::Shading
                        | ResourceFamily::XObject
                        | ResourceFamily::Pattern
                )
                && (state.enabled || resource.family == ResourceFamily::XObject)
                && (resource.family != ResourceFamily::Pattern
                    || offset.is_some_and(|offset| invocations.contains_key(&offset)))
                && selected.insert((resource.family.as_str(), &resource.name))
            {
                self.scan_resource(resource.family, &resource.name, path)?;
            }
            match fact {
                OperatorFact::InlineImage {
                    color_space: Some(name),
                    image_mask: false,
                    ..
                } if state.enabled && selected.insert(("inlineImage", name)) => {
                    self.colour(self.colour_resource(name, true)?, path)?;
                }
                OperatorFact::InlineImage {
                    color_space_array: Some(array),
                    image_mask: false,
                    ..
                } if state.enabled => {
                    if let Some(offset) = offset {
                        let canonical =
                            retained_inline_colour(self.graph, content.key, offset, array)?;
                        Scanner::new(self.graph, self.resources, self.objects, self.max_objects)
                            .colour(Some(&canonical), path)?;
                    }
                }
                OperatorFact::Color { op, .. } if state.enabled => {
                    state.select(op.as_str());
                    if let Some(name) = device_name(op.as_str())
                        && device_selectors.insert(name)
                    {
                        let name = PdfName::try_from(name.to_owned())?;
                        self.colour(self.colour_resource(&name, false)?, path)?;
                    }
                }
                OperatorFact::TextState {
                    op, rendering_mode, ..
                } if op.as_str() == "Tr" => {
                    state.text_mode = rendering_mode.unwrap_or(0);
                }
                _ => {}
            }
            if !recorded_gray && self.uses_initial_gray(fact, state)? {
                if let Some(origin) = state.origin {
                    self.paint_initial_gray(origin, path)?;
                }
                recorded_gray = true;
            }
            if let Some(invocation) = offset.and_then(|offset| invocations.get_mut(&offset)) {
                invocation.device_n_state = Some(state);
            }
        }
        Ok(())
    }

    fn initial_state(&self, content: &'scope ContentStreamModel<'_>) -> Result<ColourUseState> {
        let mut state = if let Some(state) = content
            .initial_color_state
            .and_then(|state| state.device_n_state)
        {
            state
        } else {
            let origin = self.initial_gray_origin(content)?;
            ColourUseState {
                enabled: true,
                origin,
                initial_fill: origin.is_some(),
                initial_stroke: origin.is_some(),
                text_mode: 0,
            }
        };
        let dictionary = &content.stream.dictionary;
        let uncoloured_pattern = matches!(
            self.resolve(dictionary.get("PatternType"))?,
            Some((_, CosObject::Integer(1)))
        ) && matches!(
            self.resolve(dictionary.get("PaintType"))?,
            Some((_, CosObject::Integer(2)))
        );
        state.enabled &= !uncoloured_pattern;
        Ok(state)
    }

    fn initial_gray_origin(&self, content: &ContentStreamModel<'_>) -> Result<Option<usize>> {
        if !self
            .resources
            .dictionaries
            .iter()
            .any(|resources| resources.get("ColorSpace").is_some())
        {
            return Ok(None);
        }
        let font = content
            .character_resources
            .as_ref()
            .and_then(|resources| resources.dictionary.clone());
        charge(self.graph, 1)?;
        let mut origins = self
            .graph
            .initial_gray_resources
            .try_borrow_mut()
            .map_err(|_| ValidationError::SubsystemUnavailable {
                subsystem: "device_n",
            })?;
        let origin = origins.len();
        origins.push(GrayResources {
            page: content.page_key,
            stream: content.resource_context.map(|_| content.key),
            font,
        });
        Ok(Some(origin))
    }

    fn paint_initial_gray(&mut self, origin: usize, path: &str) -> Result<()> {
        let unavailable = || ValidationError::SubsystemUnavailable {
            subsystem: "device_n",
        };
        let origins = self
            .graph
            .initial_gray_resources
            .try_borrow()
            .map_err(|_| unavailable())?;
        let origin = origins.get(origin).ok_or_else(unavailable)?;
        let page = origin
            .page
            .map(|page| {
                effective_resources_for_page_key(self.graph.document, page, self.graph.limits)
            })
            .transpose()?
            .and_then(|resources| resources.dictionaries.last().copied());
        let stream = origin
            .stream
            .and_then(|key| self.graph.document.objects.get(&key))
            .and_then(|object| match &object.object {
                CosObject::Stream(stream) => stream.dictionary.get("Resources"),
                _ => None,
            });
        let local = self.dictionary(stream)?.or(origin.font.as_deref());
        let resources = EffectiveResources {
            dictionaries: page.into_iter().chain(local).collect(),
            cyclic: false,
        };
        let mut scanner = Scanner::new(self.graph, &resources, self.objects, self.max_objects);
        let gray = PdfName::try_from("DeviceGray".to_owned())?;
        scanner.colour(scanner.colour_resource(&gray, false)?, path)
    }

    fn scan_resource(&mut self, family: ResourceFamily, name: &PdfName, path: &str) -> Result<()> {
        if family == ResourceFamily::ColorSpace {
            return self.colour(self.colour_resource(name, false)?, path);
        }
        let Some(value) = self.resource(family, name)? else {
            return Ok(());
        };
        match family {
            ResourceFamily::Shading => self.shading(value, path),
            ResourceFamily::XObject => self.image(value, path),
            ResourceFamily::Pattern => {
                if let Some(pattern) = self.dictionary(Some(value))?
                    && matches!(
                        self.resolve(pattern.get("PatternType"))?,
                        Some((_, CosObject::Integer(2)))
                    )
                    && let Some(value) = pattern.get("Shading")
                {
                    self.shading(value, path)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn uses_initial_gray(&self, fact: &OperatorFact, state: ColourUseState) -> Result<bool> {
        Ok(match fact {
            OperatorFact::Path { op, .. } => match op.as_str() {
                "f" | "F" | "f*" => state.initial_fill,
                "S" | "s" => state.initial_stroke,
                "B" | "B*" | "b" | "b*" => state.initial_fill || state.initial_stroke,
                _ => false,
            },
            OperatorFact::TextShow { .. } => match state.text_mode {
                0 | 4 => state.initial_fill,
                1 | 5 => state.initial_stroke,
                2 | 6 => state.initial_fill || state.initial_stroke,
                _ => false,
            },
            OperatorFact::Color { op, .. } if state.enabled && op.as_str() == "scn" => {
                state.initial_fill
            }
            OperatorFact::Color { op, .. } if state.enabled && op.as_str() == "SCN" => {
                state.initial_stroke
            }
            OperatorFact::InlineImage {
                image_mask: true, ..
            } => state.initial_fill,
            OperatorFact::XObjectInvoke { name, .. } if state.initial_fill => {
                let image = self.resource(ResourceFamily::XObject, name)?;
                if let Some(dictionary) = self.dictionary(image)? {
                    matches!(self.resolve(dictionary.get("Subtype"))?, Some((_, CosObject::Name(name))) if name.matches("Image"))
                        && matches!(
                            self.resolve(dictionary.get("ImageMask"))?,
                            Some((_, CosObject::Boolean(true)))
                        )
                } else {
                    false
                }
            }
            _ => false,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ColourUseState {
    enabled: bool,
    origin: Option<usize>,
    initial_fill: bool,
    initial_stroke: bool,
    text_mode: i64,
}

/// Resource scope where the implicit gray was selected, kept through nested paint calls.
#[derive(Debug)]
pub(super) struct GrayResources {
    page: Option<ObjectKey>,
    stream: Option<ObjectKey>,
    font: Option<Arc<Dictionary>>,
}

impl ColourUseState {
    fn select(&mut self, operator: &str) {
        match operator {
            "cs" | "g" | "rg" | "k" => self.initial_fill = false,
            "CS" | "G" | "RG" | "K" => self.initial_stroke = false,
            _ => {}
        }
    }
}

fn device_name(operator: &str) -> Option<&'static str> {
    match operator {
        "g" | "G" => Some("DeviceGray"),
        "rg" | "RG" => Some("DeviceRGB"),
        "k" | "K" => Some("DeviceCMYK"),
        _ => None,
    }
}

struct Scanner<'scope, 'doc> {
    graph: &'scope ModelGraph<'doc>,
    resources: &'scope EffectiveResources<'scope>,
    objects: &'scope mut Vec<ModelObjectRef<'doc>>,
    max_objects: usize,
    colours: HashSet<usize>,
    images: HashSet<(usize, bool)>,
}

impl<'scope, 'doc> Scanner<'scope, 'doc> {
    fn new(
        graph: &'scope ModelGraph<'doc>,
        resources: &'scope EffectiveResources<'scope>,
        objects: &'scope mut Vec<ModelObjectRef<'doc>>,
        max_objects: usize,
    ) -> Self {
        Self {
            graph,
            resources,
            objects,
            max_objects,
            colours: HashSet::new(),
            images: HashSet::new(),
        }
    }

    fn resource(
        &self,
        family: ResourceFamily,
        name: &PdfName,
    ) -> Result<Option<&'scope CosObject>> {
        scoped_resource_value(self.graph, self.resources, family, name)
    }

    fn colour_resource(&self, name: &PdfName, inline: bool) -> Result<Option<&'scope CosObject>> {
        let default = match name.as_bytes() {
            b"DeviceGray" => "DefaultGray",
            b"DeviceRGB" => "DefaultRGB",
            b"DeviceCMYK" => "DefaultCMYK",
            b"G" if inline => "DefaultGray",
            b"RGB" if inline => "DefaultRGB",
            b"CMYK" if inline => "DefaultCMYK",
            _ => return self.resource(ResourceFamily::ColorSpace, name),
        };
        let default = PdfName::try_from(default.to_owned())?;
        if let Some(resources) = self.resources.dictionaries.last()
            && let Some(collection) = self.dictionary(resources.get("ColorSpace"))?
            && let Some(value) = collection.get_name(&default)
            && !matches!(value, CosObject::Null)
        {
            return Ok(Some(value));
        }
        if inline {
            return Ok(None);
        }
        self.resource(ResourceFamily::ColorSpace, name)
    }

    fn resolve(
        &self,
        value: Option<&'scope CosObject>,
    ) -> Result<Option<(Option<ObjectKey>, &'scope CosObject)>> {
        semantic_names::resolve_reference(
            self.graph.document,
            value,
            self.graph.limits.max_object_depth,
        )
    }

    fn dictionary(&self, value: Option<&'scope CosObject>) -> Result<Option<&'scope Dictionary>> {
        Ok(self
            .resolve(value)?
            .and_then(|(_, value)| value.as_dictionary()))
    }

    fn group(&mut self, value: &'scope CosObject, path: &str) -> Result<()> {
        if let Some(group) = self.dictionary(Some(value))? {
            self.colour(group.get("CS"), path)?;
        }
        Ok(())
    }

    fn shading(&mut self, value: &'scope CosObject, path: &str) -> Result<()> {
        if let Some(shading) = self.dictionary(Some(value))? {
            self.colour(shading.get("ColorSpace"), path)?;
        }
        Ok(())
    }

    fn image(&mut self, value: &'scope CosObject, path: &str) -> Result<()> {
        let Some(image) = self.dictionary(Some(value))? else {
            return Ok(());
        };
        if !matches!(self.resolve(image.get("Subtype"))?, Some((_, CosObject::Name(name))) if name.matches("Image"))
        {
            return Ok(());
        }
        let mut pending = vec![(image, false)];
        while let Some((image, hard_mask)) = pending.pop() {
            charge(self.graph, 1)?;
            if !self.images.insert((ptr::from_ref(image).addr(), hard_mask)) {
                continue;
            }
            if !hard_mask {
                if !matches!(
                    self.resolve(image.get("ImageMask"))?,
                    Some((_, CosObject::Boolean(true)))
                ) {
                    self.colour(image.get("ColorSpace"), path)?;
                }
                if let Some((_, CosObject::Stream(mask))) = self.resolve(image.get("Mask"))? {
                    pending.push((&mask.dictionary, true));
                }
            }
            if let Some((_, CosObject::Array(alternates))) =
                self.resolve(image.get("Alternates"))?
            {
                charge(self.graph, alternates.len())?;
                self.check_walk_budget(
                    self.images.len(),
                    pending.len().saturating_add(alternates.len()),
                )?;
                for alternate in alternates {
                    if let Some(dictionary) = self.dictionary(Some(alternate))?
                        && let Some((_, CosObject::Stream(image))) =
                            self.resolve(dictionary.get("Image"))?
                    {
                        pending.push((&image.dictionary, false));
                    }
                }
            }
            self.check_walk_budget(self.images.len(), pending.len())?;
        }
        Ok(())
    }

    fn colour(&mut self, value: Option<&'scope CosObject>, path: &str) -> Result<()> {
        let mut pending: Vec<_> = value.into_iter().collect();
        while let Some(value) = pending.pop() {
            charge(self.graph, 1)?;
            let Some((key, value)) = self.resolve(Some(value))? else {
                continue;
            };
            let address = ptr::from_ref(value).addr();
            if !self.colours.insert(address) {
                continue;
            }
            match value {
                CosObject::Name(name) => {
                    pending.extend(self.colour_resource(name, false)?);
                }
                CosObject::Array(parts) => self.array(parts, address, key, path, &mut pending)?,
                _ => {}
            }
            self.check_walk_budget(self.colours.len(), pending.len())?;
        }
        Ok(())
    }

    fn check_walk_budget(&self, visited: usize, pending: usize) -> Result<()> {
        if u64::try_from(visited.saturating_add(pending)).unwrap_or(u64::MAX)
            > self.graph.limits.max_objects
        {
            return Err(ValidationError::LimitExceeded {
                limit: "colour_space_contexts",
            }
            .into());
        }
        Ok(())
    }

    fn array(
        &mut self,
        parts: &'scope [CosObject],
        address: usize,
        key: Option<ObjectKey>,
        path: &str,
        pending: &mut Vec<&'scope CosObject>,
    ) -> Result<()> {
        if let [value] = parts {
            pending.push(value);
            return Ok(());
        }
        let Some((_, CosObject::Name(family))) = self.resolve(parts.first())? else {
            return Ok(());
        };
        match family.as_bytes() {
            b"DeviceN" => {
                self.append(parts, address, key, path)?;
                pending.extend(parts.get(2));
                if let Some(attributes) = self.dictionary(parts.get(4))? {
                    if let Some(process) = self.dictionary(attributes.get("Process"))? {
                        pending.extend(process.get("ColorSpace"));
                    }
                    if let Some(colorants) = self.dictionary(attributes.get("Colorants"))? {
                        charge(self.graph, colorants.len())?;
                        self.check_walk_budget(
                            self.colours.len(),
                            pending.len().saturating_add(colorants.len()),
                        )?;
                        for (_, value) in colorants.iter() {
                            if let Some((_, CosObject::Array(separation))) =
                                self.resolve(Some(value))?
                                && matches!(self.resolve(separation.first())?, Some((_, CosObject::Name(family))) if family.matches("Separation"))
                            {
                                pending.push(value);
                            }
                        }
                    }
                }
            }
            b"Separation" => pending.extend(parts.get(2)),
            b"Indexed" => pending.extend(parts.get(1)),
            _ => {}
        }
        Ok(())
    }

    fn append(
        &mut self,
        parts: &'scope [CosObject],
        address: usize,
        key: Option<ObjectKey>,
        path: &str,
    ) -> Result<()> {
        let ordinal = {
            let mut seen = self.graph.device_n_spaces.try_borrow_mut().map_err(|_| {
                ValidationError::SubsystemUnavailable {
                    subsystem: "device_n",
                }
            })?;
            if !seen.insert(address) {
                return Ok(());
            }
            seen.len()
        };
        let names = match self.resolve(parts.get(1))? {
            Some((_, CosObject::Array(names))) => names.as_slice(),
            _ => &[],
        };
        charge(self.graph, names.len())?;
        let attributes = self.dictionary(parts.get(4))?;
        let colorants =
            self.dictionary(attributes.and_then(|attributes| attributes.get("Colorants")))?;
        let process =
            self.dictionary(attributes.and_then(|attributes| attributes.get("Process")))?;
        let mut process_components = HashSet::new();
        if let Some((_, CosObject::Array(components))) =
            self.resolve(process.and_then(|process| process.get("Components")))?
        {
            charge(self.graph, components.len())?;
            for value in components {
                if let Some((_, CosObject::Name(name))) = self.resolve(Some(value))? {
                    process_components.insert(name);
                }
            }
        }
        let mut colorants_present = true;
        for value in names {
            let component_present = match self.resolve(Some(value))? {
                Some((_, CosObject::Name(name))) => {
                    matches!(
                        name.as_bytes(),
                        b"Cyan" | b"Magenta" | b"Yellow" | b"Black" | b"None"
                    ) || process_components.contains(name)
                        || colorants.is_some_and(|colorants| colorants.get_name(name).is_some())
                }
                _ => false,
            };
            colorants_present &= component_present;
        }
        let mut model = GenericModel::new_value(
            self.graph.document,
            FAMILY,
            ModelValue::Null,
            ordinal,
            format!("{path}/deviceN[{ordinal}]"),
        );
        model.key = key;
        model.offset = key.and_then(|key| {
            self.graph
                .document
                .objects
                .get(&key)
                .map(|object| object.offset)
        });
        model.device_n = Some(Evidence {
            components: names.len(),
            colorants_present,
        });
        model.supertypes.clear();
        push_linked(
            self.objects,
            ModelObjectRef::Generic(model),
            self.max_objects,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, Result, ValidationFlavour,
        ValidationOptions, Validator,
    };

    #[rstest]
    #[case::unused("", 33, "", (0, 0), (0, 0))]
    #[case::reached("/Ink cs", 33, "", (0, 1), (0, 1))]
    #[case::shared("/Ink cs /Same CS", 33, "", (0, 1), (0, 1))]
    #[case::nested("/Index cs", 32, "", (1, 0), (0, 1))]
    #[case::non_device_n("/RGB cs", 33, "", (0, 0), (0, 0))]
    #[case::explicit_default("0 g", 32, "", (1, 0), (0, 1))]
    #[case::initial_gray("0 0 10 10 re f", 32, "", (1, 0), (0, 1))]
    #[case::restored_gray("q 0 0 0 rg Q 0 0 10 10 re f", 32, "", (1, 0), (0, 1))]
    #[case::replaced_gray("1 0 0 rg 0 0 10 10 re f", 32, "", (0, 0), (0, 0))]
    #[case::inherited_gray("/Form Do", 32, "", (1, 0), (0, 1))]
    #[case::inherited_gray_replaced("1 0 0 rg /Form Do", 32, "", (0, 0), (0, 0))]
    #[case::inline_default("BI /W 1 /H 1 /BPC 8 /CS /G ID x EI", 32, "", (1, 0), (0, 1))]
    #[case::inline_array(
        "BI /W 1 /H 1 /BPC 8 /ColorSpace /Ink /CS [/DeviceN [/Spot] /DeviceCMYK 6 0 R] ID x EI",
        33,
        "",
        (1, 0),
        (0, 1)
    )]
    #[case::wrapped_space("/Wrapper cs", 32, "", (1, 0), (0, 1))]
    #[case::listed("/Ink cs", 32, "<< /Colorants << /Spot null >> >>", (1, 0), (1, 0))]
    #[case::process("/Ink cs", 32, "<< /Process << /Components [/Spot] >> >>", (1, 0), (1, 0))]
    fn test_should_measure_only_device_n_reached_from_color_selectors(
        #[case] content: &str,
        #[case] components: usize,
        #[case] attributes: &str,
        #[case] count: (usize, usize),
        #[case] colorants: (usize, usize),
    ) -> Result<()> {
        let names = "/Spot ".repeat(components);
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /Form 7 0 R >> \
             /ColorSpace << /Ink 5 0 R /Same 5 0 R /DefaultGray 5 0 R /Wrapper [/Ink] /RGB \
             /DeviceRGB /Index [/Indexed 5 0 R 0 <00>] >> >> >>\nendobj\n4 0 obj\n<< /Length {} \
             >>\nstream\n{content}\nendstream\nendobj\n5 0 obj\n[/DeviceN [{names}] /DeviceCMYK 6 \
             0 R {attributes}]\nendobj\n6 0 obj\n<< /FunctionType 2 /Domain [0 1] /N 1 \
             >>\nendobj\n7 0 obj\n<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources \
             << >> /Length 14 >>\nstream\n0 0 10 10 re f\nendstream\nendobj\ntrailer\n<< /Root 1 \
             0 R >>\n%%EOF\n",
            content.len()
        );
        let part = NonZeroU32::new(3).ok_or(ProfileError::UnsupportedSelection)?;
        let flavour = ValidationFlavour::new("pdfa", part, "b")?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        let report = Validator::new(options)?
            .validate_reader(Cursor::new(pdf.as_bytes()), InputName::memory())?;
        let profile = report
            .profile_reports
            .first()
            .ok_or(ProfileError::UnsupportedSelection)?;
        for (rule, expected) in [
            ("iso-19005-3-6-1-13-9", count),
            ("iso-19005-3-6-2-4-4-1", colorants),
        ] {
            let matches_rule = |assertion: &&crate::Assertion| assertion.rule_id.0.as_str() == rule;
            assert_eq!(
                (
                    profile
                        .passed_assertions
                        .iter()
                        .filter(matches_rule)
                        .count(),
                    profile
                        .failed_assertions
                        .iter()
                        .filter(matches_rule)
                        .count()
                ),
                expected
            );
            assert!(
                !profile
                    .unsupported_rules
                    .iter()
                    .any(|item| item.rule_id.0.as_str() == rule)
            );
        }
        Ok(())
    }
}
