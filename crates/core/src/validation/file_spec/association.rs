//! PDF/A-3 associated-file evidence from owners and marked content.

use std::{
    cell::{RefCell, RefMut},
    collections::HashSet,
    ptr,
};

use super::super::{
    ContentStreamModel, ModelGraph, ModelObjectRef, PageModel, catalog_dictionary,
    effective_page_resources, resolve_dictionary_value, semantic_names,
};
use crate::{
    CosObject, Dictionary, ObjectKey, Result, ValidationError, accessibility::is_structure_element,
    content::ContentStreamSummary,
};

#[derive(Debug, Default)]
pub(in crate::validation) struct Associations {
    files: HashSet<ObjectKey>,
    visited: HashSet<NodeId>,
    arrays: HashSet<usize>,
    inspected: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct NodeId {
    role: NodeRole,
    address: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum NodeRole {
    StructureKids,
    Structure,
    Resources,
    XObject,
    Font,
    Pattern,
    GraphicsState,
}

#[derive(Clone, Copy, Debug)]
struct Node<'a> {
    role: NodeRole,
    value: &'a CosObject,
}

fn scan<'scope>(graph: &'scope ModelGraph<'_>) -> Result<Option<RefMut<'scope, Associations>>> {
    graph
        .associated_files
        .as_ref()
        .map(RefCell::try_borrow_mut)
        .transpose()
        .map_err(|_| {
            ValidationError::SubsystemUnavailable {
                subsystem: "associated_files",
            }
            .into()
        })
}

impl Associations {
    fn charge(&mut self, limit: u64) -> Result<()> {
        if self.inspected >= limit {
            return Err(ValidationError::LimitExceeded {
                limit: "associated_file_sites",
            }
            .into());
        }
        self.inspected = self.inspected.saturating_add(1);
        Ok(())
    }

    fn register(&mut self, graph: &ModelGraph<'_>, value: Option<&CosObject>) -> Result<()> {
        if let Some((_, array @ CosObject::Array(files))) =
            super::resolve_reference(graph.document, value, graph.limits.max_object_depth)?
            && self.arrays.insert(ptr::from_ref(array).addr())
        {
            self.charge(graph.limits.max_objects)?;
            for file in files {
                self.register_member(graph, file)?;
            }
        }
        Ok(())
    }

    fn register_member<'a>(
        &mut self,
        graph: &ModelGraph<'a>,
        mut file: &'a CosObject,
    ) -> Result<()> {
        for _ in 0..=graph.limits.max_object_depth {
            self.charge(graph.limits.max_objects)?;
            let CosObject::Reference(key) = file else {
                break;
            };
            match semantic_names::resolve(graph.document, Some(file)) {
                Some(CosObject::Dictionary(_)) => {
                    self.files.insert(*key);
                    break;
                }
                Some(alias @ CosObject::Reference(_)) => file = alias,
                _ => break,
            }
        }
        Ok(())
    }

    fn walk<'a>(&mut self, graph: &ModelGraph<'a>, mut pending: Vec<Node<'a>>) -> Result<()> {
        while let Some(node) = pending.pop() {
            self.charge(graph.limits.max_objects)?;
            let Some(value) = semantic_names::resolve(graph.document, Some(node.value)) else {
                continue;
            };
            if !self.visited.insert(NodeId {
                role: node.role,
                address: ptr::from_ref(value).addr(),
            }) {
                continue;
            }
            self.expand(graph, node.role, value, &mut pending)?;
            if u64::try_from(pending.len()).unwrap_or(u64::MAX)
                > graph.limits.max_objects.saturating_sub(self.inspected)
            {
                return Err(ValidationError::LimitExceeded {
                    limit: "associated_file_sites",
                }
                .into());
            }
        }
        Ok(())
    }

    fn expand<'a>(
        &mut self,
        graph: &ModelGraph<'a>,
        role: NodeRole,
        value: &'a CosObject,
        pending: &mut Vec<Node<'a>>,
    ) -> Result<()> {
        use NodeRole::{
            Font, GraphicsState, Pattern, Resources, Structure, StructureKids, XObject,
        };
        let document = graph.document;
        if let (StructureKids, CosObject::Array(children)) = (role, value) {
            if u64::try_from(pending.len().saturating_add(children.len())).unwrap_or(u64::MAX)
                > graph.limits.max_objects.saturating_sub(self.inspected)
            {
                return Err(ValidationError::LimitExceeded {
                    limit: "associated_file_sites",
                }
                .into());
            }
            pending.extend(children.iter().map(|value| Node {
                role: Structure,
                value,
            }));
            return Ok(());
        }
        let Some(dictionary) = value.as_dictionary() else {
            return Ok(());
        };
        match role {
            Structure | StructureKids
                if matches!(value, CosObject::Dictionary(_))
                    && is_structure_element(document, dictionary) =>
            {
                self.register(graph, dictionary.get("AF"))?;
                push(pending, StructureKids, dictionary.get("K"));
            }
            Resources => {
                self.resource_entries(graph, dictionary, pending)?;
            }
            XObject => {
                if let Some(subtype) = semantic_names::name(document, dictionary.get("Subtype"))
                    && (subtype.matches("Form")
                        || subtype.matches("Image")
                        || subtype.matches("PS"))
                {
                    self.register(graph, dictionary.get("AF"))?;
                    if subtype.matches("Form") {
                        push(pending, Resources, dictionary.get("Resources"));
                    }
                }
            }
            Font if semantic_names::name(document, dictionary.get("Subtype"))
                .is_some_and(|subtype| subtype.matches("Type3")) =>
            {
                push(pending, Resources, dictionary.get("Resources"));
            }
            GraphicsState => {
                if let Some(CosObject::Array(font)) =
                    semantic_names::resolve(document, dictionary.get("Font"))
                {
                    push(pending, Font, font.first());
                }
            }
            Pattern => match semantic_names::resolve(document, dictionary.get("PatternType")) {
                Some(CosObject::Integer(1)) => {
                    push(pending, Resources, dictionary.get("Resources"));
                }
                Some(CosObject::Integer(2)) => {
                    push(pending, GraphicsState, dictionary.get("ExtGState"));
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn resource_entries<'a>(
        &self,
        graph: &ModelGraph<'a>,
        resources: &'a Dictionary,
        pending: &mut Vec<Node<'a>>,
    ) -> Result<()> {
        use NodeRole::{Font, GraphicsState, Pattern, XObject};
        let remaining = graph.limits.max_objects.saturating_sub(self.inspected);
        for (field, role) in [
            ("XObject", XObject),
            ("Font", Font),
            ("Pattern", Pattern),
            ("ExtGState", GraphicsState),
        ] {
            if let Some(entries) = resolve_dictionary_value(graph.document, resources.get(field)) {
                if u64::try_from(pending.len().saturating_add(entries.len())).unwrap_or(u64::MAX)
                    > remaining
                {
                    return Err(ValidationError::LimitExceeded {
                        limit: "associated_file_sites",
                    }
                    .into());
                }
                pending.extend(entries.iter().map(|(_, value)| Node { role, value }));
            }
        }
        Ok(())
    }

    fn appearance<'a>(&mut self, graph: &ModelGraph<'a>, value: &'a CosObject) -> Result<()> {
        if let Some(CosObject::Stream(stream)) =
            semantic_names::resolve(graph.document, Some(value))
        {
            self.charge(graph.limits.max_objects)?;
            self.register(graph, stream.dictionary.get("AF"))?;
            if let Some(value) = stream.dictionary.get("Resources") {
                self.walk(
                    graph,
                    vec![Node {
                        role: NodeRole::Resources,
                        value,
                    }],
                )?;
            }
        }
        Ok(())
    }

    fn register_marks(
        &mut self,
        graph: &ModelGraph<'_>,
        content: &ContentStreamModel<'_>,
        summary: &ContentStreamSummary,
    ) -> Result<()> {
        if summary.truncated || summary.scoped_operands_truncated {
            return Err(ValidationError::LimitExceeded {
                limit: "associated_file_operands",
            }
            .into());
        }
        let mut names = summary
            .marked_content
            .iter()
            .filter_map(|mark| mark.associated_file_property.as_ref())
            .peekable();
        if names.peek().is_none() {
            return Ok(());
        }
        let resources = content.effective_resources(graph.document)?;
        let properties: Vec<_> = resources
            .dictionaries
            .iter()
            .rev()
            .filter_map(|dictionary| {
                resolve_dictionary_value(graph.document, dictionary.get("Properties"))
            })
            .collect();
        let mut seen = HashSet::new();
        for name in names {
            if seen.insert(name) {
                self.register(
                    graph,
                    properties.iter().find_map(|dictionary| {
                        dictionary
                            .get_name(name)
                            .filter(|value| !matches!(value, CosObject::Null))
                    }),
                )?;
            }
        }
        Ok(())
    }
}

fn push<'a>(pending: &mut Vec<Node<'a>>, role: NodeRole, value: Option<&'a CosObject>) {
    pending.extend(value.map(|value| Node { role, value }));
}

pub(in crate::validation) fn collect_catalog(graph: &ModelGraph<'_>) -> Result<()> {
    let Some(mut sites) = scan(graph)? else {
        return Ok(());
    };
    if let Some(catalog) = catalog_dictionary(graph.document) {
        sites.register(graph, catalog.get("AF"))?;
        if let Some(structure) =
            resolve_dictionary_value(graph.document, catalog.get("StructTreeRoot"))
            && let Some(value) = structure.get("K")
        {
            sites.walk(
                graph,
                vec![Node {
                    role: NodeRole::StructureKids,
                    value,
                }],
            )?;
        }
    }
    Ok(())
}

pub(in crate::validation) fn collect_page<'a>(
    graph: &ModelGraph<'a>,
    page: &PageModel<'a>,
) -> Result<()> {
    let Some(mut sites) = scan(graph)? else {
        return Ok(());
    };
    sites.register(graph, page.dictionary.get("AF"))?;
    let resources = effective_page_resources(graph.document, page)?;
    if let Some(dictionary) = resources.dictionaries.last() {
        let mut pending = Vec::new();
        sites.resource_entries(graph, dictionary, &mut pending)?;
        sites.walk(graph, pending)?;
    }
    if let Some(CosObject::Array(annotations)) =
        semantic_names::resolve(graph.document, page.dictionary.get("Annots"))
    {
        for annotation in annotations {
            sites.charge(graph.limits.max_objects)?;
            if let Some(dictionary) = resolve_dictionary_value(graph.document, Some(annotation)) {
                sites.register(graph, dictionary.get("AF"))?;
                collect_appearances(&mut sites, graph, dictionary)?;
            }
        }
    }
    Ok(())
}

fn collect_appearances<'a>(
    sites: &mut Associations,
    graph: &ModelGraph<'a>,
    annotation: &'a Dictionary,
) -> Result<()> {
    let Some(appearances) = resolve_dictionary_value(graph.document, annotation.get("AP")) else {
        return Ok(());
    };
    for kind in ["N", "R", "D"] {
        let Some(value) = appearances.get(kind) else {
            continue;
        };
        if let Some(CosObject::Dictionary(states)) =
            semantic_names::resolve(graph.document, Some(value))
        {
            for (_, state) in states.iter() {
                sites.charge(graph.limits.max_objects)?;
                sites.appearance(graph, state)?;
            }
        } else {
            sites.appearance(graph, value)?;
        }
    }
    Ok(())
}

pub(in crate::validation) fn collect_content(
    graph: &ModelGraph<'_>,
    content: &ContentStreamModel<'_>,
    summary: &ContentStreamSummary,
) -> Result<()> {
    let Some(mut sites) = scan(graph)? else {
        return Ok(());
    };
    if content.page_key.is_some() {
        sites.register_marks(graph, content, summary)?;
    }
    Ok(())
}

pub(in crate::validation) fn finalize(
    graph: &ModelGraph<'_>,
    object: &mut ModelObjectRef<'_>,
) -> Result<()> {
    let ModelObjectRef::Generic(model) = object else {
        return Ok(());
    };
    if let Some(evidence) = model.file_spec.as_mut()
        && let Some(key) = model.key
        && let Some(sites) = scan(graph)?
    {
        evidence.associated = sites.files.contains(&key)
            && graph
                .document
                .objects
                .get(&key)
                .and_then(|object| object.object.as_dictionary())
                .is_some_and(|dictionary| ptr::eq(dictionary, evidence.dictionary));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, Result, ValidationFlavour,
        ValidationOptions, Validator,
    };

    const PATTERN_RESOURCES: &str = "/Resources << /Pattern << /T 6 0 R >> >>";
    const PATTERN: &str = "6 0 obj\n<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
                           /BBox [0 0 10 10] /XStep 10 /YStep 10 /Length 14 /Resources << \
                           /Properties << /P [5 0 R] >> >> >>\nstream\n/AF /P BDC \
                           EMC\nendstream\nendobj\n";

    #[rstest]
    #[case::catalog("/AF [5 0 R]", "", "", "", true)]
    #[case::nested_array("/AF [[5 0 R]]", "", "", "", false)]
    #[case::wrong_generation("/AF [5 1 R]", "", "", "", false)]
    #[case::alias("/AF [6 0 R]", "", "", "6 0 obj\n5 0 R\nendobj\n", true)]
    #[case::name_tree_only("", "", "", "", false)]
    #[case::page("", "/AF [5 0 R]", "", "", true)]
    #[case::roleless_structure(
        "/StructTreeRoot << /K 6 0 R >>",
        "",
        "",
        "6 0 obj\n<< /Type /StructElem /AF [5 0 R] >>\nendobj\n",
        true
    )]
    #[case::structure_root("/StructTreeRoot << /AF [5 0 R] >>", "", "", "", false)]
    #[case::nested_structure_array(
        "/StructTreeRoot << /K [[6 0 R]] >>",
        "",
        "",
        "6 0 obj\n<< /Type /StructElem /AF [5 0 R] >>\nendobj\n",
        false
    )]
    #[case::annotation(
        "",
        "/Annots [6 0 R]",
        "",
        "6 0 obj\n<< /Type /Annot /Subtype /Stamp /AF [5 0 R] >>\nendobj\n",
        true
    )]
    #[case::unused_form(
        "",
        "/Resources << /XObject << /Form 6 0 R >> >>",
        "",
        "6 0 obj\n<< /Subtype /Form /AF [5 0 R] /Length 0 >>\nstream\nendstream\nendobj\n",
        true
    )]
    #[case::unreachable_form(
        "",
        "",
        "",
        "6 0 obj\n<< /Subtype /Form /AF [5 0 R] /Length 0 >>\nstream\nendstream\nendobj\n",
        false
    )]
    #[case::bdc(
        "",
        "/Resources << /Properties << /P [5 0 R] >> >>",
        "/AF /P BDC EMC",
        "",
        true
    )]
    #[case::dp(
        "",
        "/Resources << /Properties << /P [5 0 R] >> >>",
        "/AF /P DP",
        "",
        false
    )]
    #[case::inline_bdc("", "", "/P << /AF [5 0 R] >> BDC EMC", "", false)]
    #[case::wrong_tag(
        "",
        "/Resources << /Properties << /P [5 0 R] >> >>",
        "/Span /P BDC EMC",
        "",
        false
    )]
    #[case::pattern("", PATTERN_RESOURCES, "/Pattern cs /T scn", PATTERN, true)]
    #[case::wrong_pattern_space("", PATTERN_RESOURCES, "/Pattern CS /T scn", PATTERN, false)]
    #[case::reset_pattern("", PATTERN_RESOURCES, "/Pattern cs 0 g /T scn", PATTERN, false)]
    fn test_should_require_reachable_association_for_embedded_files(
        #[case] catalog: &str,
        #[case] page: &str,
        #[case] content: &str,
        #[case] objects: &str,
        #[case] associated: bool,
    ) -> Result<()> {
        for conformance in ["a", "b", "u"] {
            let part = NonZeroU32::new(3).ok_or(ProfileError::UnsupportedSelection)?;
            let flavour = ValidationFlavour::new("pdfa", part, conformance)?;
            let options = ValidationOptions::builder()
                .flavour(FlavourSelection::Explicit { flavour })
                .record_passed_assertions(true)
                .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
                .build();
            let pdf = format!(
                "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R {catalog} /Names << \
                 /EmbeddedFiles << /Names [(file) 5 0 R] >> >> >>\nendobj\n2 0 obj\n<< /Type \
                 /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
                 /MediaBox [0 0 100 100] /Contents 4 0 R {page} >>\nendobj\n4 0 obj\n<< /Length \
                 {} >>\nstream\n{content}\nendstream\nendobj\n5 0 obj\n<< /Type /Filespec /F \
                 (file) /UF (file) /AFRelationship /Data >>\nendobj\n{objects}trailer\n<< /Root 1 \
                 0 R >>\n%%EOF\n",
                content.len(),
            );
            let report = Validator::new(options)?
                .validate_reader(Cursor::new(pdf.as_bytes()), InputName::memory())?;
            let profile = report
                .profile_reports
                .first()
                .ok_or(ProfileError::UnsupportedSelection)?;
            let rule = "iso-19005-3-6-8-4";
            let passes = profile
                .passed_assertions
                .iter()
                .filter(|item| item.rule_id.0.as_str() == rule)
                .count();
            let failures = profile
                .failed_assertions
                .iter()
                .filter(|item| item.rule_id.0.as_str() == rule)
                .count();
            assert_eq!((passes, failures), if associated { (2, 0) } else { (0, 2) });
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
