//! Use-site traversal of Separation and `DeviceN` colourant names.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ptr,
    sync::Arc,
};

use super::{Collector, Evidence, borrow_scope, owner_location, resolve, resolved_name};
use crate::{
    CosObject, Dictionary, ObjectKey, ObjectLocation, PdfName, Result, ValidationError,
    validation::{
        ContentStreamModel, ContentStreamSummary, EffectiveResources, ModelGraph, ModelObjectRef,
        OperatorFact, PageModel, ResourceFamily, device_color::GraphicsState,
        effective_page_resources, scoped_resource_value,
    },
};

const MAX_COLOUR_DEPTH: u32 = 128;
type ResourceView = [Option<usize>; 2];

#[derive(Debug, Default)]
pub(super) struct Scope {
    emitted: HashSet<(ObjectKey, Option<ResourceView>)>,
    selections: Vec<Arc<Colour>>,
    selection_ids: HashMap<(ResourceView, PdfName), Option<usize>>,
    visits: u64,
}

#[derive(Debug)]
struct Colour {
    key: Option<ObjectKey>,
    resources: Option<ResourceView>,
    names: Vec<Evidence>,
    children: Vec<Self>,
}

impl Colour {
    fn emit(&self, collector: &mut Collector<'_, '_>) -> Result<()> {
        if let Some(key) = self.key
            && !borrow_scope(collector.graph)?
                .colours
                .emitted
                .insert((key, self.resources))
        {
            return Ok(());
        }
        for name in &self.names {
            collector.push(name.clone())?;
        }
        for child in &self.children {
            child.emit(collector)?;
        }
        Ok(())
    }

    fn owned_by(self, key: Option<ObjectKey>) -> Self {
        if key.is_none() || key == self.key {
            return self;
        }
        Self {
            key,
            resources: self.resources,
            names: Vec::new(),
            children: vec![self],
        }
    }
}

type ColourArray<'a> = (Option<ObjectKey>, &'a [CosObject], bool);

pub(super) fn append_resource(
    graph: &ModelGraph<'_>,
    resources: &EffectiveResources<'_>,
    value: &CosObject,
    collector: &mut Collector<'_, '_>,
) -> Result<()> {
    if let Some((_, parts, _)) = colour_array(graph, resources, Some(value))? {
        for_each_colourant(graph, parts, |name| collector.name(name))?;
    }
    Ok(())
}

fn for_each_colourant(
    graph: &ModelGraph<'_>,
    parts: &[CosObject],
    mut emit: impl FnMut(&PdfName) -> Result<()>,
) -> Result<()> {
    let Some(family) = resolved_name(graph, parts.first())? else {
        return Ok(());
    };
    match family.as_bytes() {
        b"Separation" => {
            if let Some(name) = resolved_name(graph, parts.get(1))? {
                emit(name)?;
            }
        }
        b"DeviceN" => {
            if let Some((_, CosObject::Array(names))) = resolve(graph, parts.get(1))? {
                charge(graph, names.len())?;
                for item in names {
                    if let Some(name) = resolved_name(graph, Some(item))? {
                        emit(name)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn colour_array<'a>(
    graph: &ModelGraph<'a>,
    resources: &EffectiveResources<'a>,
    mut value: Option<&'a CosObject>,
) -> Result<Option<ColourArray<'a>>> {
    let mut seen = HashSet::new();
    for _ in 0..=graph.limits.max_object_depth {
        charge(graph, 1)?;
        match resolve(graph, value)? {
            Some((key, CosObject::Array(parts))) => {
                return Ok(Some((key, parts, !seen.is_empty())));
            }
            Some((_, CosObject::Name(name))) if seen.insert(name) => {
                value = lookup(graph, resources, name, false)?;
            }
            _ => return Ok(None),
        }
    }
    Err(ValidationError::LimitExceeded {
        limit: "unicode_colour_depth",
    }
    .into())
}

fn lookup<'a>(
    graph: &ModelGraph<'a>,
    resources: &EffectiveResources<'a>,
    name: &PdfName,
    inline: bool,
) -> Result<Option<&'a CosObject>> {
    let device = match name.as_bytes() {
        b"DeviceGray" => Some("DefaultGray"),
        b"DeviceRGB" => Some("DefaultRGB"),
        b"DeviceCMYK" => Some("DefaultCMYK"),
        b"G" if inline => Some("DefaultGray"),
        b"RGB" if inline => Some("DefaultRGB"),
        b"CMYK" if inline => Some("DefaultCMYK"),
        _ => None,
    };
    if let Some(device) = device {
        let nearest = resources
            .dictionaries
            .last()
            .and_then(|dict| dict.get("ColorSpace"));
        if let Some((_, CosObject::Dictionary(colours))) = resolve(graph, nearest)? {
            return Ok(colours.get(device));
        }
        return Ok(None);
    }
    scoped_resource_value(graph, resources, ResourceFamily::ColorSpace, name)
}

pub(super) fn charge(graph: &ModelGraph<'_>, amount: usize) -> Result<()> {
    let limit = || ValidationError::LimitExceeded {
        limit: "unicode_colour_links",
    };
    let amount = u64::try_from(amount).map_err(|_| limit())?;
    let mut scope = borrow_scope(graph)?;
    scope.colours.visits = scope
        .colours
        .visits
        .checked_add(amount)
        .filter(|total| *total <= graph.limits.max_objects)
        .ok_or_else(limit)?;
    Ok(())
}

struct ColourReader<'g, 'scope> {
    graph: &'g ModelGraph<'scope>,
    resources: &'g EffectiveResources<'scope>,
    active: HashSet<usize>,
}

impl<'g, 'scope> ColourReader<'g, 'scope> {
    fn new(graph: &'g ModelGraph<'scope>, resources: &'g EffectiveResources<'scope>) -> Self {
        Self {
            graph,
            resources,
            active: HashSet::new(),
        }
    }

    fn read(&mut self, value: Option<&'scope CosObject>, depth: u32) -> Result<Option<Colour>> {
        let Some((key, parts, alias)) = colour_array(self.graph, self.resources, value)? else {
            return Ok(None);
        };
        let address = ptr::from_ref(parts).addr();
        if !self.active.insert(address) {
            return Ok(None);
        }
        if depth > self.graph.limits.max_object_depth.min(MAX_COLOUR_DEPTH) {
            return Err(ValidationError::LimitExceeded {
                limit: "unicode_colour_depth",
            }
            .into());
        }
        let colour = self.read_array(key, parts, alias, depth.saturating_add(1))?;
        self.active.remove(&address);
        Ok(colour)
    }

    fn read_array(
        &mut self,
        key: Option<ObjectKey>,
        parts: &'scope [CosObject],
        alias: bool,
        child_depth: u32,
    ) -> Result<Option<Colour>> {
        let Some(family) = resolved_name(self.graph, parts.first())? else {
            return Ok(None);
        };
        let mut names = Vec::new();
        for_each_colourant(self.graph, parts, |name| {
            let evidence = Evidence::from_name(name);
            self.graph.charge_text_bytes(evidence.text.as_str().len())?;
            names.push(evidence);
            Ok(())
        })?;
        let mut children = Vec::new();
        let alternate = match family.as_bytes() {
            b"Separation" | b"DeviceN" => parts.get(2),
            b"Indexed" | b"I" | b"Pattern" => parts.get(1),
            _ => None,
        };
        if alternate.is_some() {
            children.extend(self.read(alternate, child_depth)?);
        }
        if family.matches("DeviceN")
            && let Some((_, CosObject::Dictionary(attributes))) = resolve(self.graph, parts.get(4))?
        {
            self.read_device_n_attributes(attributes, child_depth, &mut children)?;
        }
        let resources = (alias || children.iter().any(|child| child.resources.is_some()))
            .then(|| resource_view(self.resources));
        Ok(
            (!names.is_empty() || !children.is_empty()).then_some(Colour {
                key,
                resources,
                names,
                children,
            }),
        )
    }

    fn read_device_n_attributes(
        &mut self,
        attributes: &'scope Dictionary,
        depth: u32,
        children: &mut Vec<Colour>,
    ) -> Result<()> {
        if let Some((_, CosObject::Dictionary(process))) =
            resolve(self.graph, attributes.get("Process"))?
        {
            children.extend(self.read(process.get("ColorSpace"), depth)?);
        }
        if let Some((_, CosObject::Dictionary(colourants))) =
            resolve(self.graph, attributes.get("Colorants"))?
        {
            charge(self.graph, colourants.len())?;
            for (_, value) in colourants.iter() {
                if let Some((_, parts, _)) = colour_array(self.graph, self.resources, Some(value))?
                    && resolved_name(self.graph, parts.first())?
                        .is_some_and(|name| name.matches("Separation"))
                {
                    children.extend(self.read(Some(value), depth)?);
                }
            }
        }
        Ok(())
    }
}

fn resource_view(resources: &EffectiveResources<'_>) -> ResourceView {
    let entries = &resources.dictionaries;
    [entries.first(), entries.last()]
        .map(|entry| entry.map(|dictionary| ptr::from_ref(*dictionary).addr()))
}

pub(super) fn append_page_group<'doc>(
    graph: &ModelGraph<'doc>,
    page: &PageModel<'doc>,
    objects: &mut Vec<ModelObjectRef<'doc>>,
    max_objects: usize,
) -> Result<()> {
    if page.dictionary.get("Group").is_none() {
        return Ok(());
    }
    let mut resources = effective_page_resources(graph.document, page)?;
    if let Some(nearest) = resources.dictionaries.pop() {
        resources.dictionaries.clear();
        resources.dictionaries.push(nearest);
    }
    let owner = owner_location(
        graph,
        Some(page.key),
        format!("root/page[{}]/Group", page.ordinal),
    );
    let mut collector = Collector::new(graph, objects, max_objects, &owner);
    append_group(
        graph,
        &resources,
        page.dictionary.get("Group"),
        None,
        &mut collector,
    )
}

fn append_group(
    graph: &ModelGraph<'_>,
    resources: &EffectiveResources<'_>,
    group: Option<&CosObject>,
    owner_key: Option<ObjectKey>,
    collector: &mut Collector<'_, '_>,
) -> Result<()> {
    if let Some((_, CosObject::Dictionary(group))) = resolve(graph, group)?
        && resolved_name(graph, group.get("S"))?.is_some_and(|name| name.matches("Transparency"))
        && let Some(colour) = ColourReader::new(graph, resources).read(group.get("CS"), 0)?
    {
        colour.owned_by(owner_key).emit(collector)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct State {
    fill: Option<usize>,
    stroke: Option<usize>,
    text_mode: i64,
    enabled: bool,
}

impl State {
    fn update(&mut self, stroke: bool, selection: Option<usize>) {
        if stroke {
            self.stroke = selection;
        } else {
            self.fill = selection;
        }
    }
}

pub(super) fn append_content<'doc>(
    graph: &ModelGraph<'doc>,
    content: &ContentStreamModel<'_>,
    summary: &ContentStreamSummary,
    invocations: &mut BTreeMap<u64, GraphicsState>,
    objects: &mut Vec<ModelObjectRef<'doc>>,
    max_objects: usize,
) -> Result<()> {
    if summary.scoped_operands_truncated || summary.truncated {
        return Err(ValidationError::LimitExceeded {
            limit: "unicode_colour_operands",
        }
        .into());
    }
    let resources = content.effective_resources(graph.document)?;
    let inherited = content
        .initial_color_state
        .map_or([None; 2], |state| state.unicode_colour_names);
    let path = if inherited == [None; 2] {
        content.context_prefix.clone()
    } else {
        format!("{}/inheritedColour{inherited:?}", content.context_prefix)
    };
    let mut scan = ContentColours {
        graph,
        resources: &resources,
        resource_view: resource_view(&resources),
        objects,
        max_objects,
        path: &path,
        key: content.key,
    };
    if content.is_form && content.stream.dictionary.get("Group").is_some() {
        let group_owner = owner_location(graph, Some(content.key), format!("{path}/Group"));
        append_group(
            graph,
            &resources,
            content.stream.dictionary.get("Group"),
            Some(content.key),
            &mut Collector::new(graph, scan.objects, max_objects, &group_owner),
        )?;
    }
    let [fill, stroke] = inherited;
    let mut state = State {
        fill,
        stroke,
        text_mode: content
            .initial_color_state
            .and_then(|state| state.text_mode)
            .map_or(0, i64::from),
        enabled: !matches!(
            (
                resolve(graph, content.stream.dictionary.get("PatternType"))?,
                resolve(graph, content.stream.dictionary.get("PaintType"))?
            ),
            (
                Some((_, CosObject::Integer(1))),
                Some((_, CosObject::Integer(2)))
            )
        ),
    };
    if !content.is_form {
        let gray = PdfName::try_from("DeviceGray".to_owned())?;
        state.fill = scan.select(&gray)?;
        state.stroke = state.fill;
    }
    scan.walk(summary, state, invocations)
}

struct ContentColours<'g, 'scope, 'doc> {
    graph: &'g ModelGraph<'doc>,
    resources: &'g EffectiveResources<'scope>,
    resource_view: ResourceView,
    objects: &'g mut Vec<ModelObjectRef<'doc>>,
    max_objects: usize,
    path: &'g str,
    key: ObjectKey,
}

impl ContentColours<'_, '_, '_> {
    fn walk(
        &mut self,
        summary: &ContentStreamSummary,
        mut state: State,
        invocations: &mut BTreeMap<u64, GraphicsState>,
    ) -> Result<()> {
        let mut saves = Vec::new();
        let mut uses = summary.resource_uses.iter().peekable();
        for (ordinal, fact) in summary.facts.iter().enumerate() {
            let offset = fact.location().offset;
            let resource = uses.next_if(|used| used.location.offset == offset);
            match fact {
                OperatorFact::GraphicsState { op, .. } if op.as_str() == "q" => saves.push(state),
                OperatorFact::GraphicsState { op, .. } if op.as_str() == "Q" => {
                    if let Some(previous) = saves.pop() {
                        state = previous;
                    }
                }
                OperatorFact::Color { op, .. } => {
                    self.colour_operator(op.as_str(), resource, &mut state, ordinal, offset)?;
                }
                OperatorFact::TextState {
                    op, rendering_mode, ..
                } if op.as_str() == "Tr" => {
                    state.text_mode = rendering_mode.unwrap_or(0);
                }
                OperatorFact::TextShow { .. } if state.enabled => {
                    let fill = matches!(state.text_mode, 0 | 2 | 4 | 6);
                    let stroke = matches!(state.text_mode, 1 | 2 | 5 | 6);
                    self.paint(state, fill, stroke, ordinal, offset)?;
                }
                OperatorFact::Path { op, .. } if state.enabled => {
                    let fill = matches!(op.as_str(), "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                    let stroke = matches!(op.as_str(), "S" | "s" | "B" | "B*" | "b" | "b*");
                    self.paint(state, fill, stroke, ordinal, offset)?;
                }
                OperatorFact::InlineImage {
                    color_space,
                    color_space_array,
                    image_mask,
                    ..
                } if state.enabled => {
                    if *image_mask {
                        self.emit_selection(state.fill, ordinal, offset, "fill")?;
                    } else {
                        let value = if let Some(name) = color_space {
                            lookup(self.graph, self.resources, name, true)?
                        } else {
                            color_space_array.as_deref()
                        };
                        self.emit_value(value, ordinal, offset, "image")?;
                    }
                }
                OperatorFact::XObjectInvoke { name, .. } => {
                    self.image(name, ordinal, offset, state)?;
                }
                _ if fact.op_name() == "d1" => state.enabled = false,
                _ => {}
            }
            if let Some(invocation) = offset.and_then(|offset| invocations.get_mut(&offset)) {
                invocation.unicode_colour_names = [state.fill, state.stroke];
            }
        }
        Ok(())
    }

    fn colour_operator(
        &mut self,
        op: &str,
        resource: Option<&crate::content::ResourceUse>,
        state: &mut State,
        ordinal: usize,
        offset: Option<u64>,
    ) -> Result<()> {
        if !state.enabled {
            return Ok(());
        }
        match op {
            "cs" | "CS" => {
                let selection = if let Some(resource) = resource {
                    self.select(&resource.name)?
                } else {
                    None
                };
                state.update(op == "CS", selection);
                self.emit_selection(selection, ordinal, offset, "select")?;
            }
            "scn" | "SCN" => {
                self.emit_selection(
                    if op == "SCN" {
                        state.stroke
                    } else {
                        state.fill
                    },
                    ordinal,
                    offset,
                    "colour",
                )?;
                if let Some(resource) =
                    resource.filter(|use_site| use_site.family == ResourceFamily::Pattern)
                    && let Some((_, CosObject::Dictionary(pattern))) = resolve(
                        self.graph,
                        scoped_resource_value(
                            self.graph,
                            self.resources,
                            ResourceFamily::Pattern,
                            &resource.name,
                        )?,
                    )?
                {
                    self.shading(pattern.get("Shading"), ordinal, offset)?;
                }
            }
            "g" | "G" | "rg" | "RG" | "k" | "K" => {
                let stroke = op.as_bytes().first().is_some_and(u8::is_ascii_uppercase);
                let device = match op {
                    "g" | "G" => "DeviceGray",
                    "rg" | "RG" => "DeviceRGB",
                    _ => "DeviceCMYK",
                };
                let name = PdfName::try_from(device.to_owned())?;
                let selection = self.select(&name)?;
                state.update(stroke, selection);
                self.emit_selection(selection, ordinal, offset, "select")?;
            }
            "sh" => {
                if let Some(resource) = resource {
                    let shading = scoped_resource_value(
                        self.graph,
                        self.resources,
                        ResourceFamily::Shading,
                        &resource.name,
                    )?;
                    self.shading(shading, ordinal, offset)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn select(&mut self, name: &PdfName) -> Result<Option<usize>> {
        let key = (self.resource_view, name.clone());
        if let Some(index) = borrow_scope(self.graph)?.colours.selection_ids.get(&key) {
            return Ok(*index);
        }
        let value = lookup(self.graph, self.resources, name, false)?;
        let index =
            if let Some(colour) = ColourReader::new(self.graph, self.resources).read(value, 0)? {
                let mut scope = borrow_scope(self.graph)?;
                let index = scope.colours.selections.len();
                scope.colours.selections.push(Arc::new(colour));
                Some(index)
            } else {
                None
            };
        borrow_scope(self.graph)?
            .colours
            .selection_ids
            .insert(key, index);
        Ok(index)
    }

    fn shading(
        &mut self,
        value: Option<&CosObject>,
        ordinal: usize,
        offset: Option<u64>,
    ) -> Result<()> {
        let Some((key, shading)) = resolve(self.graph, value)?
            .and_then(|(key, value)| value.as_dictionary().map(|dict| (key, dict)))
        else {
            return Ok(());
        };
        if let Some(colour) =
            ColourReader::new(self.graph, self.resources).read(shading.get("ColorSpace"), 0)?
        {
            self.emit(&colour.owned_by(key), ordinal, offset, "shading")?;
        }
        Ok(())
    }

    fn image(
        &mut self,
        name: &PdfName,
        ordinal: usize,
        offset: Option<u64>,
        state: State,
    ) -> Result<()> {
        let value =
            scoped_resource_value(self.graph, self.resources, ResourceFamily::XObject, name)?;
        let mut pending: Vec<_> = value.into_iter().collect();
        let mut seen = HashSet::new();
        while let Some(value) = pending.pop() {
            charge(self.graph, 1)?;
            let Some((key, CosObject::Stream(stream))) = resolve(self.graph, Some(value))? else {
                continue;
            };
            let image = &stream.dictionary;
            if !seen.insert(ptr::from_ref(image))
                || !resolved_name(self.graph, image.get("Subtype"))?
                    .is_some_and(|name| name.matches("Image"))
            {
                continue;
            }
            let role = format!("image[{}]", seen.len());
            if matches!(
                resolve(self.graph, image.get("ImageMask"))?,
                Some((_, CosObject::Boolean(true)))
            ) {
                if state.enabled {
                    self.emit_selection(state.fill, ordinal, offset, &role)?;
                }
            } else if let Some(colour) =
                ColourReader::new(self.graph, self.resources).read(image.get("ColorSpace"), 0)?
            {
                self.emit(&colour.owned_by(key), ordinal, offset, &role)?;
            }
            if let Some((_, CosObject::Array(alternates))) =
                resolve(self.graph, image.get("Alternates"))?
            {
                charge(self.graph, alternates.len())?;
                for alternate in alternates.iter().rev() {
                    if let Some((_, CosObject::Dictionary(alternate))) =
                        resolve(self.graph, Some(alternate))?
                    {
                        pending.extend(alternate.get("Image"));
                    }
                }
            }
        }
        Ok(())
    }

    fn paint(
        &mut self,
        state: State,
        fill: bool,
        stroke: bool,
        ordinal: usize,
        offset: Option<u64>,
    ) -> Result<()> {
        if fill {
            self.emit_selection(state.fill, ordinal, offset, "fill")?;
        }
        if stroke {
            self.emit_selection(state.stroke, ordinal, offset, "stroke")?;
        }
        Ok(())
    }

    fn emit_selection(
        &mut self,
        index: Option<usize>,
        ordinal: usize,
        offset: Option<u64>,
        role: &str,
    ) -> Result<()> {
        let Some(index) = index else {
            return Ok(());
        };
        let colour = borrow_scope(self.graph)?
            .colours
            .selections
            .get(index)
            .cloned()
            .ok_or(ValidationError::SubsystemUnavailable {
                subsystem: "unicode_colour_selection",
            })?;
        self.emit(&colour, ordinal, offset, role)
    }

    fn emit_value(
        &mut self,
        value: Option<&CosObject>,
        ordinal: usize,
        offset: Option<u64>,
        role: &str,
    ) -> Result<()> {
        if value.is_some()
            && let Some(colour) = ColourReader::new(self.graph, self.resources).read(value, 0)?
        {
            self.emit(&colour, ordinal, offset, role)?;
        }
        Ok(())
    }

    fn emit(
        &mut self,
        colour: &Colour,
        ordinal: usize,
        offset: Option<u64>,
        role: &str,
    ) -> Result<()> {
        let owner = ObjectLocation {
            object: Some(self.key),
            offset,
            path: Some(crate::BoundedText::unchecked(format!(
                "{}/operators[{ordinal}]/{role}",
                self.path
            ))),
        };
        let mut collector = Collector::new(self.graph, self.objects, self.max_objects, &owner);
        colour.emit(&mut collector)
    }
}
