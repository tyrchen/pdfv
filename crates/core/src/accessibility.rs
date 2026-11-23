//! Bounded structure-tree traversal and accessibility semantic summaries.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    num::NonZeroU32,
};

use crate::{
    BoundedText, CosObject, Dictionary, ObjectKey, ObjectLocation, ParsedDocument, PdfName,
    ResourceLimits, Result, ValidationError, ValidationWarning,
};

const DEFAULT_MAX_STRUCTURE_NODES: u64 = 100_000;
const DEFAULT_MAX_STRUCTURE_DEPTH: u32 = 128;
const DEFAULT_MAX_PARENT_TREE_ENTRIES: u64 = 100_000;
const MAX_ROLE_MAP_STEPS: u32 = 32;
const PDF_17_NAMESPACE: &str = "http://iso.org/pdf/ssn";
const PDF_20_NAMESPACE: &str = "http://iso.org/pdf2/ssn";
pub(crate) const MATH_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
const STANDARD_ROLES: &[&str] = &[
    "Document",
    "Part",
    "Art",
    "Sect",
    "Div",
    "BlockQuote",
    "Caption",
    "TOC",
    "TOCI",
    "Index",
    "NonStruct",
    "Private",
    "P",
    "H",
    "H1",
    "H2",
    "H3",
    "H4",
    "H5",
    "H6",
    "L",
    "LI",
    "Lbl",
    "LBody",
    "Table",
    "TR",
    "TH",
    "TD",
    "THead",
    "TBody",
    "TFoot",
    "Span",
    "Quote",
    "Note",
    "Reference",
    "BibEntry",
    "Code",
    "Link",
    "Annot",
    "Ruby",
    "RB",
    "RP",
    "RT",
    "Warichu",
    "WP",
    "WT",
    "Figure",
    "Formula",
    "Form",
    "Artifact",
    "Strong",
    "Em",
    "Title",
    "FENote",
    "Aside",
    "Sub",
    "DocumentFragment",
];

/// Default maximum structure nodes retained while building an accessibility graph.
pub(crate) const fn default_max_structure_nodes() -> u64 {
    DEFAULT_MAX_STRUCTURE_NODES
}

/// Default maximum structure tree nesting depth.
pub(crate) const fn default_max_structure_depth() -> u32 {
    DEFAULT_MAX_STRUCTURE_DEPTH
}

/// Default maximum parent-tree or ID-tree entries traversed.
pub(crate) const fn default_max_parent_tree_entries() -> u64 {
    DEFAULT_MAX_PARENT_TREE_ENTRIES
}

/// Stable accessibility node id.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AccessibilityNodeId(pub(crate) usize);

/// Bounded accessibility graph for one document.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "accessibility graph exposes independent PDF feature facts used by profile rules"
)]
pub(crate) struct AccessibilityGraph {
    /// Whether the catalog declares `/MarkInfo << /Marked true >>`.
    pub tagged: bool,
    /// Catalog language string byte count and safe value when ASCII-compatible.
    pub language: Option<String>,
    /// Whether `/StructTreeRoot` exists and has a dictionary shape.
    pub has_structure_tree_root: bool,
    /// Whether the root itself directly lists a content item under `/K`.
    pub root_has_content_items: bool,
    /// Whether `/RoleMap` exists.
    pub role_map_present: bool,
    /// Whether `/ClassMap` exists.
    pub class_map_present: bool,
    /// Number of bounded ID-tree entries observed.
    pub id_tree_entries: u64,
    /// Bounded ID-tree mappings.
    pub id_tree: BTreeMap<String, ObjectKey>,
    /// Number of bounded `/ClassMap` entries observed.
    pub class_map_entries: u64,
    /// Number of bounded parent-tree entries observed.
    pub parent_tree_entries: u64,
    /// Next parent-tree key declared by the structure tree root.
    pub parent_tree_next_key: Option<i64>,
    /// Structure nodes in deterministic traversal order.
    pub nodes: Vec<AccessibilityNode>,
    /// Marked-content associations derived from content streams and parent tree data.
    pub marked_content: Vec<AccessibilityMarkedContent>,
    /// Annotation/object-reference associations from structure elements.
    pub object_references: Vec<AccessibilityObjectReference>,
    /// Artifact content facts derived from marked-content tags or roles.
    pub artifacts: Vec<AccessibilityArtifact>,
    /// Recoverable graph construction warnings.
    pub warnings: Vec<ValidationWarning>,
    /// Whether construction stopped at a configured cap.
    pub truncated: bool,
    content_parents: NumberTree,
    nodes_by_key: BTreeMap<ObjectKey, AccessibilityNodeId>,
}

impl AccessibilityGraph {
    pub(crate) fn node_from_key(&self, key: ObjectKey) -> Option<&AccessibilityNode> {
        self.node(*self.nodes_by_key.get(&key)?)
    }

    pub(crate) fn content_parent(
        &self,
        document: &ParsedDocument,
        owner: &Dictionary,
        mcid: Option<i64>,
    ) -> Result<Option<ObjectKey>> {
        let tree = &self.content_parents;
        if tree.capped {
            return Err(ValidationError::LimitExceeded {
                limit: "max_parent_tree_entries",
            }
            .into());
        }
        if let Some(key) = integer_value(resolved_value(document, owner.get("StructParent"))) {
            return Ok(tree.structure_refs.get(&key).copied());
        }
        Ok(
            integer_value(resolved_value(document, owner.get("StructParents")))
                .zip(mcid)
                .and_then(|(parent, mcid)| tree.mcid_parent(document, parent, mcid)),
        )
    }
}

/// One traversed structure element.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "structure element facts mirror independent veraPDF accessibility properties"
)]
pub(crate) struct AccessibilityNode {
    /// Graph-local id.
    pub id: AccessibilityNodeId,
    /// Indirect object key when available.
    pub key: Option<ObjectKey>,
    /// Original role from `/S`.
    pub role: String,
    /// Role after deterministic `/RoleMap` normalization.
    pub normalized_role: String,
    /// Parent node when known from traversal.
    pub parent: Option<AccessibilityNodeId>,
    /// Structure namespace URL from `/NS` when declared.
    pub namespace_url: Option<String>,
    /// Namespace of the resolved standard role.
    pub standard_namespace_url: Option<String>,
    /// Original standard role when a role map overrides it.
    pub remapped_standard_role: Option<String>,
    /// First role changed within the same declared namespace.
    pub same_namespace_remapping: Option<String>,
    /// Associated page object.
    pub page: Option<ObjectKey>,
    /// Associated page ordinal.
    pub page_ordinal: Option<usize>,
    /// Source location.
    pub location: ObjectLocation,
    /// Child node ids.
    pub children: Vec<AccessibilityNodeId>,
    /// Content items listed under `/K`.
    pub content_items: Vec<AccessibilityContentItem>,
    /// Whether the dictionary declares `/Alt`.
    pub has_alt_text: bool,
    /// Bounded `/Alt` byte count.
    pub alt_text_bytes: u64,
    /// Whether the dictionary declares `/ActualText`.
    pub has_actual_text: bool,
    /// Bounded `/ActualText` byte count.
    pub actual_text_bytes: u64,
    /// Whether the dictionary declares `/E`.
    pub has_expansion_text: bool,
    /// Bounded `/E` byte count.
    pub expansion_text_bytes: u64,
    /// Whether `/Lang` exists on this element.
    pub has_language: bool,
    /// Bounded `/Lang` value.
    pub language: Option<String>,
    /// Whether `/A` attributes exist.
    pub has_attributes: bool,
    /// Bounded role attribute value from `/A` when present.
    pub role_attribute: Option<String>,
    /// Bounded list numbering attribute value from `/A` when present.
    pub list_numbering: Option<String>,
    /// Bounded note type attribute value from `/A` when present.
    pub note_type: Option<String>,
    /// Table row span from `/A /RowSpan`, defaulting to one.
    pub row_span: u32,
    /// Table column span from `/A /ColSpan`, defaulting to one.
    pub col_span: u32,
    /// Header identifiers from `/A /Headers`.
    pub headers: Vec<String>,
    /// Header scope from `/A /Scope`.
    pub scope: Option<String>,
    /// Whether `/C` class mapping references exist.
    pub has_class: bool,
    /// Whether `/ID` exists.
    pub has_id: bool,
    /// Bounded `/ID` value when string-like.
    pub id_text: Option<String>,
    /// Whether this element's `/ID` resolves through `/IDTree`.
    pub id_tree_resolved: bool,
    /// Whether `/P` exists.
    pub contains_parent: bool,
    /// Whether traversal observed an object reference under `/K`.
    pub contains_ref: bool,
    /// Whether this element is role-normalized to a non-standard role.
    pub non_standard_role: bool,
    /// Whether role-map normalization detected a cycle.
    pub circular_role_mapping: bool,
    /// Whether `/C` class references resolve through `/ClassMap`.
    pub class_map_resolved: bool,
}

/// Structure content item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccessibilityContentItem {
    /// Marked-content id associated with the element.
    Mcid {
        /// MCID value.
        mcid: i64,
        /// Associated page.
        page: Option<ObjectKey>,
    },
    /// Object reference associated with the element.
    Object {
        /// Referenced object.
        object: Option<ObjectKey>,
        /// Associated page.
        page: Option<ObjectKey>,
    },
}

/// Association between marked content and a structure element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityMarkedContent {
    /// Owning structure node.
    pub node: Option<AccessibilityNodeId>,
    /// Page ordinal.
    pub page_ordinal: usize,
    /// Content stream object.
    pub stream: ObjectKey,
    /// Marked-content tag.
    pub tag: String,
    /// MCID when known.
    pub mcid: Option<i64>,
    /// Whether the content stream contains text-show operators.
    pub has_text: bool,
    /// Whether the associated content is image-like.
    pub has_image: bool,
    /// Deterministic source location.
    pub location: ObjectLocation,
}

/// Association between a structure element and an object reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityObjectReference {
    /// Owning structure node.
    pub node: AccessibilityNodeId,
    /// Referenced object.
    pub object: Option<ObjectKey>,
    /// Associated page ordinal.
    pub page_ordinal: Option<usize>,
    /// Whether the referenced object is a link annotation.
    pub is_link_annotation: bool,
}

/// Artifact semantic fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityArtifact {
    /// Structure node when the artifact is structure-backed.
    pub node: Option<AccessibilityNodeId>,
    /// Page ordinal when known.
    pub page_ordinal: Option<usize>,
    /// Artifact tag or role.
    pub tag: String,
    /// Deterministic source location.
    pub location: ObjectLocation,
}

#[derive(Clone, Debug)]
struct TraversalItem<'a> {
    value: &'a CosObject,
    parent: Option<AccessibilityNodeId>,
    inherited_page: Option<ObjectKey>,
    depth: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct NumberTree {
    entries: u64,
    mcid_arrays: BTreeMap<i64, McidArray>,
    structure_refs: BTreeMap<i64, ObjectKey>,
    object_refs: BTreeMap<ObjectKey, ObjectKey>,
    capped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum McidArray {
    Indirect(ObjectKey),
    Direct(Vec<Option<ObjectKey>>),
}

impl NumberTree {
    fn mcid_parent(&self, document: &ParsedDocument, parent: i64, mcid: i64) -> Option<ObjectKey> {
        let index = usize::try_from(mcid).ok()?;
        match self.mcid_arrays.get(&parent)? {
            McidArray::Direct(items) => items.get(index).copied().flatten(),
            McidArray::Indirect(key) => {
                let CosObject::Array(items) = &document.objects.get(key)?.object else {
                    return None;
                };
                items.get(index).and_then(object_ref)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentTreeOwnerKind {
    Page,
    Annotation,
    Image,
    Form,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParentTreeOwner {
    object: ObjectKey,
    page_ordinal: Option<usize>,
    kind: ParentTreeOwnerKind,
}

#[derive(Debug)]
struct KidCollector<'a, 'g> {
    document: &'a ParsedDocument,
    node_id: AccessibilityNodeId,
    page: Option<ObjectKey>,
    page_ordinals: &'g BTreeMap<ObjectKey, usize>,
    graph: &'g mut AccessibilityGraph,
    stack: &'g mut Vec<TraversalItem<'a>>,
    depth: u32,
}

#[derive(Clone, Debug)]
struct PageContent<'a> {
    key: ObjectKey,
    ordinal: usize,
    dictionary: &'a Dictionary,
}

/// Builds a bounded accessibility graph for a parsed document.
///
/// # Errors
///
/// Returns [`crate::PdfvError`] when checked arithmetic or configured hard
/// limits prevent deterministic graph construction.
pub(crate) fn build_accessibility_graph(
    document: &ParsedDocument,
    limits: &ResourceLimits,
) -> Result<AccessibilityGraph> {
    let pages = collect_pages(document, limits)?;
    let page_ordinals = pages
        .iter()
        .map(|page| (page.key, page.ordinal))
        .collect::<BTreeMap<_, _>>();
    let parent_tree_owners = collect_parent_tree_owners(document, &pages);
    let catalog = catalog_dictionary(document);
    let tagged = catalog.is_some_and(|catalog| catalog_marked(document, catalog));
    let language = catalog.and_then(|catalog| text_string(catalog.get("Lang")));
    let Some(struct_tree) =
        catalog.and_then(|catalog| dictionary_from_value(document, catalog.get("StructTreeRoot")))
    else {
        return Ok(AccessibilityGraph {
            tagged,
            language,
            has_structure_tree_root: false,
            root_has_content_items: false,
            role_map_present: false,
            class_map_present: false,
            id_tree_entries: 0,
            id_tree: BTreeMap::new(),
            class_map_entries: 0,
            parent_tree_entries: 0,
            parent_tree_next_key: None,
            nodes: Vec::new(),
            marked_content: Vec::new(),
            object_references: Vec::new(),
            artifacts: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
            content_parents: NumberTree::default(),
            nodes_by_key: BTreeMap::new(),
        });
    };

    let mut warnings = Vec::new();
    let id_tree = struct_tree
        .get("IDTree")
        .map_or_else(BTreeMap::new, |value| {
            collect_id_tree(document, value, limits, &mut warnings)
        });
    let id_tree_entries =
        u64::try_from(id_tree.len()).map_err(|_| ValidationError::LimitExceeded {
            limit: "max_parent_tree_entries",
        })?;
    let class_map_entries = class_map_entry_count(struct_tree, limits, &mut warnings)?;
    let parent_tree = struct_tree
        .get("ParentTree")
        .map_or_else(NumberTree::default, |value| {
            collect_parent_tree(document, value, limits, &parent_tree_owners, &mut warnings)
        });
    let parent_tree_next_key = integer_value(struct_tree.get("ParentTreeNextKey"));
    let mut graph = AccessibilityGraph {
        tagged,
        language,
        has_structure_tree_root: true,
        root_has_content_items: root_has_content_items(document, struct_tree),
        role_map_present: struct_tree.get("RoleMap").is_some(),
        class_map_present: struct_tree.get("ClassMap").is_some(),
        id_tree_entries,
        id_tree,
        class_map_entries,
        parent_tree_entries: parent_tree.entries,
        parent_tree_next_key,
        nodes: Vec::new(),
        marked_content: Vec::new(),
        object_references: Vec::new(),
        artifacts: Vec::new(),
        warnings,
        truncated: false,
        content_parents: NumberTree::default(),
        nodes_by_key: BTreeMap::new(),
    };

    if let Some(kids) = struct_tree.get("K") {
        traverse_structure_tree(
            document,
            kids,
            limits,
            struct_tree,
            &page_ordinals,
            &mut graph,
        )?;
    }
    graph.nodes_by_key = graph
        .nodes
        .iter()
        .filter_map(|node| node.key.map(|key| (key, node.id)))
        .collect();
    collect_content_associations(document, limits, &pages, &parent_tree, &mut graph);
    collect_parent_tree_object_associations(
        document,
        &parent_tree,
        &parent_tree_owners,
        &mut graph,
    );
    collect_artifacts(&mut graph);
    graph.content_parents = parent_tree;
    Ok(graph)
}

#[allow(
    clippy::too_many_lines,
    reason = "iterative structure traversal keeps the state machine local and avoids recursive \
              helper churn"
)]
fn traverse_structure_tree(
    document: &ParsedDocument,
    root: &CosObject,
    limits: &ResourceLimits,
    struct_tree: &Dictionary,
    page_ordinals: &BTreeMap<ObjectKey, usize>,
    graph: &mut AccessibilityGraph,
) -> Result<()> {
    let mut stack = Vec::from([TraversalItem {
        value: root,
        parent: None,
        inherited_page: None,
        depth: 0,
    }]);
    let mut visited = HashSet::new();
    while let Some(item) = stack.pop() {
        if graph.truncated {
            break;
        }
        if item.depth > limits.max_structure_depth {
            graph.truncated = true;
            graph
                .warnings
                .push(warning("structure tree depth cap reached"));
            continue;
        }
        if let Some(CosObject::Array(values)) = resolved_value(document, Some(item.value)) {
            if let Some(key) = object_ref(item.value)
                && !visited.insert(key)
            {
                graph
                    .warnings
                    .push(warning("structure tree array cycle detected"));
                continue;
            }
            for child in values.iter().rev() {
                stack.push(TraversalItem {
                    value: child,
                    parent: item.parent,
                    inherited_page: item.inherited_page,
                    depth: item.depth,
                });
            }
            continue;
        }
        let Some((key, offset, dictionary)) = structure_dictionary(document, item.value) else {
            continue;
        };
        if let Some(key) = key
            && !visited.insert(key)
        {
            graph
                .warnings
                .push(warning("structure tree cycle detected"));
            continue;
        }
        let Some(role) = role_name(document, dictionary) else {
            continue;
        };
        if u64::try_from(graph.nodes.len()).unwrap_or(u64::MAX) >= limits.max_structure_nodes {
            graph.truncated = true;
            graph.warnings.push(warning("structure node cap reached"));
            continue;
        }
        let mapped = resolve_role(document, struct_tree, dictionary, &role);
        let page = dictionary
            .get("Pg")
            .and_then(object_ref)
            .or(item.inherited_page);
        let id = AccessibilityNodeId(graph.nodes.len());
        if let Some(parent) = item.parent
            && let Some(parent_node) = graph.nodes.get_mut(parent.0)
        {
            parent_node.children.push(id);
        }
        let mut node = AccessibilityNode {
            id,
            key,
            role,
            normalized_role: mapped.name,
            parent: item.parent,
            namespace_url: namespace_url(document, dictionary),
            standard_namespace_url: mapped.standard_namespace_url,
            remapped_standard_role: mapped.remapped_standard_role,
            same_namespace_remapping: mapped.same_namespace_remapping,
            page,
            page_ordinal: page.and_then(|page| page_ordinals.get(&page).copied()),
            location: ObjectLocation {
                object: key,
                offset,
                path: Some(BoundedText::unchecked(format!(
                    "root/structureElement[{}]",
                    id.0
                ))),
            },
            children: Vec::new(),
            content_items: Vec::new(),
            has_alt_text: dictionary.get("Alt").is_some(),
            alt_text_bytes: text_bytes(dictionary.get("Alt")),
            has_actual_text: dictionary.get("ActualText").is_some(),
            actual_text_bytes: text_bytes(dictionary.get("ActualText")),
            has_expansion_text: dictionary.get("E").is_some(),
            expansion_text_bytes: text_bytes(dictionary.get("E")),
            has_language: dictionary.get("Lang").is_some(),
            language: text_string(dictionary.get("Lang")),
            has_attributes: dictionary.get("A").is_some(),
            role_attribute: attribute_text(document, dictionary, "Role"),
            list_numbering: attribute_text(document, dictionary, "ListNumbering"),
            note_type: attribute_text(document, dictionary, "NoteType"),
            row_span: attribute_positive_u32(document, dictionary, "RowSpan").unwrap_or(1),
            col_span: attribute_positive_u32(document, dictionary, "ColSpan").unwrap_or(1),
            headers: attribute_texts(document, dictionary, "Headers"),
            scope: attribute_text(document, dictionary, "Scope"),
            has_class: dictionary.get("C").is_some(),
            has_id: dictionary.get("ID").is_some(),
            id_text: element_id(dictionary),
            id_tree_resolved: element_id(dictionary).is_some_and(|id| {
                graph
                    .id_tree
                    .get(&id)
                    .is_some_and(|mapped| Some(*mapped) == key)
            }),
            contains_parent: dictionary.get("P").is_some(),
            contains_ref: false,
            non_standard_role: !mapped.is_standard,
            circular_role_mapping: mapped.circular,
            class_map_resolved: class_references_resolve(struct_tree, dictionary),
        };
        let mut collector = KidCollector {
            document,
            node_id: id,
            page,
            page_ordinals,
            graph,
            stack: &mut stack,
            depth: item.depth,
        };
        collect_kids(dictionary.get("K"), &mut node, &mut collector)?;
        graph.nodes.push(node);
    }
    Ok(())
}

fn root_has_content_items(document: &ParsedDocument, root: &Dictionary) -> bool {
    let is_item = |value| match resolved_value(document, Some(value)) {
        Some(CosObject::Integer(_)) => true,
        Some(CosObject::Dictionary(dictionary)) => is_mcr(dictionary) || is_objr(dictionary),
        _ => false,
    };
    let Some(kids) = resolved_value(document, root.get("K")) else {
        return false;
    };
    if let CosObject::Array(values) = kids {
        values.iter().any(is_item)
    } else {
        is_item(kids)
    }
}

fn collect_kids<'a>(
    kids: Option<&'a CosObject>,
    node: &mut AccessibilityNode,
    collector: &mut KidCollector<'a, '_>,
) -> Result<()> {
    let Some(kids) = kids else {
        return Ok(());
    };
    if let Some(CosObject::Array(values)) = resolved_value(collector.document, Some(kids)) {
        for child in values.iter().rev() {
            collect_kid_value(child, node, collector)?;
        }
    } else {
        collect_kid_value(kids, node, collector)?;
    }
    Ok(())
}

fn collect_kid_value<'a>(
    value: &'a CosObject,
    node: &mut AccessibilityNode,
    collector: &mut KidCollector<'a, '_>,
) -> Result<()> {
    if let Some(CosObject::Integer(mcid)) = resolved_value(collector.document, Some(value)) {
        node.content_items.push(AccessibilityContentItem::Mcid {
            mcid: *mcid,
            page: collector.page,
        });
        return Ok(());
    }

    let Some((_key, _offset, dictionary)) = structure_dictionary(collector.document, value) else {
        return Ok(());
    };
    if is_mcr(dictionary) {
        let mcid = integer_value(resolved_value(collector.document, dictionary.get("MCID")));
        let item_page = dictionary.get("Pg").and_then(object_ref).or(collector.page);
        if let Some(mcid) = mcid {
            node.content_items.push(AccessibilityContentItem::Mcid {
                mcid,
                page: item_page,
            });
        }
    } else if is_objr(dictionary) {
        node.contains_ref = true;
        let object = dictionary.get("Obj").and_then(object_ref);
        let item_page = dictionary.get("Pg").and_then(object_ref).or(collector.page);
        node.content_items.push(AccessibilityContentItem::Object {
            object,
            page: item_page,
        });
        collector
            .graph
            .object_references
            .push(AccessibilityObjectReference {
                node: collector.node_id,
                object,
                page_ordinal: item_page
                    .and_then(|page| collector.page_ordinals.get(&page).copied()),
                is_link_annotation: object
                    .and_then(|key| collector.document.objects.get(&key))
                    .and_then(|object| object.object.as_dictionary())
                    .is_some_and(is_link_annotation),
            });
    } else {
        collector.stack.push(TraversalItem {
            value,
            parent: Some(collector.node_id),
            inherited_page: dictionary.get("Pg").and_then(object_ref).or(collector.page),
            depth: collector
                .depth
                .checked_add(1)
                .ok_or(ValidationError::LimitExceeded {
                    limit: "max_structure_depth",
                })?,
        });
    }
    Ok(())
}

fn collect_content_associations(
    document: &ParsedDocument,
    limits: &ResourceLimits,
    pages: &[PageContent<'_>],
    parent_tree: &NumberTree,
    graph: &mut AccessibilityGraph,
) {
    for page in pages {
        let struct_parent = integer_value(page.dictionary.get("StructParents"));
        let properties = page_properties(document, page.dictionary);
        for (ordinal, (stream_key, stream)) in content_streams(document, page.dictionary)
            .into_iter()
            .enumerate()
        {
            let decoded = match stream.decoded_bytes(limits) {
                Ok(decoded) => decoded,
                Err(error) => {
                    graph.warnings.push(warning(&format!(
                        "content stream decode skipped for accessibility graph: {error}"
                    )));
                    continue;
                }
            };
            let summary = match crate::content::summarize_content_stream(
                stream_key,
                &format!("root/page[{}]/contentStream[{ordinal}]", page.ordinal),
                &decoded,
                limits,
            ) {
                Ok(summary) => summary,
                Err(error) => {
                    graph.warnings.push(warning(&format!(
                        "content stream parse skipped for accessibility graph: {error}"
                    )));
                    continue;
                }
            };
            for span in summary.marked_content {
                let mcid = span
                    .mcid
                    .or_else(|| {
                        span.properties
                            .and_then(|key| mcid_from_object(document, key))
                    })
                    .or_else(|| {
                        span.properties_name.as_ref().and_then(|name| {
                            properties
                                .get(name)
                                .and_then(|value| mcid_from_value(document, value))
                        })
                    });
                let node = mcid.and_then(|mcid| {
                    explicit_node_for_mcid(graph, page.key, mcid).or_else(|| {
                        struct_parent.and_then(|parent| {
                            parent_tree
                                .mcid_parent(document, parent, mcid)
                                .and_then(|key| graph.nodes_by_key.get(&key).copied())
                        })
                    })
                });
                graph.marked_content.push(AccessibilityMarkedContent {
                    node,
                    page_ordinal: page.ordinal,
                    stream: stream_key,
                    tag: name_to_string(&span.tag),
                    mcid,
                    has_text: span.has_text,
                    has_image: span.has_image
                        || node.is_some_and(|node| {
                            graph
                                .nodes
                                .get(node.0)
                                .is_some_and(|node| node.normalized_role == "Figure")
                        }),
                    location: span.location,
                });
            }
        }
    }
}

fn explicit_node_for_mcid(
    graph: &AccessibilityGraph,
    page: ObjectKey,
    mcid: i64,
) -> Option<AccessibilityNodeId> {
    graph.nodes.iter().find_map(|node| {
        node.content_items
            .iter()
            .any(|item| {
                matches!(
                    item,
                    AccessibilityContentItem::Mcid {
                        mcid: item_mcid,
                        page: Some(item_page),
                    } if *item_mcid == mcid && *item_page == page
                ) || matches!(
                    item,
                    AccessibilityContentItem::Mcid {
                        mcid: item_mcid,
                        page: None,
                    } if *item_mcid == mcid && node.page == Some(page)
                )
            })
            .then_some(node.id)
    })
}

fn collect_artifacts(graph: &mut AccessibilityGraph) {
    let mut seen = BTreeSet::new();
    for node in &graph.nodes {
        if node.normalized_role == "Artifact"
            && seen.insert((
                Some(node.id),
                node.page_ordinal,
                node.normalized_role.clone(),
            ))
        {
            graph.artifacts.push(AccessibilityArtifact {
                node: Some(node.id),
                page_ordinal: node.page_ordinal,
                tag: node.normalized_role.clone(),
                location: node.location.clone(),
            });
        }
    }
    for marked in &graph.marked_content {
        if marked.tag == "Artifact"
            && seen.insert((marked.node, Some(marked.page_ordinal), marked.tag.clone()))
        {
            graph.artifacts.push(AccessibilityArtifact {
                node: marked.node,
                page_ordinal: Some(marked.page_ordinal),
                tag: marked.tag.clone(),
                location: marked.location.clone(),
            });
        }
    }
}

fn collect_parent_tree(
    document: &ParsedDocument,
    value: &CosObject,
    limits: &ResourceLimits,
    owners: &BTreeMap<i64, ParentTreeOwner>,
    warnings: &mut Vec<ValidationWarning>,
) -> NumberTree {
    let mut tree = NumberTree::default();
    let mut stack = Vec::from([value]);
    let mut visited = HashSet::new();
    while let Some(value) = stack.pop() {
        let Some((key, _offset, dictionary)) = dictionary_ref_or_inline(document, value) else {
            continue;
        };
        if let Some(key) = key
            && !visited.insert(key)
        {
            warnings.push(warning("parent tree cycle detected"));
            continue;
        }
        if let Some(CosObject::Array(kids)) = resolved_value(document, dictionary.get("Kids")) {
            for kid in kids.iter().rev() {
                stack.push(kid);
            }
        }
        if let Some(CosObject::Array(nums)) = resolved_value(document, dictionary.get("Nums")) {
            for pair in nums.chunks(2) {
                if tree.entries >= limits.max_parent_tree_entries {
                    warnings.push(warning("parent tree entry cap reached"));
                    tree.capped = true;
                    return tree;
                }
                let Some(CosObject::Integer(parent_key)) = resolved_value(document, pair.first())
                else {
                    continue;
                };
                let Some(value) = pair.get(1) else {
                    continue;
                };
                tree.entries = tree.entries.saturating_add(1);
                collect_parent_tree_value(document, *parent_key, value, owners, &mut tree);
            }
        }
    }
    tree
}

fn collect_parent_tree_value(
    document: &ParsedDocument,
    parent_key: i64,
    value: &CosObject,
    owners: &BTreeMap<i64, ParentTreeOwner>,
    tree: &mut NumberTree,
) {
    let resolved = resolved_value(document, Some(value));
    if let Some(CosObject::Array(values)) = resolved {
        let entries = object_ref(value).map_or_else(
            || McidArray::Direct(values.iter().map(object_ref).collect()),
            McidArray::Indirect,
        );
        tree.mcid_arrays.insert(parent_key, entries);
    } else if let (Some(element), Some(CosObject::Dictionary(_))) = (object_ref(value), resolved) {
        tree.structure_refs.insert(parent_key, element);
        if let Some(owner) = owners.get(&parent_key)
            && owner.kind != ParentTreeOwnerKind::Page
        {
            tree.object_refs.insert(owner.object, element);
        }
    }
}

fn collect_parent_tree_owners(
    document: &ParsedDocument,
    pages: &[PageContent<'_>],
) -> BTreeMap<i64, ParentTreeOwner> {
    let mut owners = BTreeMap::new();
    for page in pages {
        if let Some(parent) = integer_value(page.dictionary.get("StructParents")) {
            owners.insert(
                parent,
                ParentTreeOwner {
                    object: page.key,
                    page_ordinal: Some(page.ordinal),
                    kind: ParentTreeOwnerKind::Page,
                },
            );
        }
        for annotation in page
            .dictionary
            .get("Annots")
            .into_iter()
            .flat_map(annotation_refs)
        {
            insert_parent_tree_owner(
                document,
                &mut owners,
                annotation,
                Some(page.ordinal),
                ParentTreeOwnerKind::Annotation,
            );
        }
        for xobject in page_xobject_refs(document, page.dictionary) {
            let kind = document
                .objects
                .get(&xobject)
                .and_then(|object| object.object.as_dictionary())
                .map_or(ParentTreeOwnerKind::Other, xobject_owner_kind);
            insert_parent_tree_owner(document, &mut owners, xobject, Some(page.ordinal), kind);
        }
    }
    owners
}

fn insert_parent_tree_owner(
    document: &ParsedDocument,
    owners: &mut BTreeMap<i64, ParentTreeOwner>,
    object: ObjectKey,
    page_ordinal: Option<usize>,
    kind: ParentTreeOwnerKind,
) {
    let Some(dictionary) = document
        .objects
        .get(&object)
        .and_then(|object| object.object.as_dictionary())
    else {
        return;
    };
    let parent = integer_value(dictionary.get("StructParent"))
        .or_else(|| integer_value(dictionary.get("StructParents")));
    if let Some(parent) = parent {
        owners.insert(
            parent,
            ParentTreeOwner {
                object,
                page_ordinal,
                kind,
            },
        );
    }
}

fn collect_parent_tree_object_associations(
    document: &ParsedDocument,
    parent_tree: &NumberTree,
    owners: &BTreeMap<i64, ParentTreeOwner>,
    graph: &mut AccessibilityGraph,
) {
    let owners_by_object = owners
        .values()
        .map(|owner| (owner.object, *owner))
        .collect::<BTreeMap<_, _>>();
    for (object, element) in &parent_tree.object_refs {
        let Some(node) = graph.nodes_by_key.get(element).copied() else {
            graph.warnings.push(warning(
                "parent tree object reference targets unknown structure element",
            ));
            continue;
        };
        let owner = owners_by_object.get(object).copied();
        let dictionary = document
            .objects
            .get(object)
            .and_then(|object| object.object.as_dictionary());
        let kind = owner
            .map(|owner| owner.kind)
            .or_else(|| dictionary.map(xobject_owner_kind))
            .unwrap_or(ParentTreeOwnerKind::Other);
        if kind == ParentTreeOwnerKind::Image {
            let role = graph.node(node).map_or_else(
                || String::from("Figure"),
                |node| node.normalized_role.clone(),
            );
            graph.marked_content.push(AccessibilityMarkedContent {
                node: Some(node),
                page_ordinal: owner.and_then(|owner| owner.page_ordinal).unwrap_or(0),
                stream: *object,
                tag: role,
                mcid: None,
                has_text: false,
                has_image: true,
                location: object_location(document, *object, "root/accessibility/imageObject"),
            });
        } else {
            graph.object_references.push(AccessibilityObjectReference {
                node,
                object: Some(*object),
                page_ordinal: owner.and_then(|owner| owner.page_ordinal),
                is_link_annotation: dictionary.is_some_and(is_link_annotation),
            });
        }
    }
}

fn annotation_refs(value: &CosObject) -> Vec<ObjectKey> {
    match value {
        CosObject::Reference(key) => vec![*key],
        CosObject::Array(values) => values.iter().filter_map(object_ref).collect(),
        _ => Vec::new(),
    }
}

fn page_xobject_refs(document: &ParsedDocument, page: &Dictionary) -> Vec<ObjectKey> {
    let Some(resources) = dictionary_from_value(document, page.get("Resources")) else {
        return Vec::new();
    };
    let Some(CosObject::Dictionary(xobjects)) = resources.get("XObject") else {
        return Vec::new();
    };
    xobjects
        .iter()
        .filter_map(|(_name, value)| object_ref(value))
        .collect()
}

fn xobject_owner_kind(dictionary: &Dictionary) -> ParentTreeOwnerKind {
    match dictionary.get("Subtype").and_then(name_value).as_deref() {
        Some("Image") => ParentTreeOwnerKind::Image,
        Some("Form") => ParentTreeOwnerKind::Form,
        _ => ParentTreeOwnerKind::Other,
    }
}

fn collect_id_tree(
    document: &ParsedDocument,
    value: &CosObject,
    limits: &ResourceLimits,
    warnings: &mut Vec<ValidationWarning>,
) -> BTreeMap<String, ObjectKey> {
    let mut mappings = BTreeMap::new();
    let mut stack = Vec::from([value]);
    let mut visited = HashSet::new();
    while let Some(value) = stack.pop() {
        let Some((key, _offset, dictionary)) = dictionary_ref_or_inline(document, value) else {
            continue;
        };
        if let Some(key) = key
            && !visited.insert(key)
        {
            warnings.push(warning("ID tree cycle detected"));
            continue;
        }
        if let Some(CosObject::Array(kids)) = dictionary.get("Kids") {
            for kid in kids.iter().rev() {
                stack.push(kid);
            }
        }
        if let Some(CosObject::Array(names)) = dictionary.get("Names") {
            for pair in names.chunks(2) {
                if u64::try_from(mappings.len()).unwrap_or(u64::MAX)
                    >= limits.max_parent_tree_entries
                {
                    warnings.push(warning("ID tree entry cap reached"));
                    return mappings;
                }
                let Some(id) = pair.first().and_then(|value| text_string(Some(value))) else {
                    warnings.push(warning("ID tree entry with non-string key skipped"));
                    continue;
                };
                let Some(element) = pair.get(1).and_then(object_ref) else {
                    warnings.push(warning("ID tree entry with non-reference value skipped"));
                    continue;
                };
                if mappings.insert(id, element).is_some() {
                    warnings.push(warning("duplicate ID tree entry replaced"));
                }
            }
        }
    }
    mappings
}

fn class_map_entry_count(
    struct_tree: &Dictionary,
    limits: &ResourceLimits,
    warnings: &mut Vec<ValidationWarning>,
) -> Result<u64> {
    let Some(CosObject::Dictionary(class_map)) = struct_tree.get("ClassMap") else {
        return Ok(0);
    };
    let entries = u64::try_from(class_map.len()).map_err(|_| ValidationError::LimitExceeded {
        limit: "max_parent_tree_entries",
    })?;
    if entries > limits.max_parent_tree_entries {
        warnings.push(warning("ClassMap entry cap reached"));
        return Ok(limits.max_parent_tree_entries);
    }
    Ok(entries)
}

fn element_id(dictionary: &Dictionary) -> Option<String> {
    text_string(dictionary.get("ID"))
}

fn class_references_resolve(struct_tree: &Dictionary, dictionary: &Dictionary) -> bool {
    let Some(class_ref) = dictionary.get("C") else {
        return false;
    };
    let Some(CosObject::Dictionary(class_map)) = struct_tree.get("ClassMap") else {
        return false;
    };
    match class_ref {
        CosObject::Array(values) => {
            let mut saw_class = false;
            for value in values {
                let Some(class_name) = class_name_from_value(value) else {
                    return false;
                };
                saw_class = true;
                if class_map.get(class_name.as_str()).is_none() {
                    return false;
                }
            }
            saw_class
        }
        value => class_name_from_value(value)
            .and_then(|class_name| class_map.get(class_name.as_str()))
            .is_some(),
    }
}

fn attribute_text(
    document: &ParsedDocument,
    dictionary: &Dictionary,
    name: &str,
) -> Option<String> {
    attribute_values(document, dictionary.get("A"))
        .into_iter()
        .find_map(|attribute| {
            attribute
                .get(name)
                .and_then(|value| name_value(value).or_else(|| text_string(Some(value))))
        })
}

fn attribute_texts(document: &ParsedDocument, dictionary: &Dictionary, name: &str) -> Vec<String> {
    attribute_values(document, dictionary.get("A"))
        .into_iter()
        .flat_map(|attribute| strings_from_value(attribute.get(name)))
        .collect()
}

fn attribute_positive_u32(
    document: &ParsedDocument,
    dictionary: &Dictionary,
    name: &str,
) -> Option<u32> {
    attribute_values(document, dictionary.get("A"))
        .into_iter()
        .find_map(|attribute| {
            attribute.get(name).and_then(|value| {
                integer_value(Some(value))
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0)
            })
        })
}

fn namespace_url(document: &ParsedDocument, dictionary: &Dictionary) -> Option<String> {
    match dictionary.get("NS")? {
        CosObject::Name(_) | CosObject::String(_) => string_from_value(dictionary.get("NS")?),
        value => dictionary_from_value(document, Some(value))
            .and_then(|namespace| namespace.get("NS"))
            .and_then(string_from_value),
    }
}

fn attribute_values<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
) -> Vec<&'a Dictionary> {
    let mut values = Vec::new();
    push_attribute_values(document, value, &mut values);
    values
}

fn push_attribute_values<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
    values: &mut Vec<&'a Dictionary>,
) {
    match value {
        Some(CosObject::Array(items)) => {
            for item in items {
                push_attribute_values(document, Some(item), values);
            }
        }
        Some(value) => {
            if let Some(dictionary) = dictionary_from_value(document, Some(value)) {
                values.push(dictionary);
            }
        }
        None => {}
    }
}

fn class_name_from_value(value: &CosObject) -> Option<String> {
    match value {
        CosObject::Name(name) => Some(name_to_string(name)),
        CosObject::String(_) => text_string(Some(value)),
        _ => None,
    }
}

fn strings_from_value(value: Option<&CosObject>) -> Vec<String> {
    let mut values = Vec::new();
    push_strings_from_value(value, &mut values);
    values
}

fn push_strings_from_value(value: Option<&CosObject>, values: &mut Vec<String>) {
    match value {
        Some(CosObject::Array(items)) => {
            for item in items {
                push_strings_from_value(Some(item), values);
            }
        }
        Some(value) => {
            if let Some(text) = string_from_value(value) {
                values.push(text);
            }
        }
        None => {}
    }
}

fn string_from_value(value: &CosObject) -> Option<String> {
    name_value(value).or_else(|| text_string(Some(value)))
}

fn collect_pages<'a>(
    document: &'a ParsedDocument,
    limits: &ResourceLimits,
) -> Result<Vec<PageContent<'a>>> {
    let mut pages = Vec::new();
    let Some(catalog) = catalog_dictionary(document) else {
        return Ok(pages);
    };
    let Some(root) = catalog.get("Pages").and_then(object_ref) else {
        return Ok(pages);
    };
    let mut stack = Vec::from([root]);
    let mut visited = HashSet::new();
    while let Some(key) = stack.pop() {
        if !visited.insert(key) {
            continue;
        }
        if u64::try_from(visited.len()).unwrap_or(u64::MAX) > limits.max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let Some(dictionary) = document
            .objects
            .get(&key)
            .and_then(|object| object.object.as_dictionary())
        else {
            continue;
        };
        if dictionary
            .get("Type")
            .and_then(name_value)
            .is_some_and(|name| name == "Page")
        {
            pages.push(PageContent {
                key,
                ordinal: pages.len(),
                dictionary,
            });
            continue;
        }
        if let Some(CosObject::Array(kids)) = dictionary.get("Kids") {
            for kid in kids.iter().rev().filter_map(object_ref) {
                stack.push(kid);
            }
        }
    }
    Ok(pages)
}

fn content_streams<'a>(
    document: &'a ParsedDocument,
    page: &'a Dictionary,
) -> Vec<(ObjectKey, &'a crate::StreamObject)> {
    let mut streams = Vec::new();
    if let Some(contents) = page.get("Contents") {
        push_content_stream_value(document, contents, &mut streams);
    }
    streams
}

fn push_content_stream_value<'a>(
    document: &'a ParsedDocument,
    value: &'a CosObject,
    streams: &mut Vec<(ObjectKey, &'a crate::StreamObject)>,
) {
    match value {
        CosObject::Reference(key) => {
            if let Some(CosObject::Stream(stream)) =
                document.objects.get(key).map(|object| &object.object)
            {
                streams.push((*key, stream));
            }
        }
        CosObject::Array(values) => {
            for value in values {
                push_content_stream_value(document, value, streams);
            }
        }
        CosObject::Stream(stream) => streams.push((
            ObjectKey {
                number: NonZeroU32::MIN,
                generation: 0,
            },
            stream,
        )),
        _ => {}
    }
}

fn page_properties<'a>(
    document: &'a ParsedDocument,
    page: &'a Dictionary,
) -> BTreeMap<PdfName, &'a CosObject> {
    let mut properties = BTreeMap::new();
    let Some(resources) = dictionary_from_value(document, page.get("Resources")) else {
        return properties;
    };
    let Some(CosObject::Dictionary(property_dict)) = resources.get("Properties") else {
        return properties;
    };
    for (name, value) in property_dict.iter() {
        properties.insert(name.clone(), value);
    }
    properties
}

fn mcid_from_object(document: &ParsedDocument, key: ObjectKey) -> Option<i64> {
    document
        .objects
        .get(&key)
        .and_then(|object| object.object.as_dictionary())
        .and_then(|dictionary| integer_value(dictionary.get("MCID")))
}

fn mcid_from_value(document: &ParsedDocument, value: &CosObject) -> Option<i64> {
    dictionary_from_value(document, Some(value))
        .and_then(|dictionary| integer_value(dictionary.get("MCID")))
}

#[derive(Debug)]
struct ResolvedRole {
    name: String,
    standard_namespace_url: Option<String>,
    remapped_standard_role: Option<String>,
    same_namespace_remapping: Option<String>,
    is_standard: bool,
    circular: bool,
}

fn resolve_role(
    document: &ParsedDocument,
    tree: &Dictionary,
    element: &Dictionary,
    source: &str,
) -> ResolvedRole {
    let original_namespace =
        namespace_url(document, element).unwrap_or_else(|| PDF_17_NAMESPACE.to_owned());
    let mut namespace = dictionary_from_value(document, element.get("NS"));
    let mut uri = original_namespace.clone();
    let mut name = source.to_owned();
    let mut seen = BTreeSet::new();
    let mut same_namespace_remapping = None;
    let mut first_standard_target = None;
    let mut circular = false;
    for _ in 0..MAX_ROLE_MAP_STEPS {
        if !seen.insert((uri.clone(), name.clone())) {
            circular = true;
            break;
        }
        let role_map_ns = namespace
            .and_then(|dictionary| dictionary_from_value(document, dictionary.get("RoleMapNS")));
        let map = role_map_ns.or_else(|| {
            (uri == PDF_17_NAMESPACE)
                .then(|| dictionary_from_value(document, tree.get("RoleMap")))
                .flatten()
        });
        let Some(value) = map.and_then(|map| resolved_value(document, map.get(&name))) else {
            break;
        };
        let (target, target_namespace) = match value {
            CosObject::Name(target) => (target, None),
            CosObject::Array(items) => {
                let (Some(CosObject::Name(target)), Some(namespace)) = (
                    resolved_value(document, items.first()),
                    dictionary_from_value(document, items.get(1)),
                ) else {
                    break;
                };
                (target, Some(namespace))
            }
            _ => break,
        };
        let target_uri = target_namespace
            .and_then(|namespace| text_string(resolved_value(document, namespace.get("NS"))))
            .unwrap_or_else(|| PDF_17_NAMESPACE.to_owned());
        let target_name = name_to_string(target);
        if is_standard_role(&target_name, &target_uri) {
            first_standard_target.get_or_insert_with(|| (target_name.clone(), target_uri.clone()));
        }
        if role_map_ns.is_some() && target_name != name {
            if target_uri == uri {
                same_namespace_remapping.get_or_insert_with(|| format!("{uri}:{name}"));
            } else if let Some((_, earlier)) = seen
                .iter()
                .find(|(earlier_uri, _)| *earlier_uri == target_uri)
            {
                same_namespace_remapping.get_or_insert_with(|| format!("{target_uri}:{earlier}"));
            }
        }
        namespace = target_namespace;
        uri = target_uri;
        name = target_name;
    }
    let is_standard = if let Some((standard_name, standard_namespace)) = first_standard_target {
        name = standard_name;
        uri = standard_namespace;
        true
    } else {
        !circular && is_standard_role(&name, &uri)
    };
    let remapped = is_standard_role(source, &original_namespace)
        && (!is_standard || name != source || uri != original_namespace);
    ResolvedRole {
        name,
        standard_namespace_url: is_standard.then_some(uri),
        remapped_standard_role: remapped.then(|| source.to_owned()),
        same_namespace_remapping,
        is_standard,
        circular,
    }
}

fn structure_dictionary<'a>(
    document: &'a ParsedDocument,
    value: &'a CosObject,
) -> Option<(Option<ObjectKey>, Option<u64>, &'a Dictionary)> {
    let (key, offset, dictionary) = dictionary_ref_or_inline(document, value)?;
    if role_name(document, dictionary).is_some() || is_mcr(dictionary) || is_objr(dictionary) {
        return Some((key, offset, dictionary));
    }
    None
}

fn dictionary_ref_or_inline<'a>(
    document: &'a ParsedDocument,
    value: &'a CosObject,
) -> Option<(Option<ObjectKey>, Option<u64>, &'a Dictionary)> {
    match value {
        CosObject::Reference(key) => {
            let object = document.objects.get(key)?;
            Some((
                Some(*key),
                Some(object.offset),
                object.object.as_dictionary()?,
            ))
        }
        CosObject::Dictionary(dictionary) => Some((None, None, dictionary)),
        CosObject::Stream(stream) => Some((None, None, &stream.dictionary)),
        _ => None,
    }
}

fn dictionary_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
) -> Option<&'a Dictionary> {
    match value? {
        CosObject::Reference(key) => document.objects.get(key)?.object.as_dictionary(),
        CosObject::Dictionary(dictionary) => Some(dictionary),
        CosObject::Stream(stream) => Some(&stream.dictionary),
        _ => None,
    }
}

fn resolved_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
) -> Option<&'a CosObject> {
    match value? {
        CosObject::Reference(key) => document.objects.get(key).map(|object| &object.object),
        value => Some(value),
    }
}

fn catalog_dictionary(document: &ParsedDocument) -> Option<&Dictionary> {
    document
        .catalog
        .and_then(|key| document.objects.get(&key))
        .and_then(|object| object.object.as_dictionary())
}

fn catalog_marked(document: &ParsedDocument, catalog: &Dictionary) -> bool {
    dictionary_from_value(document, catalog.get("MarkInfo"))
        .and_then(|mark_info| mark_info.get("Marked"))
        .is_some_and(|value| matches!(value, CosObject::Boolean(true)))
}

fn role_name(document: &ParsedDocument, dictionary: &Dictionary) -> Option<String> {
    name_value(resolved_value(document, dictionary.get("S"))?)
}

fn is_mcr(dictionary: &Dictionary) -> bool {
    dictionary
        .get("Type")
        .and_then(name_value)
        .is_some_and(|name| name == "MCR")
        || dictionary.get("MCID").is_some()
}

fn is_objr(dictionary: &Dictionary) -> bool {
    dictionary
        .get("Type")
        .and_then(name_value)
        .is_some_and(|name| name == "OBJR")
        || dictionary.get("Obj").is_some()
}

fn is_link_annotation(dictionary: &Dictionary) -> bool {
    dictionary
        .get("Subtype")
        .and_then(name_value)
        .is_some_and(|name| name == "Link")
}

fn object_ref(value: &CosObject) -> Option<ObjectKey> {
    match value {
        CosObject::Reference(key) => Some(*key),
        _ => None,
    }
}

fn integer_value(value: Option<&CosObject>) -> Option<i64> {
    match value? {
        CosObject::Integer(value) => Some(*value),
        _ => None,
    }
}

fn name_value(value: &CosObject) -> Option<String> {
    match value {
        CosObject::Name(name) => Some(name_to_string(name)),
        _ => None,
    }
}

fn text_string(value: Option<&CosObject>) -> Option<String> {
    match value? {
        CosObject::String(value) => Some(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        _ => None,
    }
}

fn text_bytes(value: Option<&CosObject>) -> u64 {
    match value {
        Some(CosObject::String(value)) => u64::try_from(value.as_bytes().len()).unwrap_or(u64::MAX),
        _ => 0,
    }
}

fn is_standard_role(role: &str, namespace: &str) -> bool {
    match namespace {
        MATH_NAMESPACE => true,
        PDF_17_NAMESPACE => {
            STANDARD_ROLES.contains(&role)
                && !matches!(
                    role,
                    "DocumentFragment"
                        | "Aside"
                        | "Title"
                        | "FENote"
                        | "Sub"
                        | "Em"
                        | "Strong"
                        | "Artifact"
                )
        }
        PDF_20_NAMESPACE => {
            (STANDARD_ROLES.contains(&role)
                && !matches!(
                    role,
                    "Art"
                        | "BlockQuote"
                        | "TOC"
                        | "TOCI"
                        | "Index"
                        | "Private"
                        | "Quote"
                        | "Note"
                        | "Reference"
                        | "BibEntry"
                        | "Code"
                ))
                || role.strip_prefix('H').is_some_and(|suffix| {
                    suffix
                        .as_bytes()
                        .first()
                        .is_some_and(|byte| matches!(byte, b'1'..=b'9'))
                        && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
        }
        _ => false,
    }
}

fn name_to_string(name: &PdfName) -> String {
    String::from_utf8_lossy(name.as_bytes()).into_owned()
}

fn warning(message: &str) -> ValidationWarning {
    ValidationWarning::General {
        message: BoundedText::new(message, 512)
            .unwrap_or_else(|_| BoundedText::unchecked("accessibility warning exceeded cap")),
    }
}

fn object_location(document: &ParsedDocument, object: ObjectKey, context: &str) -> ObjectLocation {
    ObjectLocation {
        object: Some(object),
        offset: document.objects.get(&object).map(|object| object.offset),
        path: Some(BoundedText::unchecked(format!(
            "{context}[{}]",
            object.number
        ))),
    }
}

impl AccessibilityGraph {
    /// Returns a structure node by id.
    #[must_use]
    pub(crate) fn node(&self, id: AccessibilityNodeId) -> Option<&AccessibilityNode> {
        self.nodes.get(id.0)
    }
}
