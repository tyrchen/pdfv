//! PDF/UA painted content and its lexical marked-content context.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    ContentStreamModel, EffectiveResources, GenericModel, ModelGraph, ModelObjectRef,
    ResourceFamily, catalog_value, cos_integer, object_key_text, page_dictionary,
    presence_model_value, push_linked, resolve_dictionary_value, resource_value, semantic_names,
    unknown_property,
};
use crate::{
    CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ParsedDocument, PdfName,
    PropertyName, Result, ValidationError,
    content::{ContentStreamSummary, MarkedContentSpan, MarkedTextProperties, OperatorFact},
};

pub(super) const MARKED: &str = "taggedContentSequence";
pub(super) const PAINTED: &str = "taggedContentItem";
pub(super) const MARKED_PROPERTIES: &[&str] = &[
    "tag",
    "isTaggedContent",
    "parentsTags",
    "parentStructureElementObjectKey",
    "ActualText",
    "Alt",
    "E",
    "Lang",
    "inheritedLang",
    "gContainsCatalogLang",
];
pub(super) const PAINTED_PROPERTIES: &[&str] = &["isTaggedContent", "parentsTags"];

#[derive(Clone, Debug, Default)]
pub(super) struct Context {
    tags: Arc<Vec<PdfName>>,
    parent: Option<StructureParent>,
    marked_language: bool,
}

#[derive(Clone, Copy, Debug)]
struct StructureParent {
    key: ObjectKey,
    reaches_root: bool,
    language: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Evidence {
    context: Context,
    marked: Option<MarkedTextProperties>,
    inherited_language: bool,
    catalog_language: bool,
}

impl Evidence {
    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        let context = &self.context;
        let attributes = self.marked.unwrap_or_default();
        Ok(match name.as_str() {
            "isTaggedContent" => {
                ModelValue::Bool(context.parent.is_some_and(|parent| parent.reaches_root))
            }
            "parentsTags" => {
                ModelValue::List(context.tags.iter().cloned().map(name_value).collect())
            }
            "tag" => context
                .tags
                .last()
                .cloned()
                .map_or(ModelValue::Null, name_value),
            "parentStructureElementObjectKey" => context
                .parent
                .map_or(ModelValue::Null, |parent| ModelValue::ObjectKey(parent.key)),
            "ActualText" => presence_model_value(attributes.actual_text),
            "Alt" => presence_model_value(attributes.alt),
            "E" => presence_model_value(attributes.expansion),
            "Lang" => presence_model_value(attributes.language),
            "inheritedLang" => presence_model_value(self.inherited_language),
            "gContainsCatalogLang" => ModelValue::Bool(self.catalog_language),
            _ => return unknown_property(name),
        })
    }
}

fn name_value(name: PdfName) -> ModelValue {
    ModelValue::from(CosObject::Name(name))
}

struct Scan<'a> {
    graph: &'a ModelGraph<'a>,
    owner: &'a Dictionary,
    resources: EffectiveResources<'a>,
    catalog_language: bool,
    invocations: BTreeMap<u64, Context>,
}

impl Scan<'_> {
    fn parent(&self, mcid: Option<i64>) -> Result<Option<StructureParent>> {
        let Some(key) = self.graph.accessibility_graph()?.content_parent(
            self.graph.document,
            self.owner,
            mcid,
        )?
        else {
            return Ok(None);
        };
        let document = self.graph.document;
        let root = catalog_value(document, "StructTreeRoot");
        let mut link = key;
        let mut visited = BTreeSet::new();
        let mut language = false;
        for depth in 0..=self.graph.limits.max_structure_depth {
            let Some(dictionary) = document
                .objects
                .get(&link)
                .and_then(|object| object.object.as_dictionary())
            else {
                break;
            };
            if matches!(root, Some(CosObject::Reference(root_key)) if *root_key == link)
                && semantic_names::name(document, dictionary.get("Type"))
                    .is_some_and(|name| name.matches("StructTreeRoot"))
            {
                return Ok(Some(StructureParent {
                    key,
                    reaches_root: true,
                    language,
                }));
            }
            if !visited.insert(link) {
                break;
            }
            language |= has_language(document, dictionary.get("Lang"));
            let Some(CosObject::Reference(next)) = dictionary.get("P") else {
                break;
            };
            if depth == self.graph.limits.max_structure_depth {
                return Err(ValidationError::LimitExceeded {
                    limit: "max_structure_depth",
                }
                .into());
            }
            link = *next;
        }
        Ok(Some(StructureParent {
            key,
            reaches_root: false,
            language,
        }))
    }

    fn mark(&self, before: &Context, span: &MarkedContentSpan) -> Result<Evidence> {
        let document = self.graph.document;
        let properties = property_list(document, &self.resources, span);
        let mcid = span.mcid.or_else(|| {
            semantic_names::resolve(
                document,
                properties.and_then(|dictionary| dictionary.get("MCID")),
            )
            .and_then(cos_integer)
        });
        let mut context = before.clone();
        if u32::try_from(context.tags.len()).unwrap_or(u32::MAX)
            >= self.graph.limits.max_structure_depth
        {
            return Err(ValidationError::LimitExceeded {
                limit: "max_structure_depth",
            }
            .into());
        }
        Arc::make_mut(&mut context.tags).push(span.tag.clone());
        if mcid.is_some() {
            context.parent = self.parent(mcid)?.or(context.parent);
        }
        let attributes = properties.map_or(span.inline_text, MarkedTextProperties::from_dictionary);
        let inherited_language =
            before.marked_language || context.parent.is_some_and(|parent| parent.language);
        context.marked_language = inherited_language || attributes.language;
        Ok(Evidence {
            context,
            marked: Some(attributes),
            inherited_language,
            catalog_language: self.catalog_language,
        })
    }

    fn is_painted(&mut self, fact: &OperatorFact, context: &Context) -> bool {
        match fact {
            OperatorFact::TextShow { .. } | OperatorFact::InlineImage { .. } => true,
            OperatorFact::Path { op, .. } => matches!(
                op.as_str(),
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*"
            ),
            OperatorFact::Color { op, .. } => op.as_str() == "sh",
            OperatorFact::XObjectInvoke { name, location } => {
                let document = self.graph.document;
                let resource = self
                    .resources
                    .dictionaries
                    .iter()
                    .rev()
                    .find_map(|dictionary| {
                        resource_value(document, dictionary, ResourceFamily::XObject, name)
                    });
                let Some(CosObject::Stream(stream)) = semantic_names::resolve(document, resource)
                else {
                    return false;
                };
                let subtype = semantic_names::name(document, stream.dictionary.get("Subtype"));
                if subtype.is_some_and(|name| name.matches("Form"))
                    && let Some(offset) = location.offset
                {
                    self.invocations.insert(offset, context.clone());
                }
                subtype.is_some_and(|name| name.matches("Image"))
            }
            _ => false,
        }
    }
}

pub(super) fn property_list<'a>(
    document: &'a ParsedDocument,
    resources: &EffectiveResources<'a>,
    span: &MarkedContentSpan,
) -> Option<&'a Dictionary> {
    resolve_property_list(
        document,
        resources,
        span.properties,
        span.properties_name.as_ref(),
    )
}

pub(super) fn resolve_property_list<'a>(
    document: &'a ParsedDocument,
    resources: &EffectiveResources<'a>,
    reference: Option<ObjectKey>,
    name: Option<&PdfName>,
) -> Option<&'a Dictionary> {
    reference
        .and_then(|key| document.objects.get(&key))
        .and_then(|object| object.object.as_dictionary())
        .or_else(|| {
            let name = name?;
            resources.dictionaries.iter().rev().find_map(|resources| {
                let dictionary = resolve_dictionary_value(document, resources.get("Properties"))?;
                resolve_dictionary_value(document, dictionary.get_name(name))
            })
        })
}

fn has_language(document: &ParsedDocument, value: Option<&CosObject>) -> bool {
    matches!(
        semantic_names::resolve(document, value),
        Some(CosObject::String(_))
    )
}

pub(super) fn append<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    summary: &ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<BTreeMap<u64, Context>> {
    let Some(mut current) = content
        .initial_tag_context
        .clone()
        .or_else(|| (!content.is_form).then(Context::default))
    else {
        return Ok(BTreeMap::new());
    };
    if summary.scoped_operands_truncated {
        return Err(ValidationError::LimitExceeded {
            limit: "content_stream_operands",
        }
        .into());
    }
    let owner = if content.is_form {
        Some(&content.stream.dictionary)
    } else {
        content
            .page_key
            .and_then(|key| page_dictionary(graph.document, key))
    };
    let Some(owner) = owner else {
        return Ok(BTreeMap::new());
    };
    let resources = content.effective_resources(graph.document)?;
    let mut scan = Scan {
        graph,
        owner,
        resources,
        catalog_language: has_language(graph.document, catalog_value(graph.document, "Lang")),
        invocations: BTreeMap::new(),
    };
    current.parent = scan.parent(None)?.or(current.parent);
    let mut scopes = Vec::new();
    let mut spans = summary.marked_content.iter();
    for fact in &summary.facts {
        match fact {
            OperatorFact::MarkedContent { op, .. } if matches!(op.as_str(), "BMC" | "BDC") => {
                let sequence = spans.next().ok_or(ValidationError::SubsystemUnavailable {
                    subsystem: "markedContentSpans",
                })?;
                if !sequence.valid_sequence {
                    scopes.push(current.clone());
                    continue;
                }
                let evidence = scan.mark(&current, sequence)?;
                scopes.push(current);
                current = evidence.context.clone();
                if graph.materializes(MARKED) {
                    emit(graph, MARKED, evidence, fact.location(), objects, max)?;
                }
            }
            OperatorFact::MarkedContent { op, .. } if op.as_str() == "EMC" => {
                if let Some(outer) = scopes.pop() {
                    current = outer;
                }
            }
            _ if scan.is_painted(fact, &current) && graph.materializes(PAINTED) => {
                emit(
                    graph,
                    PAINTED,
                    Evidence {
                        context: current.clone(),
                        marked: None,
                        inherited_language: false,
                        catalog_language: scan.catalog_language,
                    },
                    fact.location(),
                    objects,
                    max,
                )?;
            }
            _ => {}
        }
    }
    Ok(scan.invocations)
}

fn emit<'a>(
    graph: &ModelGraph<'a>,
    family: &'static str,
    evidence: Evidence,
    location: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let context = location.path.as_ref().map_or_else(
        || location.object.map_or_else(String::new, object_key_text),
        |path| path.as_str().to_owned(),
    );
    let mut model = GenericModel::new_value(graph.document, family, ModelValue::Null, 0, context);
    model.key = location.object;
    model.offset = location.offset;
    model.supertypes.clear();
    model.tagged_content = Some(evidence);
    push_linked(objects, ModelObjectRef::Generic(model), max)
}
