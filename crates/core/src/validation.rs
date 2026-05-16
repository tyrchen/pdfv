//! End-to-end validation session and validation model graph.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use crate::{
    Assertion, BoundedText, BuiltinProfileRepository, ENGINE_VERSION, ErrorArgument, Identifier,
    IndirectObject, InputKind, InputSummary, ModelValue, ObjectKey, ObjectLocation, ObjectTypeName,
    ParsedDocument, Parser, PdfName, PdfvError, ProfileReport, ProfileRepository, PropertyName,
    ResourceLimits, Result, Rule, RuleEvaluator, RuleId, RuleOutcome, TaskDuration,
    UnsupportedRule, ValidationError, ValidationOptions, ValidationReport, ValidationStatus,
    profile::DefaultRuleEvaluator, xmp::FlavourDetector,
};

const CATALOG_DIRECT_PROPERTIES: &[&str] = &["Type", "Metadata", "Pages", "OutputIntents"];
const METADATA_DIRECT_PROPERTIES: &[&str] = &["Type", "Subtype", "Filter", "Length"];
const PAGE_DIRECT_PROPERTIES: &[&str] = &["Type", "Parent", "Contents", "Resources", "Annots"];
const FONT_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "BaseFont",
    "FontDescriptor",
    "FirstChar",
    "LastChar",
    "Widths",
    "Encoding",
    "ToUnicode",
    "CIDToGIDMap",
];
const ANNOTATION_DIRECT_PROPERTIES: &[&str] = &[
    "Type", "Subtype", "F", "C", "IC", "AP", "FT", "CA", "A", "AA",
];
const OUTPUT_INTENT_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "S",
    "DestOutputProfile",
    "OutputConditionIdentifier",
    "Info",
];
const STREAM_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "Filter",
    "DecodeParms",
    "F",
    "FFilter",
    "FDecodeParms",
];

const RESOURCE_DIRECT_PROPERTIES: &[&str] = &[
    "Font",
    "XObject",
    "ColorSpace",
    "ExtGState",
    "Pattern",
    "Shading",
    "Properties",
    "ProcSet",
];
const ACRO_FORM_DIRECT_PROPERTIES: &[&str] = &[
    "Fields",
    "NeedAppearances",
    "SigFlags",
    "DR",
    "DA",
    "Q",
    "XFA",
];
const STRUCTURE_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "K",
    "ParentTree",
    "ParentTreeNextKey",
    "RoleMap",
    "ClassMap",
    "IDTree",
];
const OPTIONAL_CONTENT_DIRECT_PROPERTIES: &[&str] = &["OCGs", "D", "Configs"];
const NAMES_DIRECT_PROPERTIES: &[&str] = &[
    "Dests",
    "AP",
    "JavaScript",
    "Pages",
    "Templates",
    "IDS",
    "URLS",
    "EmbeddedFiles",
    "AlternatePresentations",
    "Renditions",
];
const OUTLINES_DIRECT_PROPERTIES: &[&str] = &["Type", "First", "Last", "Count"];
const DESTINATION_DIRECT_PROPERTIES: &[&str] = &["D", "Dest", "A"];
const ACTION_DIRECT_PROPERTIES: &[&str] = &["Type", "S", "D", "URI", "Next", "NewWindow"];
const FORM_FIELD_DIRECT_PROPERTIES: &[&str] = &[
    "FT", "T", "TU", "TM", "Ff", "V", "DV", "Kids", "Parent", "AA",
];
const IMAGE_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "Width",
    "Height",
    "ColorSpace",
    "BitsPerComponent",
    "Filter",
    "DecodeParms",
    "SMask",
    "Mask",
    "Intent",
];
const XOBJECT_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "BBox",
    "Matrix",
    "Resources",
    "Group",
    "Filter",
    "DecodeParms",
];
const CMAP_DIRECT_PROPERTIES: &[&str] = &["Type", "Subtype", "CMapName", "CIDSystemInfo"];
const COLOR_SPACE_DIRECT_PROPERTIES: &[&str] =
    &["Type", "N", "Alternate", "Range", "Metadata", "Filter"];
const EXT_GSTATE_DIRECT_PROPERTIES: &[&str] =
    &["Type", "BM", "CA", "ca", "SMask", "AIS", "OP", "op", "OPM"];
const SIGNATURE_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Filter",
    "SubFilter",
    "ByteRange",
    "Contents",
    "Reference",
    "M",
];
const SECURITY_DIRECT_PROPERTIES: &[&str] = &["Filter", "SubFilter", "V", "R", "Length", "P"];

const DIRECT_PROPERTY_NAMES: &[&str] = &[
    "A",
    "AA",
    "AIS",
    "AP",
    "Alternate",
    "AlternatePresentations",
    "Annot",
    "Annots",
    "BBox",
    "BM",
    "BaseFont",
    "BitsPerComponent",
    "ByteRange",
    "C",
    "CA",
    "CIDSystemInfo",
    "CIDToGIDMap",
    "ClassMap",
    "ColorSpace",
    "Configs",
    "Contents",
    "Count",
    "D",
    "DA",
    "DR",
    "DV",
    "DecodeParms",
    "Dest",
    "DestOutputProfile",
    "Dests",
    "EmbeddedFiles",
    "Encoding",
    "F",
    "FDecodeParms",
    "FFilter",
    "FT",
    "Ff",
    "Fields",
    "Filter",
    "First",
    "FirstChar",
    "Font",
    "FontDescriptor",
    "Group",
    "Height",
    "IC",
    "IDS",
    "IDTree",
    "Info",
    "Intent",
    "JavaScript",
    "K",
    "Kids",
    "Last",
    "LastChar",
    "Length",
    "M",
    "Mask",
    "Matrix",
    "Metadata",
    "N",
    "NeedAppearances",
    "Next",
    "OCGs",
    "OP",
    "OPM",
    "OutputConditionIdentifier",
    "P",
    "Pages",
    "Parent",
    "ParentTree",
    "ParentTreeNextKey",
    "Pattern",
    "ProcSet",
    "Properties",
    "Q",
    "Range",
    "Reference",
    "Renditions",
    "Resources",
    "RoleMap",
    "S",
    "SMask",
    "Shading",
    "SigFlags",
    "SubFilter",
    "Subtype",
    "T",
    "TM",
    "TU",
    "Templates",
    "ToUnicode",
    "Type",
    "URI",
    "URLS",
    "V",
    "Width",
    "Widths",
    "XFA",
    "XObject",
    "ca",
    "op",
];

const OBJECT_PROPERTIES: &[&str] = &["Type", "Subtype"];
const DOCUMENT_PROPERTIES: &[&str] = &[
    "headerOffset",
    "postEOFDataSize",
    "header",
    "encrypted",
    "isEncrypted",
    "hasCatalog",
    "containsXRefStream",
    "nrIndirects",
    "containsPDFUAIdentification",
    "containsPDFAIdentification",
    "part",
    "partPrefix",
    "rev",
    "revPrefix",
];
const CATALOG_PROPERTIES: &[&str] = &[
    "hasMetadata",
    "hasAcroForm",
    "hasStructTreeRoot",
    "hasOCProperties",
    "hasLang",
    "hasOutlines",
    "hasNames",
    "hasDests",
    "language",
    "permissions",
    "containsStructTreeRoot",
    "containsOCProperties",
    "containsAcroForm",
    "Marked",
    "Type",
    "Metadata",
    "Pages",
    "OutputIntents",
    "AcroForm",
    "StructTreeRoot",
    "OCProperties",
    "Lang",
    "Perms",
    "Outlines",
    "Names",
    "Dests",
];
const METADATA_PROPERTIES: &[&str] = &[
    "present",
    "catalogMetadata",
    "containsPDFAIdentification",
    "containsPDFUAIdentification",
    "part",
    "partPrefix",
    "conformance",
    "conformancePrefix",
    "rev",
    "revPrefix",
    "amdPrefix",
    "corrPrefix",
    "declarations",
    "Type",
    "Subtype",
    "Filter",
    "Length",
];
const PAGE_PROPERTIES: &[&str] = &[
    "hasContents",
    "hasResources",
    "annotationCount",
    "Type",
    "Parent",
    "Contents",
    "Resources",
    "Annots",
];
const PAGE_TREE_PROPERTIES: &[&str] = &["Type", "Kids", "Count", "Parent", "Resources"];
const RESOURCE_PROPERTIES: &[&str] = RESOURCE_DIRECT_PROPERTIES;
const NAMES_PROPERTIES: &[&str] = NAMES_DIRECT_PROPERTIES;
const OUTLINE_PROPERTIES: &[&str] = OUTLINES_DIRECT_PROPERTIES;
const DESTINATION_PROPERTIES: &[&str] = DESTINATION_DIRECT_PROPERTIES;
const ACRO_FORM_PROPERTIES: &[&str] = ACRO_FORM_DIRECT_PROPERTIES;
const OPTIONAL_CONTENT_PROPERTIES: &[&str] = OPTIONAL_CONTENT_DIRECT_PROPERTIES;
const PERMISSIONS_PROPERTIES: &[&str] = &["DocMDP", "UR", "UR3"];
const FONT_PROPERTIES: &[&str] = &[
    "embedded",
    "hasSubtype",
    "Type",
    "Subtype",
    "BaseFont",
    "FontDescriptor",
    "FirstChar",
    "LastChar",
    "Widths",
    "Encoding",
    "ToUnicode",
    "CIDToGIDMap",
];
const CMAP_PROPERTIES: &[&str] = CMAP_DIRECT_PROPERTIES;
const IMAGE_PROPERTIES: &[&str] = IMAGE_DIRECT_PROPERTIES;
const XOBJECT_PROPERTIES: &[&str] = XOBJECT_DIRECT_PROPERTIES;
const CONTENT_STREAM_PROPERTIES: &[&str] = &[
    "lengthMatches",
    "declaredLength",
    "discoveredLength",
    "operatorCount",
    "markedContentCount",
    "Type",
    "Subtype",
    "Filter",
    "DecodeParms",
    "F",
    "FFilter",
    "FDecodeParms",
];
const ANNOTATION_PROPERTIES: &[&str] = &[
    "hasSubtype",
    "Type",
    "Subtype",
    "F",
    "C",
    "IC",
    "AP",
    "FT",
    "CA",
    "A",
    "AA",
];
const ACTION_PROPERTIES: &[&str] = ACTION_DIRECT_PROPERTIES;
const FORM_FIELD_PROPERTIES: &[&str] = FORM_FIELD_DIRECT_PROPERTIES;
const COLOR_SPACE_PROPERTIES: &[&str] = COLOR_SPACE_DIRECT_PROPERTIES;
const EXT_GSTATE_PROPERTIES: &[&str] = EXT_GSTATE_DIRECT_PROPERTIES;
const STRUCTURE_PROPERTIES: &[&str] = STRUCTURE_DIRECT_PROPERTIES;
const STRUCTURE_ELEMENT_PROPERTIES: &[&str] = &[
    "Type",
    "S",
    "P",
    "K",
    "Pg",
    "Alt",
    "ActualText",
    "Lang",
    "A",
    "C",
    "ID",
    "containsParent",
    "containsRef",
    "parentStandardType",
    "parentStandardTypeNamespaceURL",
    "parentType",
    "parentNamespaceURL",
    "structParentStandardType",
    "structParentType",
    "firstChildStandardTypeNamespaceURL",
    "kidsStandardTypes",
    "hasContentItems",
    "containsLabels",
    "ListNumbering",
    "NoteType",
    "orphanRefs",
    "ghostRefs",
    "isArtifact",
    "isTaggedContent",
    "parentsTags",
    "isNotMappedToStandardType",
    "circularMappingExist",
    "roleMapToSameNamespaceTag",
    "remappedStandardType",
    "hasIntersection",
    "numberOfColumnWithWrongRowSpan",
    "numberOfRowWithWrongColumnSpan",
    "wrongColumnSpan",
    "differentTargetAnnotObjectKey",
];
const SIGNATURE_PROPERTIES: &[&str] = SIGNATURE_DIRECT_PROPERTIES;
const SECURITY_PROPERTIES: &[&str] = SECURITY_DIRECT_PROPERTIES;
const OUTPUT_INTENT_PROPERTIES: &[&str] = &[
    "hasDestOutputProfile",
    "Type",
    "S",
    "DestOutputProfile",
    "OutputConditionIdentifier",
    "Info",
];
const STREAM_PROPERTIES: &[&str] = &[
    "lengthMatches",
    "declaredLength",
    "discoveredLength",
    "streamKeywordCRLFCompliant",
    "endstreamKeywordEOLCompliant",
    "Type",
    "Subtype",
    "Filter",
    "DecodeParms",
    "F",
    "FFilter",
    "FDecodeParms",
];

const EMPTY_LINK_NAMES: &[(&str, &str)] = &[];
const DOCUMENT_LINKS: &[(&str, &str)] = &[("catalog", "catalog"), ("streams", "stream")];
const CATALOG_LINKS: &[(&str, &str)] = &[
    ("metadata", "metadata"),
    ("pages", "page"),
    ("outputIntents", "outputIntent"),
    ("acroForm", "acroForm"),
    ("structureTreeRoot", "structureTreeRoot"),
    ("optionalContentProperties", "optionalContentProperties"),
    ("names", "names"),
    ("outlines", "outline"),
    ("destinations", "destination"),
    ("permissions", "permissions"),
];
const PAGE_LINKS: &[(&str, &str)] = &[
    ("resources", "resource"),
    ("fonts", "font"),
    ("annotations", "annotation"),
    ("contentStreams", "contentStream"),
];

/// Bounded input name used by reader validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputName(Option<PathBuf>);

impl InputName {
    /// Creates an in-memory input name.
    #[must_use]
    pub fn memory() -> Self {
        Self(None)
    }

    /// Creates a filesystem input name.
    #[must_use]
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self(Some(path.into()))
    }

    fn summary(&self, kind: InputKind, bytes: Option<u64>) -> InputSummary {
        InputSummary::new(kind, self.0.clone(), bytes)
    }
}

/// Validation facade for parser, profile selection, traversal, and reports.
#[derive(Clone)]
pub struct Validator {
    options: ValidationOptions,
    profiles: Arc<dyn ProfileRepository + Send + Sync>,
}

impl std::fmt::Debug for Validator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Validator")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl Validator {
    /// Creates a validator with the built-in profile repository.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if profile selection for the supplied options is invalid.
    pub fn new(options: ValidationOptions) -> Result<Self> {
        let validator = Self {
            options,
            profiles: Arc::new(BuiltinProfileRepository::new()),
        };
        validator
            .profiles
            .profiles_for(&validator.options.flavour)?;
        Ok(validator)
    }

    /// Creates a validator with an explicit profile repository.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if profile selection for the supplied options is invalid.
    pub fn with_profiles(
        options: ValidationOptions,
        profiles: Arc<dyn ProfileRepository + Send + Sync>,
    ) -> Result<Self> {
        let validator = Self { options, profiles };
        validator
            .profiles
            .profiles_for(&validator.options.flavour)?;
        Ok(validator)
    }

    /// Validates a PDF file from disk.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] for I/O failures or validation engine failures.
    #[allow(
        clippy::disallowed_types,
        reason = "core validation is synchronous per spec; async file I/O belongs to the CLI phase"
    )]
    pub fn validate_path(&self, path: impl AsRef<Path>) -> Result<ValidationReport> {
        let path = path.as_ref();
        let file = std::fs::File::open(path).map_err(|source| PdfvError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
        let name = InputName::path(path);
        self.validate_reader_with_kind(file, &name, InputKind::File)
    }

    /// Validates a seekable PDF reader.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] for I/O failures or validation engine failures.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "public API owns InputName to match the validation facade contract"
    )]
    pub fn validate_reader<R: Read + Seek>(
        &self,
        source: R,
        name: InputName,
    ) -> Result<ValidationReport> {
        self.validate_reader_with_kind(source, &name, InputKind::Memory)
    }

    fn validate_reader_with_kind<R: Read + Seek>(
        &self,
        mut source: R,
        name: &InputName,
        kind: InputKind,
    ) -> Result<ValidationReport> {
        let started = Instant::now();
        let bytes = reader_len(&mut source)?;
        source
            .rewind()
            .map_err(|source| PdfvError::Io { path: None, source })?;
        let source_summary = name.summary(kind, bytes);
        let parser = Parser::new(self.options.resource_limits.clone());
        let parsed = match parser.parse_with_options(
            source,
            crate::ParseOptions {
                password: self.options.password.as_ref(),
            },
        ) {
            Ok(parsed) => parsed,
            Err(PdfvError::Parse(error)) => {
                return parse_failed_report(source_summary, &error, started.elapsed());
            }
            Err(error) => return Err(error),
        };

        if parsed.is_encrypted() {
            return base_report(
                source_summary,
                ValidationStatus::Encrypted,
                Vec::new(),
                parsed,
                started.elapsed(),
            );
        }

        let mut parsed = parsed;
        let profiles = match &self.options.flavour {
            crate::FlavourSelection::Auto { default } => {
                let detected = FlavourDetector::new(Arc::clone(&self.profiles)).detect(
                    &parsed,
                    default.as_ref(),
                    &self.options.resource_limits,
                )?;
                parsed.parse_facts.extend(detected.parse_facts);
                parsed.warnings.extend(detected.warnings);
                detected.profiles
            }
            crate::FlavourSelection::Explicit { .. }
            | crate::FlavourSelection::CustomProfile { .. } => {
                self.profiles.profiles_for(&self.options.flavour)?
            }
        };
        if profiles.is_empty() {
            return base_report(
                source_summary,
                ValidationStatus::Incomplete,
                Vec::new(),
                parsed,
                started.elapsed(),
            );
        }
        let mut session = ValidationSession::new(
            parsed,
            self.options.resource_limits.clone(),
            self.options.max_failed_assertions_per_rule.get(),
            self.options.record_passed_assertions,
        );
        let mut profile_reports = Vec::with_capacity(profiles.len());
        for profile in &profiles {
            profile_reports.push(session.validate_profile(profile)?);
        }
        let status = if profile_reports
            .iter()
            .any(|report| !report.unsupported_rules.is_empty())
        {
            ValidationStatus::Incomplete
        } else if profile_reports.iter().all(|report| report.is_compliant) {
            ValidationStatus::Valid
        } else {
            ValidationStatus::Invalid
        };
        let flavours = profiles
            .iter()
            .map(|profile| profile.flavour.clone())
            .collect::<Vec<_>>();

        let parse_facts = session.document.parse_facts.clone();
        let warnings = session.document.warnings.clone();
        Ok(ValidationReport::builder()
            .engine_version(ENGINE_VERSION.to_owned())
            .source(source_summary)
            .status(status)
            .flavours(flavours)
            .profile_reports(profile_reports)
            .parse_facts(parse_facts)
            .warnings(warnings)
            .task_durations(vec![TaskDuration::from_duration(
                Identifier::new("validate")?,
                started.elapsed(),
            )])
            .build())
    }
}

/// Mutable validation state for one input.
#[derive(Debug)]
pub struct ValidationSession {
    document: ParsedDocument,
    limits: ResourceLimits,
    max_failed_assertions_per_rule: u32,
    record_passed_assertions: bool,
}

impl ValidationSession {
    fn new(
        document: ParsedDocument,
        limits: ResourceLimits,
        max_failed_assertions_per_rule: u32,
        record_passed_assertions: bool,
    ) -> Self {
        Self {
            document,
            limits,
            max_failed_assertions_per_rule,
            record_passed_assertions,
        }
    }

    fn validate_profile(&mut self, profile: &crate::ValidationProfile) -> Result<ProfileReport> {
        let index = RuleIndex::new(&profile.rules);
        let graph = ModelGraph::for_rules(&self.document, &self.limits, &profile.rules);
        let mut evaluator = DefaultRuleEvaluator::new(self.limits.clone());
        let mut state = ProfileState::new(
            profile.identity.clone(),
            self.max_failed_assertions_per_rule,
            self.record_passed_assertions,
        );
        state.register_static_unsupported_rules(&profile.rules);
        let mut stack = Vec::from([ModelObjectRef::Document(DocumentModel::new(&self.document))]);
        let mut visited = HashSet::new();
        let mut deferred = Vec::new();

        while let Some(object) = stack.pop() {
            let visited_key = object.identity_key();
            if !visited.insert(visited_key) {
                continue;
            }
            let object_rules = index.rules_for(&object);
            for rule in object_rules {
                if matches!(rule.test, crate::RuleExpr::Unsupported { .. }) {
                    continue;
                }
                if rule.deferred {
                    deferred.push((object.clone(), rule));
                } else {
                    state.apply_rule(&object, rule, &mut evaluator)?;
                }
            }
            if u64::try_from(visited.len()).map_err(|_| ValidationError::LimitExceeded {
                limit: "max_objects",
            })? > self.limits.max_objects
            {
                return Err(ValidationError::LimitExceeded {
                    limit: "max_objects",
                }
                .into());
            }
            let object_budget = remaining_object_budget(&self.limits, visited.len(), stack.len())?;
            for linked in object.linked_objects(&graph, object_budget)? {
                stack.push(linked);
            }
        }
        for (object, rule) in deferred {
            state.apply_rule(&object, rule, &mut evaluator)?;
        }
        Ok(state.finish())
    }
}

/// Property exposed by a validation model family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PropertySpec {
    /// Property name.
    pub name: PropertyName,
}

impl PropertySpec {
    fn new(name: &str) -> Self {
        Self {
            name: PropertyName::unchecked(name),
        }
    }
}

/// Link exposed by a validation model family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkSpec {
    /// Link name.
    pub name: LinkName,
    /// Target model family.
    pub target: ObjectTypeName,
}

impl LinkSpec {
    fn new(name: &'static str, target: &'static str) -> Self {
        Self {
            name: LinkName(Identifier::unchecked(name)),
            target: ObjectTypeName::unchecked(target),
        }
    }
}

/// Validation model family schema entry.
pub(crate) trait ModelFamily {
    /// Family name.
    fn family_name(&self) -> ObjectTypeName;
    /// Properties allowed on this family.
    fn property_schema(&self) -> &[PropertySpec];
    /// Links allowed on this family.
    fn link_schema(&self) -> &[LinkSpec];
}

/// Internal registry of validation model family schemas.
#[derive(Clone)]
pub(crate) struct ModelRegistry {
    families: BTreeMap<ObjectTypeName, Arc<dyn ModelFamily + Send + Sync>>,
    all_properties: BTreeSet<PropertyName>,
}

impl std::fmt::Debug for ModelRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelRegistry")
            .field("families", &self.families.keys().collect::<Vec<_>>())
            .field("all_properties_len", &self.all_properties.len())
            .finish()
    }
}

impl ModelRegistry {
    /// Builds the default internal registry.
    #[must_use]
    pub(crate) fn default_registry() -> Self {
        let families = [
            family("document", DOCUMENT_PROPERTIES, DOCUMENT_LINKS),
            family("catalog", CATALOG_PROPERTIES, CATALOG_LINKS),
            family("metadata", METADATA_PROPERTIES, EMPTY_LINK_NAMES),
            family("page", PAGE_PROPERTIES, PAGE_LINKS),
            family("pageTree", PAGE_TREE_PROPERTIES, EMPTY_LINK_NAMES),
            family("resource", RESOURCE_PROPERTIES, EMPTY_LINK_NAMES),
            family("names", NAMES_PROPERTIES, EMPTY_LINK_NAMES),
            family("outline", OUTLINE_PROPERTIES, EMPTY_LINK_NAMES),
            family("destination", DESTINATION_PROPERTIES, EMPTY_LINK_NAMES),
            family("acroForm", ACRO_FORM_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "optionalContentProperties",
                OPTIONAL_CONTENT_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("permissions", PERMISSIONS_PROPERTIES, EMPTY_LINK_NAMES),
            family("font", FONT_PROPERTIES, EMPTY_LINK_NAMES),
            family("cMap", CMAP_PROPERTIES, EMPTY_LINK_NAMES),
            family("embeddedFontFile", STREAM_PROPERTIES, EMPTY_LINK_NAMES),
            family("image", IMAGE_PROPERTIES, EMPTY_LINK_NAMES),
            family("xObject", XOBJECT_PROPERTIES, EMPTY_LINK_NAMES),
            family("contentStream", CONTENT_STREAM_PROPERTIES, EMPTY_LINK_NAMES),
            family("annotation", ANNOTATION_PROPERTIES, EMPTY_LINK_NAMES),
            family("action", ACTION_PROPERTIES, EMPTY_LINK_NAMES),
            family("formField", FORM_FIELD_PROPERTIES, EMPTY_LINK_NAMES),
            family("colorSpace", COLOR_SPACE_PROPERTIES, EMPTY_LINK_NAMES),
            family("extGState", EXT_GSTATE_PROPERTIES, EMPTY_LINK_NAMES),
            family("structureTreeRoot", STRUCTURE_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "structureElement",
                STRUCTURE_ELEMENT_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("signature", SIGNATURE_PROPERTIES, EMPTY_LINK_NAMES),
            family("security", SECURITY_PROPERTIES, EMPTY_LINK_NAMES),
            family("outputIntent", OUTPUT_INTENT_PROPERTIES, EMPTY_LINK_NAMES),
            family("stream", STREAM_PROPERTIES, EMPTY_LINK_NAMES),
            family("object", OBJECT_PROPERTIES, EMPTY_LINK_NAMES),
        ];
        let mut by_name: BTreeMap<ObjectTypeName, Arc<dyn ModelFamily + Send + Sync>> =
            BTreeMap::new();
        let mut all_properties = BTreeSet::new();
        for family in families {
            for property in family.property_schema() {
                all_properties.insert(property.name.clone());
            }
            by_name.insert(family.family_name(), Arc::new(family) as Arc<_>);
        }
        for family in by_name.values() {
            for link in family.link_schema() {
                debug_assert!(
                    by_name.contains_key(&link.target),
                    "model registry link target is not registered"
                );
            }
        }
        for property in DIRECT_PROPERTY_NAMES {
            all_properties.insert(PropertyName::unchecked(*property));
        }
        Self {
            families: by_name,
            all_properties,
        }
    }

    /// Returns true when a family is registered.
    #[must_use]
    pub(crate) fn has_family(&self, family: &ObjectTypeName) -> bool {
        self.families.contains_key(family)
    }

    /// Returns true when a property is present on a specific registered family schema.
    #[must_use]
    pub(crate) fn has_family_property(
        &self,
        family: &ObjectTypeName,
        property: &PropertyName,
    ) -> bool {
        self.families.get(family).is_some_and(|family| {
            family
                .property_schema()
                .iter()
                .any(|spec| spec.name == *property)
        })
    }

    /// Iterates registered family names.
    #[cfg(test)]
    pub(crate) fn family_names(&self) -> impl Iterator<Item = &ObjectTypeName> {
        self.families.keys()
    }
}

#[derive(Debug)]
struct StaticModelFamily {
    name: ObjectTypeName,
    properties: Vec<PropertySpec>,
    links: Vec<LinkSpec>,
}

impl ModelFamily for StaticModelFamily {
    fn family_name(&self) -> ObjectTypeName {
        self.name.clone()
    }

    fn property_schema(&self) -> &[PropertySpec] {
        &self.properties
    }

    fn link_schema(&self) -> &[LinkSpec] {
        &self.links
    }
}

fn family(
    name: &'static str,
    properties: &'static [&'static str],
    links: &'static [(&'static str, &'static str)],
) -> StaticModelFamily {
    StaticModelFamily {
        name: ObjectTypeName::unchecked(name),
        properties: properties
            .iter()
            .map(|name| PropertySpec::new(name))
            .collect(),
        links: links
            .iter()
            .map(|(name, target)| LinkSpec::new(name, target))
            .collect(),
    }
}

/// Stable object identity used by traversal.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ObjectIdentity {
    key: String,
}

/// Validation model link name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LinkName(Identifier);

impl LinkName {
    /// Creates a link name.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ConfigError`] when the identifier violates policy.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, crate::ConfigError> {
        Ok(Self(Identifier::new(value)?))
    }
}

/// Validation model object.
pub trait ModelObject {
    /// Optional stable object identity.
    fn id(&self) -> Option<ObjectIdentity>;
    /// Concrete object type.
    fn object_type(&self) -> ObjectTypeName;
    /// Supertype names.
    fn super_types(&self) -> &[ObjectTypeName];
    /// Extra diagnostic context.
    fn extra_context(&self) -> Option<&str>;
    /// Looks up a property.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when the property is unknown or cannot be materialized.
    fn property(&self, name: &PropertyName) -> Result<ModelValue>;
    /// Link names exposed by this object.
    fn links(&self) -> &[LinkName];
    /// Resolves linked model objects.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when link materialization fails.
    fn linked_objects<'a>(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>>;
}

/// Validation model object reference.
#[derive(Clone, Debug)]
pub enum ModelObjectRef<'a> {
    /// Document root object.
    Document(DocumentModel<'a>),
    /// Catalog object.
    Catalog(CatalogModel<'a>),
    /// Metadata stream object.
    Metadata(MetadataModel<'a>),
    /// Page dictionary object.
    Page(PageModel<'a>),
    /// Font dictionary object.
    Font(FontModel<'a>),
    /// Annotation dictionary object.
    Annotation(AnnotationModel<'a>),
    /// Output intent dictionary object.
    OutputIntent(OutputIntentModel<'a>),
    /// Page content stream object.
    ContentStream(ContentStreamModel<'a>),
    /// Basic stream object.
    Stream(StreamModel<'a>),
    /// Generic dictionary-backed model family object.
    Generic(GenericModel<'a>),
}

impl<'a> ModelObjectRef<'a> {
    /// Returns the parsed document backing this model object.
    #[must_use]
    pub fn document(&self) -> &'a ParsedDocument {
        match self {
            Self::Document(model) => model.document,
            Self::Catalog(model) => model.document,
            Self::Metadata(model) => model.document,
            Self::Page(model) => model.document,
            Self::Font(model) => model.document,
            Self::Annotation(model) => model.document,
            Self::OutputIntent(model) => model.document,
            Self::ContentStream(model) => model.document,
            Self::Stream(model) => model.document,
            Self::Generic(model) => model.document,
        }
    }

    /// Returns this object's type.
    #[must_use]
    pub fn object_type(&self) -> ObjectTypeName {
        match self {
            Self::Document(model) => model.object_type(),
            Self::Catalog(model) => model.object_type(),
            Self::Metadata(model) => model.object_type(),
            Self::Page(model) => model.object_type(),
            Self::Font(model) => model.object_type(),
            Self::Annotation(model) => model.object_type(),
            Self::OutputIntent(model) => model.object_type(),
            Self::ContentStream(model) => model.object_type(),
            Self::Stream(model) => model.object_type(),
            Self::Generic(model) => model.object_type(),
        }
    }

    /// Looks up a property.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when the property is unknown.
    pub fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match self {
            Self::Document(model) => model.property(name),
            Self::Catalog(model) => model.property(name),
            Self::Metadata(model) => model.property(name),
            Self::Page(model) => model.property(name),
            Self::Font(model) => model.property(name),
            Self::Annotation(model) => model.property(name),
            Self::OutputIntent(model) => model.property(name),
            Self::ContentStream(model) => model.property(name),
            Self::Stream(model) => model.property(name),
            Self::Generic(model) => model.property(name),
        }
    }

    fn location(&self) -> ObjectLocation {
        match self {
            Self::Document(_) => ObjectLocation {
                object: None,
                offset: None,
                path: Some(BoundedText::unchecked("root")),
            },
            Self::Catalog(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked("root/catalog[0]")),
            },
            Self::Metadata(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked("root/catalog[0]/metadata[0]")),
            },
            Self::Page(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked(format!(
                    "root/page[{}]",
                    model.ordinal
                ))),
            },
            Self::Font(model) => ObjectLocation {
                object: model.key,
                offset: model.offset,
                path: Some(BoundedText::unchecked(format!(
                    "root/page[{}]/font[{}]",
                    model.page_ordinal,
                    String::from_utf8_lossy(model.name.as_bytes())
                ))),
            },
            Self::Annotation(model) => ObjectLocation {
                object: model.key,
                offset: model.offset,
                path: Some(BoundedText::unchecked(format!(
                    "root/page[{}]/annotation[{}]",
                    model.page_ordinal, model.ordinal
                ))),
            },
            Self::OutputIntent(model) => ObjectLocation {
                object: model.key,
                offset: model.offset,
                path: Some(BoundedText::unchecked(format!(
                    "root/catalog[0]/outputIntent[{}]",
                    model.ordinal
                ))),
            },
            Self::ContentStream(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked(format!(
                    "root/page[{}]/contentStream[{}]",
                    model.page_ordinal, model.ordinal
                ))),
            },
            Self::Stream(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked(format!(
                    "root/stream[{}]",
                    model.key.number
                ))),
            },
            Self::Generic(model) => ObjectLocation {
                object: model.key,
                offset: model.offset,
                path: Some(BoundedText::unchecked(model.context.clone())),
            },
        }
    }

    fn context(&self) -> BoundedText {
        match self {
            Self::Document(_) => BoundedText::unchecked("root"),
            Self::Catalog(_) => BoundedText::unchecked("root/catalog[0]"),
            Self::Metadata(_) => BoundedText::unchecked("root/catalog[0]/metadata[0]"),
            Self::Page(model) => BoundedText::unchecked(format!("root/page[{}]", model.ordinal)),
            Self::Font(model) => BoundedText::unchecked(format!(
                "root/page[{}]/font[{}]",
                model.page_ordinal,
                String::from_utf8_lossy(model.name.as_bytes())
            )),
            Self::Annotation(model) => BoundedText::unchecked(format!(
                "root/page[{}]/annotation[{}]",
                model.page_ordinal, model.ordinal
            )),
            Self::OutputIntent(model) => {
                BoundedText::unchecked(format!("root/catalog[0]/outputIntent[{}]", model.ordinal))
            }
            Self::ContentStream(model) => BoundedText::unchecked(format!(
                "root/page[{}]/contentStream[{}]",
                model.page_ordinal, model.ordinal
            )),
            Self::Stream(model) => {
                BoundedText::unchecked(format!("root/stream[{}]", model.key.number))
            }
            Self::Generic(model) => BoundedText::unchecked(model.context.clone()),
        }
    }

    fn identity_key(&self) -> String {
        match self {
            Self::Document(_) => String::from("document"),
            Self::Catalog(model) => {
                format!("catalog:{}:{}", model.key.number, model.key.generation)
            }
            Self::Metadata(model) => {
                format!("metadata:{}:{}", model.key.number, model.key.generation)
            }
            Self::Page(model) => format!("page:{}:{}", model.key.number, model.key.generation),
            Self::Font(model) => format!(
                "font:{}:{}:{}",
                model.page_ordinal,
                model.key.map_or(0, |key| key.number.get()),
                String::from_utf8_lossy(model.name.as_bytes())
            ),
            Self::Annotation(model) => format!(
                "annotation:{}:{}:{}",
                model.page_ordinal,
                model.ordinal,
                model.key.map_or(0, |key| key.number.get())
            ),
            Self::OutputIntent(model) => format!(
                "outputIntent:{}:{}",
                model.ordinal,
                model.key.map_or(0, |key| key.number.get())
            ),
            Self::ContentStream(model) => format!(
                "contentStream:{}:{}:{}",
                model.page_ordinal, model.key.number, model.key.generation
            ),
            Self::Stream(model) => format!("stream:{}:{}", model.key.number, model.key.generation),
            Self::Generic(model) => format!(
                "{}:{}:{}",
                model.object_type.as_str(),
                model.ordinal,
                model.key.map_or(0, |key| key.number.get())
            ),
        }
    }

    fn linked_objects(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        match self {
            Self::Document(model) => model.linked_objects(graph, max_objects),
            Self::Catalog(model) => model.linked_objects(graph, max_objects),
            Self::Metadata(model) => model.linked_objects(graph, max_objects),
            Self::Page(model) => model.linked_objects(graph, max_objects),
            Self::Font(model) => model.linked_objects(graph, max_objects),
            Self::Annotation(model) => model.linked_objects(graph, max_objects),
            Self::OutputIntent(model) => model.linked_objects(graph, max_objects),
            Self::ContentStream(model) => model.linked_objects(graph, max_objects),
            Self::Stream(model) => model.linked_objects(graph, max_objects),
            Self::Generic(model) => model.linked_objects(graph, max_objects),
        }
    }
}

/// Document model wrapper.
#[derive(Debug)]
pub struct ModelGraph<'a> {
    document: &'a ParsedDocument,
    limits: &'a ResourceLimits,
    materialized_families: BTreeSet<ObjectTypeName>,
}

#[derive(Clone, Copy, Debug)]
struct ResourceCollection<'a> {
    resources: &'a crate::Dictionary,
    resource_name: &'static str,
    family: &'static str,
    context_prefix: &'static str,
    page_ordinal: usize,
    max_objects: usize,
}

impl<'a> ModelGraph<'a> {
    fn for_rules(document: &'a ParsedDocument, limits: &'a ResourceLimits, rules: &[Rule]) -> Self {
        let materialized_families = rules
            .iter()
            .filter(|rule| !matches!(rule.test, crate::RuleExpr::Unsupported { .. }))
            .map(|rule| rule.object_type.clone())
            .collect();
        Self {
            document,
            limits,
            materialized_families,
        }
    }

    #[cfg(test)]
    fn with_all_families(document: &'a ParsedDocument, limits: &'a ResourceLimits) -> Self {
        Self {
            document,
            limits,
            materialized_families: ModelRegistry::default_registry()
                .family_names()
                .cloned()
                .collect(),
        }
    }

    fn materializes(&self, family: &str) -> bool {
        self.materialized_families
            .iter()
            .any(|materialized| materialized.as_str() == family)
    }

    fn materializes_generic_roots(&self) -> bool {
        self.materialized_families.iter().any(|family| {
            !matches!(
                family.as_str(),
                "document"
                    | "catalog"
                    | "metadata"
                    | "page"
                    | "font"
                    | "annotation"
                    | "outputIntent"
                    | "contentStream"
                    | "stream"
                    | "object"
            )
        })
    }

    fn catalog(&self) -> Option<CatalogModel<'a>> {
        self.document
            .catalog
            .and_then(|key| CatalogModel::new(self.document, key))
    }

    fn metadata(&self, catalog: &CatalogModel<'_>) -> Option<MetadataModel<'a>> {
        MetadataModel::new(self.document, catalog.metadata)
    }

    fn pages(&self, catalog: &CatalogModel<'_>, max_objects: usize) -> Result<Vec<PageModel<'a>>> {
        PageModel::from_catalog(self.document, catalog, self.limits, max_objects)
    }

    fn fonts(&self, page: &PageModel<'_>, max_objects: usize) -> Result<Vec<FontModel<'a>>> {
        FontModel::from_page(self.document, page, max_objects)
    }

    fn annotations(
        &self,
        page: &PageModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<AnnotationModel<'a>>> {
        AnnotationModel::from_page(self.document, page, max_objects)
    }

    fn output_intents(
        &self,
        catalog: &CatalogModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<OutputIntentModel<'a>>> {
        OutputIntentModel::from_catalog(self.document, catalog, max_objects)
    }

    fn content_streams(
        &self,
        page: &PageModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<ContentStreamModel<'a>>> {
        ContentStreamModel::from_page(self.document, page, max_objects)
    }

    fn push_streams(
        &self,
        objects: &mut Vec<ModelObjectRef<'a>>,
        max_objects: usize,
    ) -> Result<()> {
        for object in self.document.objects.values() {
            let Some(stream) = StreamModel::from_indirect_with_document(self.document, object)
            else {
                continue;
            };
            if Some(stream.key) != self.document.catalog {
                push_linked(objects, ModelObjectRef::Stream(stream), max_objects)?;
            }
        }
        Ok(())
    }

    fn push_generic_roots(
        &self,
        objects: &mut Vec<ModelObjectRef<'a>>,
        max_objects: usize,
    ) -> Result<()> {
        for model in self.generic_models(max_objects)? {
            if !self.materialized_families.contains(&model.object_type) {
                continue;
            }
            push_linked(objects, ModelObjectRef::Generic(model), max_objects)?;
        }
        Ok(())
    }

    fn generic_models(&self, max_objects: usize) -> Result<Vec<GenericModel<'a>>> {
        let mut models = Vec::new();
        if let Some(catalog) = self.catalog() {
            self.push_catalog_generic_models(&catalog, max_objects, &mut models)?;
            for page in self.pages(&catalog, max_objects)? {
                self.push_page_generic_models(&page, max_objects, &mut models)?;
            }
        }
        self.push_indirect_generic_models(max_objects, &mut models)?;
        Ok(models)
    }

    fn push_catalog_generic_models(
        &self,
        catalog: &CatalogModel<'_>,
        max_objects: usize,
        models: &mut Vec<GenericModel<'a>>,
    ) -> Result<()> {
        let Some(catalog_object) = self.document.objects.get(&catalog.key) else {
            return Ok(());
        };
        let Some(dictionary) = catalog_object.object.as_dictionary() else {
            return Ok(());
        };
        for (family, key_name, context) in [
            ("acroForm", "AcroForm", "root/catalog[0]/acroForm[0]"),
            (
                "structureTreeRoot",
                "StructTreeRoot",
                "root/catalog[0]/structureTreeRoot[0]",
            ),
            (
                "optionalContentProperties",
                "OCProperties",
                "root/catalog[0]/optionalContentProperties[0]",
            ),
            ("names", "Names", "root/catalog[0]/names[0]"),
            ("outline", "Outlines", "root/catalog[0]/outline[0]"),
            ("permissions", "Perms", "root/catalog[0]/permissions[0]"),
        ] {
            if let Some((key, offset, dictionary)) =
                resolve_named_dictionary_from_option(self.document, dictionary.get(key_name))
            {
                push_generic_model(
                    models,
                    GenericModel::new(
                        self.document,
                        family,
                        key,
                        offset,
                        dictionary,
                        models.len(),
                        context,
                    ),
                    max_objects,
                )?;
            }
        }
        for (ordinal, value) in array_values(dictionary.get("Dests")).enumerate() {
            if let Some((key, offset, dictionary)) = resolve_named_dictionary(self.document, value)
            {
                push_generic_model(
                    models,
                    GenericModel::new(
                        self.document,
                        "destination",
                        key,
                        offset,
                        dictionary,
                        ordinal,
                        format!("root/catalog[0]/destination[{ordinal}]"),
                    ),
                    max_objects,
                )?;
            }
        }
        Ok(())
    }

    fn push_page_generic_models(
        &self,
        page: &PageModel<'a>,
        max_objects: usize,
        models: &mut Vec<GenericModel<'a>>,
    ) -> Result<()> {
        if let Some(resources) =
            resolve_dictionary_value(self.document, page.dictionary.get("Resources"))
        {
            push_generic_model(
                models,
                GenericModel::new(
                    self.document,
                    "resource",
                    None,
                    None,
                    resources,
                    page.ordinal,
                    format!("root/page[{}]/resources[0]", page.ordinal),
                ),
                max_objects,
            )?;
            self.push_resource_collection(
                ResourceCollection {
                    resources,
                    resource_name: "XObject",
                    family: "xObject",
                    context_prefix: "root/page",
                    page_ordinal: page.ordinal,
                    max_objects,
                },
                models,
            )?;
            self.push_resource_collection(
                ResourceCollection {
                    resources,
                    resource_name: "ColorSpace",
                    family: "colorSpace",
                    context_prefix: "root/page",
                    page_ordinal: page.ordinal,
                    max_objects,
                },
                models,
            )?;
            self.push_resource_collection(
                ResourceCollection {
                    resources,
                    resource_name: "ExtGState",
                    family: "extGState",
                    context_prefix: "root/page",
                    page_ordinal: page.ordinal,
                    max_objects,
                },
                models,
            )?;
        }
        Ok(())
    }

    fn push_resource_collection(
        &self,
        collection: ResourceCollection<'a>,
        models: &mut Vec<GenericModel<'a>>,
    ) -> Result<()> {
        let Some(crate::CosObject::Dictionary(resources)) =
            collection.resources.get(collection.resource_name)
        else {
            return Ok(());
        };
        for (ordinal, (name, value)) in resources.iter().enumerate() {
            if let Some((key, offset, dictionary)) = resolve_named_dictionary(self.document, value)
            {
                let object_family = if collection.family == "xObject" {
                    classify_xobject(dictionary).unwrap_or(collection.family)
                } else {
                    collection.family
                };
                push_generic_model(
                    models,
                    GenericModel::new(
                        self.document,
                        object_family,
                        key,
                        offset,
                        dictionary,
                        ordinal,
                        format!(
                            "{}[{}]/{}[{}]",
                            collection.context_prefix,
                            collection.page_ordinal,
                            collection.family,
                            String::from_utf8_lossy(name.as_bytes())
                        ),
                    ),
                    collection.max_objects,
                )?;
            }
        }
        Ok(())
    }

    fn push_indirect_generic_models(
        &self,
        max_objects: usize,
        models: &mut Vec<GenericModel<'a>>,
    ) -> Result<()> {
        for object in self.document.objects.values() {
            let Some(dictionary) = object.object.as_dictionary() else {
                continue;
            };
            let Some(family) = classify_dictionary(dictionary) else {
                continue;
            };
            if matches!(
                family,
                "catalog" | "page" | "font" | "annotation" | "outputIntent" | "metadata"
            ) {
                continue;
            }
            push_generic_model(
                models,
                GenericModel::new(
                    self.document,
                    family,
                    Some(object.key),
                    Some(object.offset),
                    dictionary,
                    models.len(),
                    format!("root/{family}[{}]", object.key.number),
                ),
                max_objects,
            )?;
        }
        Ok(())
    }
}

/// Document model wrapper.
#[derive(Clone, Debug)]
pub struct DocumentModel<'a> {
    document: &'a ParsedDocument,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> DocumentModel<'a> {
    /// Creates a document model wrapper.
    #[must_use]
    pub fn new(document: &'a ParsedDocument) -> Self {
        Self {
            document,
            object_type: ObjectTypeName::unchecked("document"),
            supertypes: Vec::new(),
            links: vec![LinkName(Identifier::unchecked("catalog"))],
        }
    }
}

impl ModelObject for DocumentModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: String::from("document"),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("root")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "headerOffset" => Ok(ModelValue::Number(u64_to_f64(header_offset(
                self.document,
            ))?)),
            "postEOFDataSize" => Ok(ModelValue::Number(u64_to_f64(post_eof_data_size(
                self.document,
            ))?)),
            "header" => Ok(ModelValue::String(BoundedText::new(
                format!(
                    "%PDF-{}.{}",
                    self.document.version.major, self.document.version.minor
                ),
                32,
            )?)),
            "encrypted" | "isEncrypted" => Ok(ModelValue::Bool(self.document.is_encrypted())),
            "hasCatalog" => Ok(ModelValue::Bool(self.document.catalog.is_some())),
            "containsXRefStream" => Ok(ModelValue::Bool(contains_xref_stream(self.document))),
            "nrIndirects" => Ok(ModelValue::Number(usize_to_f64(
                self.document.objects.len(),
            )?)),
            "containsPDFUAIdentification" => Ok(ModelValue::Bool(contains_xmp_family(
                self.document,
                "pdfua",
            ))),
            "containsPDFAIdentification" => {
                Ok(ModelValue::Bool(contains_xmp_family(self.document, "pdfa")))
            }
            "part" => Ok(ModelValue::Number(0.0)),
            "partPrefix" | "rev" | "revPrefix" => Ok(ModelValue::Null),
            _ => Err(crate::ProfileError::UnknownProperty {
                property: BoundedText::unchecked(name.as_str()),
            }
            .into()),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut objects = Vec::new();
        if let Some(catalog) = graph.catalog() {
            push_linked(&mut objects, ModelObjectRef::Catalog(catalog), max_objects)?;
        }
        if graph.materializes("stream") {
            graph.push_streams(&mut objects, max_objects)?;
        }
        if graph.materializes_generic_roots() {
            graph.push_generic_roots(&mut objects, max_objects)?;
        }
        Ok(objects)
    }
}

/// Catalog model wrapper.
#[derive(Clone, Debug)]
pub struct CatalogModel<'a> {
    document: &'a ParsedDocument,
    key: ObjectKey,
    offset: u64,
    metadata: Option<ObjectKey>,
    pages: Option<ObjectKey>,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> CatalogModel<'a> {
    fn new(document: &'a ParsedDocument, key: ObjectKey) -> Option<Self> {
        let object = document.objects.get(&key)?;
        let dictionary = object.object.as_dictionary()?;
        let metadata = match dictionary.get("Metadata") {
            Some(crate::CosObject::Reference(key)) => Some(*key),
            _ => None,
        };
        let pages = match dictionary.get("Pages") {
            Some(crate::CosObject::Reference(key)) => Some(*key),
            _ => None,
        };
        Some(Self {
            document,
            key,
            offset: object.offset,
            metadata,
            pages,
            object_type: ObjectTypeName::unchecked("catalog"),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: vec![LinkName(Identifier::unchecked("metadata"))],
        })
    }
}

impl ModelObject for CatalogModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("catalog:{}:{}", self.key.number, self.key.generation),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("catalog")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "hasMetadata" => Ok(ModelValue::Bool(self.metadata.is_some())),
            "hasAcroForm" | "containsAcroForm" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("AcroForm"))
                    .is_some(),
            )),
            "hasStructTreeRoot" | "containsStructTreeRoot" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("StructTreeRoot"))
                    .is_some(),
            )),
            "hasOCProperties" | "containsOCProperties" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("OCProperties"))
                    .is_some(),
            )),
            "hasLang" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("Lang"))
                    .is_some(),
            )),
            "hasOutlines" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("Outlines"))
                    .is_some(),
            )),
            "hasNames" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("Names"))
                    .is_some(),
            )),
            "hasDests" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("Dests"))
                    .is_some(),
            )),
            "Marked" => Ok(ModelValue::Bool(false)),
            _ => self
                .document
                .objects
                .get(&self.key)
                .and_then(|object| object.object.as_dictionary())
                .map_or_else(
                    || unknown_property(name),
                    |dictionary| dictionary_property(dictionary, name, CATALOG_DIRECT_PROPERTIES),
                ),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut objects = graph
            .metadata(self)
            .map(ModelObjectRef::Metadata)
            .into_iter()
            .collect::<Vec<_>>();
        if objects.len() > max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let mut output_intents =
            graph.output_intents(self, max_objects.saturating_sub(objects.len()))?;
        output_intents.reverse();
        for output_intent in output_intents {
            push_linked(
                &mut objects,
                ModelObjectRef::OutputIntent(output_intent),
                max_objects,
            )?;
        }
        let mut pages = graph.pages(self, max_objects.saturating_sub(objects.len()))?;
        pages.reverse();
        for page in pages {
            push_linked(&mut objects, ModelObjectRef::Page(page), max_objects)?;
        }
        Ok(objects)
    }
}

/// Metadata stream model wrapper.
#[derive(Clone, Debug)]
pub struct MetadataModel<'a> {
    document: &'a ParsedDocument,
    key: ObjectKey,
    offset: u64,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> MetadataModel<'a> {
    fn new(document: &'a ParsedDocument, key: Option<ObjectKey>) -> Option<Self> {
        let key = key?;
        let object = document.objects.get(&key)?;
        if !matches!(object.object, crate::CosObject::Stream(_)) {
            return None;
        }
        Some(Self {
            document,
            key,
            offset: object.offset,
            object_type: ObjectTypeName::unchecked("metadata"),
            supertypes: vec![
                ObjectTypeName::unchecked("stream"),
                ObjectTypeName::unchecked("object"),
            ],
            links: Vec::new(),
        })
    }
}

impl ModelObject for MetadataModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("metadata:{}:{}", self.key.number, self.key.generation),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("metadata")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "present" | "catalogMetadata" => Ok(ModelValue::Bool(true)),
            "containsPDFAIdentification" => {
                Ok(ModelValue::Bool(contains_xmp_family(self.document, "pdfa")))
            }
            "containsPDFUAIdentification" => Ok(ModelValue::Bool(contains_xmp_family(
                self.document,
                "pdfua",
            ))),
            "part" => Ok(ModelValue::Number(xmp_part(self.document).unwrap_or(0.0))),
            "partPrefix" => Ok(ModelValue::String(BoundedText::unchecked(
                xmp_prefix_for_claim(self.document).unwrap_or("pdfaid"),
            ))),
            "conformance" => Ok(
                xmp_conformance(self.document).map_or(ModelValue::Null, |value| {
                    ModelValue::String(BoundedText::unchecked(value))
                }),
            ),
            "conformancePrefix" | "revPrefix" | "amdPrefix" | "corrPrefix" => {
                Ok(ModelValue::String(BoundedText::unchecked("pdfaid")))
            }
            "rev" => Ok(ModelValue::Null),
            "declarations" => Ok(ModelValue::List(xmp_declarations(self.document))),
            _ => self.document.objects.get(&self.key).map_or_else(
                || unknown_property(name),
                |object| match &object.object {
                    crate::CosObject::Stream(stream) => {
                        dictionary_property(&stream.dictionary, name, METADATA_DIRECT_PROPERTIES)
                    }
                    _ => unknown_property(name),
                },
            ),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

/// Page dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct PageModel<'a> {
    document: &'a ParsedDocument,
    key: ObjectKey,
    offset: u64,
    ordinal: usize,
    dictionary: &'a crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> PageModel<'a> {
    fn from_catalog(
        document: &'a ParsedDocument,
        catalog: &CatalogModel<'_>,
        limits: &ResourceLimits,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let Some(pages_root) = catalog.pages else {
            return Ok(Vec::new());
        };
        let mut stack = vec![pages_root];
        let mut pages = Vec::new();
        let mut visited = HashSet::new();
        while let Some(key) = stack.pop() {
            if !visited.insert(key) {
                continue;
            }
            let Some(object) = document.objects.get(&key) else {
                continue;
            };
            let Some(dictionary) = object.object.as_dictionary() else {
                continue;
            };
            match dictionary.get("Type") {
                Some(crate::CosObject::Name(name)) if name.matches("Page") => {
                    if pages.len() >= max_objects {
                        return Err(ValidationError::LimitExceeded {
                            limit: "max_objects",
                        }
                        .into());
                    }
                    pages.push(Self {
                        document,
                        key,
                        offset: object.offset,
                        ordinal: pages.len(),
                        dictionary,
                        object_type: ObjectTypeName::unchecked("page"),
                        supertypes: vec![ObjectTypeName::unchecked("object")],
                        links: vec![
                            LinkName(Identifier::unchecked("fonts")),
                            LinkName(Identifier::unchecked("annotations")),
                            LinkName(Identifier::unchecked("contentStreams")),
                        ],
                    });
                }
                _ => {
                    for kid in object_refs_from_array(dictionary.get("Kids"))
                        .into_iter()
                        .rev()
                    {
                        stack.push(kid);
                    }
                }
            }
            if u64::try_from(visited.len()).map_err(|_| ValidationError::LimitExceeded {
                limit: "max_objects",
            })? > limits.max_objects
            {
                return Err(ValidationError::LimitExceeded {
                    limit: "max_objects",
                }
                .into());
            }
        }
        Ok(pages)
    }
}

impl ModelObject for PageModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("page:{}:{}", self.key.number, self.key.generation),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("page")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "hasContents" => Ok(ModelValue::Bool(self.dictionary.get("Contents").is_some())),
            "hasResources" => Ok(ModelValue::Bool(self.dictionary.get("Resources").is_some())),
            "annotationCount" => Ok(ModelValue::Number(usize_to_f64(
                object_refs_or_direct_count(self.dictionary.get("Annots")),
            )?)),
            _ => dictionary_property(self.dictionary, name, PAGE_DIRECT_PROPERTIES),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut objects = Vec::new();
        let mut content_streams = graph.content_streams(self, max_objects)?;
        content_streams.reverse();
        for content_stream in content_streams {
            push_linked(
                &mut objects,
                ModelObjectRef::ContentStream(content_stream),
                max_objects,
            )?;
        }
        let mut annotations = graph.annotations(self, max_objects.saturating_sub(objects.len()))?;
        annotations.reverse();
        for annotation in annotations {
            push_linked(
                &mut objects,
                ModelObjectRef::Annotation(annotation),
                max_objects,
            )?;
        }
        let mut fonts = graph.fonts(self, max_objects.saturating_sub(objects.len()))?;
        fonts.reverse();
        for font in fonts {
            push_linked(&mut objects, ModelObjectRef::Font(font), max_objects)?;
        }
        Ok(objects)
    }
}

/// Font dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct FontModel<'a> {
    document: &'a ParsedDocument,
    page_ordinal: usize,
    key: Option<ObjectKey>,
    offset: Option<u64>,
    name: PdfName,
    dictionary: &'a crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> FontModel<'a> {
    fn from_page(
        document: &'a ParsedDocument,
        page: &PageModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let mut fonts = Vec::new();
        let Some(page_dictionary) = page_dictionary(document, page.key) else {
            return Ok(fonts);
        };
        let Some(resources) = resolve_dictionary_value(document, page_dictionary.get("Resources"))
        else {
            return Ok(fonts);
        };
        let Some(crate::CosObject::Dictionary(fonts_dictionary)) = resources.get("Font") else {
            return Ok(fonts);
        };
        for (name, value) in fonts_dictionary.iter() {
            if let Some((key, offset, dictionary)) = resolve_named_dictionary(document, value) {
                if fonts.len() >= max_objects {
                    return Err(ValidationError::LimitExceeded {
                        limit: "max_objects",
                    }
                    .into());
                }
                fonts.push(Self {
                    document,
                    page_ordinal: page.ordinal,
                    key,
                    offset,
                    name: name.clone(),
                    dictionary,
                    object_type: ObjectTypeName::unchecked("font"),
                    supertypes: vec![ObjectTypeName::unchecked("object")],
                    links: Vec::new(),
                });
            }
        }
        Ok(fonts)
    }
}

impl ModelObject for FontModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "font:{}:{}",
                self.page_ordinal,
                String::from_utf8_lossy(self.name.as_bytes())
            ),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("font")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "embedded" => Ok(ModelValue::Bool(
                self.dictionary.get("FontDescriptor").is_some(),
            )),
            "hasSubtype" => Ok(ModelValue::Bool(self.dictionary.get("Subtype").is_some())),
            _ => dictionary_property(self.dictionary, name, FONT_DIRECT_PROPERTIES),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

/// Annotation dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct AnnotationModel<'a> {
    document: &'a ParsedDocument,
    page_ordinal: usize,
    ordinal: usize,
    key: Option<ObjectKey>,
    offset: Option<u64>,
    dictionary: &'a crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> AnnotationModel<'a> {
    fn from_page(
        document: &'a ParsedDocument,
        page: &PageModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let mut annotations = Vec::new();
        let Some(page_dictionary) = page_dictionary(document, page.key) else {
            return Ok(annotations);
        };
        for (ordinal, value) in array_values(page_dictionary.get("Annots")).enumerate() {
            if let Some((key, offset, dictionary)) = resolve_named_dictionary(document, value) {
                if annotations.len() >= max_objects {
                    return Err(ValidationError::LimitExceeded {
                        limit: "max_objects",
                    }
                    .into());
                }
                annotations.push(Self {
                    document,
                    page_ordinal: page.ordinal,
                    ordinal,
                    key,
                    offset,
                    dictionary,
                    object_type: ObjectTypeName::unchecked("annotation"),
                    supertypes: vec![ObjectTypeName::unchecked("object")],
                    links: Vec::new(),
                });
            }
        }
        Ok(annotations)
    }
}

impl ModelObject for AnnotationModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("annotation:{}:{}", self.page_ordinal, self.ordinal),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("annotation")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "hasSubtype" => Ok(ModelValue::Bool(self.dictionary.get("Subtype").is_some())),
            _ => dictionary_property(self.dictionary, name, ANNOTATION_DIRECT_PROPERTIES),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

/// Output intent dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct OutputIntentModel<'a> {
    document: &'a ParsedDocument,
    ordinal: usize,
    key: Option<ObjectKey>,
    offset: Option<u64>,
    dictionary: &'a crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> OutputIntentModel<'a> {
    fn from_catalog(
        document: &'a ParsedDocument,
        catalog: &CatalogModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let Some(catalog_object) = document.objects.get(&catalog.key) else {
            return Ok(Vec::new());
        };
        let Some(catalog_dictionary) = catalog_object.object.as_dictionary() else {
            return Ok(Vec::new());
        };
        let mut output_intents = Vec::new();
        for (ordinal, value) in array_values(catalog_dictionary.get("OutputIntents")).enumerate() {
            if let Some((key, offset, dictionary)) = resolve_named_dictionary(document, value) {
                if output_intents.len() >= max_objects {
                    return Err(ValidationError::LimitExceeded {
                        limit: "max_objects",
                    }
                    .into());
                }
                output_intents.push(Self {
                    document,
                    ordinal,
                    key,
                    offset,
                    dictionary,
                    object_type: ObjectTypeName::unchecked("outputIntent"),
                    supertypes: vec![ObjectTypeName::unchecked("object")],
                    links: Vec::new(),
                });
            }
        }
        Ok(output_intents)
    }
}

impl ModelObject for OutputIntentModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("outputIntent:{}", self.ordinal),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("outputIntent")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "hasDestOutputProfile" => Ok(ModelValue::Bool(
                self.dictionary.get("DestOutputProfile").is_some(),
            )),
            _ => dictionary_property(self.dictionary, name, OUTPUT_INTENT_DIRECT_PROPERTIES),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

/// Page content stream model wrapper.
#[derive(Clone, Debug)]
pub struct ContentStreamModel<'a> {
    document: &'a ParsedDocument,
    page_ordinal: usize,
    ordinal: usize,
    key: ObjectKey,
    offset: u64,
    stream: &'a crate::StreamObject,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> ContentStreamModel<'a> {
    fn from_page(
        document: &'a ParsedDocument,
        page: &PageModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let mut streams = Vec::new();
        let Some(page_dictionary) = page_dictionary(document, page.key) else {
            return Ok(streams);
        };
        push_content_streams_from_value(
            document,
            page,
            page_dictionary.get("Contents"),
            max_objects,
            &mut streams,
        )?;
        Ok(streams)
    }
}

impl ModelObject for ContentStreamModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "contentStream:{}:{}:{}",
                self.page_ordinal, self.key.number, self.key.generation
            ),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("contentStream")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "lengthMatches" => {
                Ok(ModelValue::Bool(self.stream.declared_length.is_none_or(
                    |declared| declared == self.stream.discovered_length,
                )))
            }
            "declaredLength" => Ok(ModelValue::Number(u64_to_f64(
                self.stream
                    .declared_length
                    .unwrap_or(self.stream.discovered_length),
            )?)),
            "discoveredLength" => Ok(ModelValue::Number(u64_to_f64(
                self.stream.discovered_length,
            )?)),
            _ => dictionary_property(&self.stream.dictionary, name, STREAM_DIRECT_PROPERTIES),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

fn resolve_dictionary_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a crate::CosObject>,
) -> Option<&'a crate::Dictionary> {
    match value {
        Some(crate::CosObject::Dictionary(dictionary)) => Some(dictionary),
        Some(crate::CosObject::Reference(key)) => document.objects.get(key)?.object.as_dictionary(),
        _ => None,
    }
}

fn page_dictionary(document: &ParsedDocument, key: ObjectKey) -> Option<&crate::Dictionary> {
    document.objects.get(&key)?.object.as_dictionary()
}

fn resolve_named_dictionary<'a>(
    document: &'a ParsedDocument,
    value: &'a crate::CosObject,
) -> Option<(Option<ObjectKey>, Option<u64>, &'a crate::Dictionary)> {
    match value {
        crate::CosObject::Dictionary(dictionary) => Some((None, None, dictionary)),
        crate::CosObject::Reference(key) => {
            let object = document.objects.get(key)?;
            let dictionary = object.object.as_dictionary()?;
            Some((Some(*key), Some(object.offset), dictionary))
        }
        _ => None,
    }
}

fn object_refs_from_array(value: Option<&crate::CosObject>) -> Vec<ObjectKey> {
    match value {
        Some(crate::CosObject::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                crate::CosObject::Reference(key) => Some(*key),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn push_content_streams_from_value<'a>(
    document: &'a ParsedDocument,
    page: &PageModel<'_>,
    value: Option<&crate::CosObject>,
    max_objects: usize,
    streams: &mut Vec<ContentStreamModel<'a>>,
) -> Result<()> {
    match value {
        Some(crate::CosObject::Reference(key)) => {
            push_content_stream(document, page, *key, 0, max_objects, streams)?;
        }
        Some(crate::CosObject::Array(values)) => {
            let mut ordinal = 0_usize;
            for value in values {
                let crate::CosObject::Reference(key) = value else {
                    continue;
                };
                push_content_stream(document, page, *key, ordinal, max_objects, streams)?;
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or(ValidationError::LimitExceeded {
                        limit: "max_objects",
                    })?;
            }
        }
        Some(_) | None => {}
    }
    Ok(())
}

fn push_content_stream<'a>(
    document: &'a ParsedDocument,
    page: &PageModel<'_>,
    key: ObjectKey,
    ordinal: usize,
    max_objects: usize,
    streams: &mut Vec<ContentStreamModel<'a>>,
) -> Result<()> {
    let Some(object) = document.objects.get(&key) else {
        return Ok(());
    };
    let crate::CosObject::Stream(stream) = &object.object else {
        return Ok(());
    };
    if streams.len() >= max_objects {
        return Err(ValidationError::LimitExceeded {
            limit: "max_objects",
        }
        .into());
    }
    streams.push(ContentStreamModel {
        document,
        page_ordinal: page.ordinal,
        ordinal,
        key,
        offset: object.offset,
        stream,
        object_type: ObjectTypeName::unchecked("contentStream"),
        supertypes: vec![
            ObjectTypeName::unchecked("stream"),
            ObjectTypeName::unchecked("object"),
        ],
        links: Vec::new(),
    });
    Ok(())
}

fn object_refs_or_direct_count(value: Option<&crate::CosObject>) -> usize {
    match value {
        Some(crate::CosObject::Array(values)) => values.len(),
        Some(_) => 1,
        None => 0,
    }
}

fn array_values(value: Option<&crate::CosObject>) -> impl Iterator<Item = &crate::CosObject> {
    value
        .and_then(|value| match value {
            crate::CosObject::Array(values) => Some(values.as_slice()),
            _ => None,
        })
        .into_iter()
        .flatten()
}

fn dictionary_property(
    dictionary: &crate::Dictionary,
    name: &PropertyName,
    allowed_names: &[&str],
) -> Result<ModelValue> {
    if !allowed_names.contains(&name.as_str()) {
        return unknown_property(name);
    }
    Ok(dictionary
        .get(name.as_str())
        .cloned()
        .map_or(ModelValue::Null, ModelValue::from))
}

fn unknown_property(name: &PropertyName) -> Result<ModelValue> {
    Err(crate::ProfileError::UnknownProperty {
        property: BoundedText::unchecked(name.as_str()),
    }
    .into())
}

/// Generic dictionary-backed model wrapper.
#[derive(Clone, Debug)]
pub struct GenericModel<'a> {
    document: &'a ParsedDocument,
    key: Option<ObjectKey>,
    offset: Option<u64>,
    dictionary: &'a crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
    allowed_properties: &'static [&'static str],
    context: String,
    ordinal: usize,
}

impl<'a> GenericModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        family: &'static str,
        key: Option<ObjectKey>,
        offset: Option<u64>,
        dictionary: &'a crate::Dictionary,
        ordinal: usize,
        context: impl Into<String>,
    ) -> Self {
        Self {
            document,
            key,
            offset,
            dictionary,
            object_type: ObjectTypeName::unchecked(family),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: Vec::new(),
            allowed_properties: family_direct_properties(family),
            context: context.into(),
            ordinal,
        }
    }
}

impl ModelObject for GenericModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("{}:{}", self.object_type.as_str(), self.ordinal),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some(&self.context)
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match (self.object_type.as_str(), name.as_str()) {
            ("image", "width") => dictionary_property(
                self.dictionary,
                &PropertyName::unchecked("Width"),
                IMAGE_DIRECT_PROPERTIES,
            ),
            ("image", "height") => dictionary_property(
                self.dictionary,
                &PropertyName::unchecked("Height"),
                IMAGE_DIRECT_PROPERTIES,
            ),
            ("contentStream", "operatorCount" | "markedContentCount") => {
                Ok(ModelValue::Number(0.0))
            }
            _ => dictionary_property(self.dictionary, name, self.allowed_properties),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

fn push_generic_model<'a>(
    models: &mut Vec<GenericModel<'a>>,
    model: GenericModel<'a>,
    max_objects: usize,
) -> Result<()> {
    if models.len() >= max_objects {
        return Err(ValidationError::LimitExceeded {
            limit: "max_objects",
        }
        .into());
    }
    models.push(model);
    Ok(())
}

fn family_direct_properties(family: &str) -> &'static [&'static str] {
    match family {
        "resource" => RESOURCE_DIRECT_PROPERTIES,
        "names" => NAMES_DIRECT_PROPERTIES,
        "outline" => OUTLINES_DIRECT_PROPERTIES,
        "destination" => DESTINATION_DIRECT_PROPERTIES,
        "acroForm" => ACRO_FORM_DIRECT_PROPERTIES,
        "optionalContentProperties" => OPTIONAL_CONTENT_DIRECT_PROPERTIES,
        "permissions" => PERMISSIONS_PROPERTIES,
        "cMap" => CMAP_DIRECT_PROPERTIES,
        "image" => IMAGE_DIRECT_PROPERTIES,
        "xObject" => XOBJECT_DIRECT_PROPERTIES,
        "action" => ACTION_DIRECT_PROPERTIES,
        "formField" => FORM_FIELD_DIRECT_PROPERTIES,
        "colorSpace" => COLOR_SPACE_DIRECT_PROPERTIES,
        "extGState" => EXT_GSTATE_DIRECT_PROPERTIES,
        "structureTreeRoot" => STRUCTURE_DIRECT_PROPERTIES,
        "structureElement" => STRUCTURE_ELEMENT_PROPERTIES,
        "signature" => SIGNATURE_DIRECT_PROPERTIES,
        "security" => SECURITY_DIRECT_PROPERTIES,
        "pageTree" => PAGE_TREE_PROPERTIES,
        _ => DIRECT_PROPERTY_NAMES,
    }
}

fn resolve_named_dictionary_from_option<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a crate::CosObject>,
) -> Option<(Option<ObjectKey>, Option<u64>, &'a crate::Dictionary)> {
    match value {
        Some(value) => resolve_named_dictionary(document, value),
        None => None,
    }
}

fn classify_xobject(dictionary: &crate::Dictionary) -> Option<&'static str> {
    match dictionary.get("Subtype") {
        Some(crate::CosObject::Name(name)) if name.matches("Image") => Some("image"),
        Some(crate::CosObject::Name(name)) if name.matches("Form") => Some("xObject"),
        _ => None,
    }
}

fn classify_dictionary(dictionary: &crate::Dictionary) -> Option<&'static str> {
    if let Some(crate::CosObject::Name(name)) = dictionary.get("Subtype") {
        if name.matches("Image") {
            return Some("image");
        }
        if name.matches("Form") {
            return Some("xObject");
        }
        if name.matches("Widget") {
            return Some("formField");
        }
    }
    if let Some(crate::CosObject::Name(name)) = dictionary.get("Type") {
        if name.matches("Pages") {
            return Some("pageTree");
        }
        if name.matches("Action") {
            return Some("action");
        }
        if name.matches("StructTreeRoot") {
            return Some("structureTreeRoot");
        }
        if name.matches("StructElem") {
            return Some("structureElement");
        }
        if name.matches("Sig") {
            return Some("signature");
        }
        if name.matches("EmbeddedFile") {
            return Some("embeddedFontFile");
        }
        if name.matches("OCProperties") {
            return Some("optionalContentProperties");
        }
        if name.matches("XObject") {
            return classify_xobject(dictionary).or(Some("xObject"));
        }
        if name.matches("Font") {
            return Some("font");
        }
        if name.matches("Annot") {
            return Some("annotation");
        }
        if name.matches("Metadata") {
            return Some("metadata");
        }
        if name.matches("OutputIntent") {
            return Some("outputIntent");
        }
        if name.matches("Filespec") {
            return Some("destination");
        }
    }
    if dictionary.get("Fields").is_some() {
        return Some("acroForm");
    }
    if dictionary.get("Filter").is_some() && dictionary.get("V").is_some() {
        return Some("security");
    }
    if dictionary.get("CMapName").is_some() {
        return Some("cMap");
    }
    if dictionary.get("ByteRange").is_some() {
        return Some("signature");
    }
    None
}

/// Stream model wrapper.
#[derive(Clone, Debug)]
pub struct StreamModel<'a> {
    document: &'a ParsedDocument,
    key: ObjectKey,
    offset: u64,
    stream: &'a crate::StreamObject,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> StreamModel<'a> {
    fn from_indirect_with_document(
        document: &'a ParsedDocument,
        object: &'a IndirectObject,
    ) -> Option<Self> {
        let crate::CosObject::Stream(stream) = &object.object else {
            return None;
        };
        Some(Self {
            document,
            key: object.key,
            offset: object.offset,
            stream,
            object_type: ObjectTypeName::unchecked("stream"),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: Vec::new(),
        })
    }
}

impl ModelObject for StreamModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!("stream:{}:{}", self.key.number, self.key.generation),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("stream")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "lengthMatches" => {
                Ok(ModelValue::Bool(self.stream.declared_length.is_none_or(
                    |declared| declared == self.stream.discovered_length,
                )))
            }
            "declaredLength" => Ok(ModelValue::Number(u64_to_f64(
                self.stream
                    .declared_length
                    .unwrap_or(self.stream.discovered_length),
            )?)),
            "discoveredLength" => Ok(ModelValue::Number(u64_to_f64(
                self.stream.discovered_length,
            )?)),
            "streamKeywordCRLFCompliant" => {
                Ok(ModelValue::Bool(self.stream.stream_keyword_crlf_compliant))
            }
            "endstreamKeywordEOLCompliant" => Ok(ModelValue::Bool(
                self.stream.endstream_keyword_eol_compliant,
            )),
            "F" | "FFilter" | "FDecodeParms" => Ok(self
                .stream
                .dictionary
                .get(name.as_str())
                .cloned()
                .map_or(ModelValue::Null, ModelValue::from)),
            _ => Err(crate::ProfileError::UnknownProperty {
                property: BoundedText::unchecked(name.as_str()),
            }
            .into()),
        }
    }

    fn links(&self) -> &[LinkName] {
        &self.links
    }

    fn linked_objects<'a>(
        &self,
        _graph: &ModelGraph<'a>,
        _max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

struct RuleIndex<'a> {
    by_type: BTreeMap<&'a str, Vec<&'a Rule>>,
}

impl<'a> RuleIndex<'a> {
    fn new(rules: &'a [Rule]) -> Self {
        let mut by_type: BTreeMap<&'a str, Vec<&'a Rule>> = BTreeMap::new();
        for rule in rules {
            by_type
                .entry(rule.object_type.as_str())
                .or_default()
                .push(rule);
        }
        Self { by_type }
    }

    fn rules_for(&self, object: &ModelObjectRef<'_>) -> Vec<&'a Rule> {
        let mut rules = self
            .by_type
            .get(object.object_type().as_str())
            .cloned()
            .unwrap_or_default();
        let supertypes = match object {
            ModelObjectRef::Document(model) => model.super_types(),
            ModelObjectRef::Catalog(model) => model.super_types(),
            ModelObjectRef::Metadata(model) => model.super_types(),
            ModelObjectRef::Page(model) => model.super_types(),
            ModelObjectRef::Font(model) => model.super_types(),
            ModelObjectRef::Annotation(model) => model.super_types(),
            ModelObjectRef::OutputIntent(model) => model.super_types(),
            ModelObjectRef::ContentStream(model) => model.super_types(),
            ModelObjectRef::Stream(model) => model.super_types(),
            ModelObjectRef::Generic(model) => model.super_types(),
        };
        for supertype in supertypes {
            if let Some(super_rules) = self.by_type.get(supertype.as_str()) {
                rules.extend(super_rules.iter().copied());
            }
        }
        rules
    }
}

struct ProfileState {
    profile: crate::ProfileIdentity,
    max_failed_assertions_per_rule: u32,
    record_passed_assertions: bool,
    checks_executed: u64,
    rules_executed: u64,
    failed_rules: u64,
    failed_assertions: Vec<Assertion>,
    passed_assertions: Vec<Assertion>,
    unsupported_rules: Vec<UnsupportedRule>,
    retained_failures_by_rule: HashMap<RuleId, u32>,
    next_ordinal: u64,
}

impl ProfileState {
    fn new(
        profile: crate::ProfileIdentity,
        max_failed_assertions_per_rule: u32,
        record_passed_assertions: bool,
    ) -> Self {
        Self {
            profile,
            max_failed_assertions_per_rule,
            record_passed_assertions,
            checks_executed: 0,
            rules_executed: 0,
            failed_rules: 0,
            failed_assertions: Vec::new(),
            passed_assertions: Vec::new(),
            unsupported_rules: Vec::new(),
            retained_failures_by_rule: HashMap::new(),
            next_ordinal: 1,
        }
    }

    fn apply_rule(
        &mut self,
        object: &ModelObjectRef<'_>,
        rule: &Rule,
        evaluator: &mut DefaultRuleEvaluator,
    ) -> Result<()> {
        self.rules_executed =
            self.rules_executed
                .checked_add(1)
                .ok_or(ValidationError::LimitExceeded {
                    limit: "rules_executed",
                })?;
        self.checks_executed =
            self.checks_executed
                .checked_add(1)
                .ok_or(ValidationError::LimitExceeded {
                    limit: "checks_executed",
                })?;
        let outcome = match evaluator.evaluate(object.clone(), rule) {
            Ok(outcome) => outcome,
            Err(PdfvError::Profile(error)) => {
                self.unsupported_rules.push(UnsupportedRule {
                    profile_id: self.profile.id.clone(),
                    rule_id: rule.id.clone(),
                    expression_fragment: Some(BoundedText::unchecked(format!("{:?}", rule.test))),
                    reason: BoundedText::new(error.to_string(), 512)?,
                    references: rule.references.clone(),
                });
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        match outcome {
            RuleOutcome::Passed if self.record_passed_assertions => {
                let assertion = self.assertion(object, rule, outcome)?;
                self.passed_assertions.push(assertion);
            }
            RuleOutcome::Passed => {}
            RuleOutcome::Failed => {
                self.failed_rules =
                    self.failed_rules
                        .checked_add(1)
                        .ok_or(ValidationError::LimitExceeded {
                            limit: "failed_rules",
                        })?;
                let retained = self
                    .retained_failures_by_rule
                    .get(&rule.id)
                    .copied()
                    .unwrap_or(0);
                if retained < self.max_failed_assertions_per_rule {
                    let assertion = self.assertion(object, rule, outcome)?;
                    self.failed_assertions.push(assertion);
                    self.retained_failures_by_rule
                        .insert(rule.id.clone(), retained.saturating_add(1));
                }
            }
        }
        Ok(())
    }

    fn register_static_unsupported_rules(&mut self, rules: &[Rule]) {
        for rule in rules {
            if let crate::RuleExpr::Unsupported { fragment, reason } = &rule.test {
                self.unsupported_rules.push(UnsupportedRule {
                    profile_id: self.profile.id.clone(),
                    rule_id: rule.id.clone(),
                    expression_fragment: Some(fragment.clone()),
                    reason: reason.clone(),
                    references: rule.references.clone(),
                });
            }
        }
    }

    fn assertion(
        &mut self,
        object: &ModelObjectRef<'_>,
        rule: &Rule,
        outcome: RuleOutcome,
    ) -> Result<Assertion> {
        let ordinal = NonZeroU64::new(self.next_ordinal).ok_or(ValidationError::LimitExceeded {
            limit: "assertion_ordinal",
        })?;
        self.next_ordinal =
            self.next_ordinal
                .checked_add(1)
                .ok_or(ValidationError::LimitExceeded {
                    limit: "assertion_ordinal",
                })?;
        Ok(Assertion {
            ordinal,
            rule_id: rule.id.clone(),
            status: outcome.assertion_status(),
            description: rule.description.clone(),
            location: object.location(),
            object_context: Some(object.context()),
            message: Some(rule.error.message.clone()),
            error_arguments: Vec::<ErrorArgument>::new(),
        })
    }

    fn finish(self) -> ProfileReport {
        ProfileReport::builder()
            .profile(self.profile)
            .is_compliant(self.failed_rules == 0 && self.unsupported_rules.is_empty())
            .checks_executed(self.checks_executed)
            .rules_executed(self.rules_executed)
            .failed_rules(self.failed_rules)
            .failed_assertions(self.failed_assertions)
            .passed_assertions(self.passed_assertions)
            .unsupported_rules(self.unsupported_rules)
            .build()
    }
}

fn reader_len<R: Read + Seek>(reader: &mut R) -> Result<Option<u64>> {
    let current = reader
        .stream_position()
        .map_err(|source| PdfvError::Io { path: None, source })?;
    let end = reader
        .seek(SeekFrom::End(0))
        .map_err(|source| PdfvError::Io { path: None, source })?;
    reader
        .seek(SeekFrom::Start(current))
        .map_err(|source| PdfvError::Io { path: None, source })?;
    Ok(Some(end))
}

fn parse_failed_report(
    source: InputSummary,
    error: &crate::ParseError,
    elapsed: std::time::Duration,
) -> Result<ValidationReport> {
    Ok(ValidationReport::builder()
        .engine_version(ENGINE_VERSION.to_owned())
        .source(source)
        .status(ValidationStatus::ParseFailed)
        .flavours(Vec::new())
        .profile_reports(Vec::new())
        .parse_facts(Vec::new())
        .warnings(vec![crate::ValidationWarning::General {
            message: BoundedText::new(error.to_string(), 512)?,
        }])
        .task_durations(vec![TaskDuration::from_duration(
            Identifier::new("parse")?,
            elapsed,
        )])
        .build())
}

fn base_report(
    source: InputSummary,
    status: ValidationStatus,
    profile_reports: Vec<ProfileReport>,
    parsed: ParsedDocument,
    elapsed: std::time::Duration,
) -> Result<ValidationReport> {
    Ok(ValidationReport::builder()
        .engine_version(ENGINE_VERSION.to_owned())
        .source(source)
        .status(status)
        .flavours(Vec::new())
        .profile_reports(profile_reports)
        .parse_facts(parsed.parse_facts)
        .warnings(parsed.warnings)
        .task_durations(vec![TaskDuration::from_duration(
            Identifier::new("validate")?,
            elapsed,
        )])
        .build())
}

fn header_offset(document: &ParsedDocument) -> u64 {
    document
        .parse_facts
        .iter()
        .find_map(|fact| match fact {
            crate::ParseFact::Header { offset, .. } => Some(*offset),
            _ => None,
        })
        .unwrap_or(0)
}

fn post_eof_data_size(document: &ParsedDocument) -> u64 {
    document
        .parse_facts
        .iter()
        .find_map(|fact| match fact {
            crate::ParseFact::PostEofData { bytes } => Some(*bytes),
            _ => None,
        })
        .unwrap_or(0)
}

fn contains_xref_stream(document: &ParsedDocument) -> bool {
    document.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            crate::ParseFact::Xref {
                fact: crate::XrefFact::XrefStreamParsed { .. }
                    | crate::XrefFact::XrefStreamUnsupported,
                ..
            }
        )
    })
}

fn contains_xmp_family(document: &ParsedDocument, family: &str) -> bool {
    document.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            crate::ParseFact::Xmp {
                fact:
                    crate::XmpFact::FlavourClaim {
                        family: claim_family,
                        ..
                    },
                ..
            } if claim_family.as_str() == family
        )
    })
}

fn xmp_part(document: &ParsedDocument) -> Option<f64> {
    document.parse_facts.iter().find_map(|fact| {
        let crate::ParseFact::Xmp {
            fact:
                crate::XmpFact::FlavourClaim {
                    family,
                    display_flavour,
                    ..
                },
            ..
        } = fact
        else {
            return None;
        };
        if family.as_str() == "pdfa" || family.as_str() == "pdfua" {
            display_flavour
                .as_str()
                .split('-')
                .nth(1)
                .and_then(|value| value.chars().next())
                .and_then(|character| character.to_digit(10))
                .map(f64::from)
        } else {
            None
        }
    })
}

fn xmp_prefix_for_claim(document: &ParsedDocument) -> Option<&'static str> {
    document.parse_facts.iter().find_map(|fact| {
        let crate::ParseFact::Xmp {
            fact: crate::XmpFact::FlavourClaim { family, .. },
            ..
        } = fact
        else {
            return None;
        };
        match family.as_str() {
            "pdfa" => Some("pdfaid"),
            "pdfua" => Some("pdfuaid"),
            _ => None,
        }
    })
}

fn xmp_conformance(document: &ParsedDocument) -> Option<String> {
    document.parse_facts.iter().find_map(|fact| {
        let crate::ParseFact::Xmp {
            fact:
                crate::XmpFact::FlavourClaim {
                    family,
                    display_flavour,
                    ..
                },
            ..
        } = fact
        else {
            return None;
        };
        if family.as_str() != "pdfa" {
            return None;
        }
        display_flavour
            .as_str()
            .chars()
            .last()
            .filter(char::is_ascii_alphabetic)
            .map(|character| character.to_ascii_uppercase().to_string())
    })
}

fn xmp_declarations(document: &ParsedDocument) -> Vec<ModelValue> {
    document
        .parse_facts
        .iter()
        .filter_map(|fact| {
            let crate::ParseFact::Xmp {
                fact:
                    crate::XmpFact::FlavourClaim {
                        family,
                        display_flavour,
                        ..
                    },
                ..
            } = fact
            else {
                return None;
            };
            if family.as_str() != "wtpdf" {
                return None;
            }
            let declaration = match display_flavour.as_str() {
                "wtpdf-1-0-accessibility" => "http://pdfa.org/declarations/wtpdf#accessibility1.0",
                "wtpdf-1-0-reuse" => "http://pdfa.org/declarations/wtpdf#reuse1.0",
                _ => return None,
            };
            Some(ModelValue::String(BoundedText::unchecked(declaration)))
        })
        .collect()
}

fn u64_to_f64(value: u64) -> Result<f64> {
    let bounded = u32::try_from(value).map_err(|_| ValidationError::LimitExceeded {
        limit: "numeric_property",
    })?;
    Ok(f64::from(bounded))
}

fn usize_to_f64(value: usize) -> Result<f64> {
    let bounded = u32::try_from(value).map_err(|_| ValidationError::LimitExceeded {
        limit: "numeric_property",
    })?;
    Ok(f64::from(bounded))
}

fn remaining_object_budget(
    limits: &ResourceLimits,
    visited_len: usize,
    stack_len: usize,
) -> Result<usize> {
    let visited = u64::try_from(visited_len).map_err(|_| ValidationError::LimitExceeded {
        limit: "max_objects",
    })?;
    let pending = u64::try_from(stack_len).map_err(|_| ValidationError::LimitExceeded {
        limit: "max_objects",
    })?;
    let consumed = visited
        .checked_add(pending)
        .ok_or(ValidationError::LimitExceeded {
            limit: "max_objects",
        })?;
    let remaining =
        limits
            .max_objects
            .checked_sub(consumed)
            .ok_or(ValidationError::LimitExceeded {
                limit: "max_objects",
            })?;
    usize::try_from(remaining).map_err(|_| {
        ValidationError::LimitExceeded {
            limit: "max_objects",
        }
        .into()
    })
}

fn push_linked<'a>(
    objects: &mut Vec<ModelObjectRef<'a>>,
    object: ModelObjectRef<'a>,
    max_objects: usize,
) -> Result<()> {
    if objects.len() >= max_objects {
        return Err(ValidationError::LimitExceeded {
            limit: "max_objects",
        }
        .into());
    }
    objects.push(object);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use super::{
        AnnotationModel, CatalogModel, ContentStreamModel, FontModel, OutputIntentModel, PageModel,
    };
    use crate::{
        BinaryOp, BoundedText, ErrorTemplate, FlavourSelection, Identifier, ModelObject,
        ModelObjectRef, ModelValue, Parser, PdfvError, ProfileIdentity, ProfileRepository,
        PropertyName, ResourceLimits, Rule, RuleExpr, RuleId, ValidationFlavour, ValidationOptions,
        ValidationProfile, Validator,
    };

    #[derive(Debug)]
    struct StaticRepo(ValidationProfile);

    impl ProfileRepository for StaticRepo {
        fn profiles_for(
            &self,
            _selection: &FlavourSelection,
        ) -> crate::Result<Vec<ValidationProfile>> {
            Ok(vec![self.0.clone()])
        }
    }

    fn m1_model_pdf() -> &'static [u8] {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /OutputIntents [8 0 R] >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Annots [5 0 R] /Contents 6 0 R >>
endobj
4 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
5 0 obj
<< /Type /Annot /Subtype /Text >>
endobj
6 0 obj
<< /Length 3 >>
stream
q Q
endstream
endobj
7 0 obj
<< /Length 0 >>
stream
endstream
endobj
8 0 obj
<< /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 7 0 R >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
    }

    fn m6_model_pdf() -> &'static [u8] {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R /StructTreeRoot 8 0 R /OCProperties 9 0 R /Names 10 0 R /Outlines 11 0 R /Perms 12 0 R /Dests [21 0 R] >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 5 0 R /Fm1 22 0 R >> /ColorSpace << /CS1 13 0 R >> /ExtGState << /GS1 14 0 R >> >> /Annots [6 0 R] /Contents 15 0 R >>
endobj
4 0 obj
<< /Type /Font /Subtype /Type0 /BaseFont /Faux /ToUnicode 16 0 R /FontDescriptor << /FontFile2 20 0 R >> >>
endobj
5 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
6 0 obj
<< /Type /Annot /Subtype /Widget /FT /Sig /A 17 0 R >>
endobj
7 0 obj
<< /Fields [6 0 R] /SigFlags 3 >>
endobj
8 0 obj
<< /Type /StructTreeRoot /K 18 0 R /RoleMap << /H1 /H >> >>
endobj
9 0 obj
<< /OCGs [] /D << >> >>
endobj
10 0 obj
<< /Dests << /Names [] >> >>
endobj
11 0 obj
<< /Type /Outlines /Count 0 >>
endobj
12 0 obj
<< /DocMDP 19 0 R >>
endobj
13 0 obj
<< /N 3 /Alternate /DeviceRGB >>
endobj
14 0 obj
<< /Type /ExtGState /BM /Normal /CA 1 >>
endobj
15 0 obj
<< /Length 3 >>
stream
q Q
endstream
endobj
16 0 obj
<< /Type /CMap /CMapName /Identity-H >>
endobj
17 0 obj
<< /Type /Action /S /URI /URI (https://example.invalid) >>
endobj
18 0 obj
<< /Type /StructElem /S /Document /K [] >>
endobj
19 0 obj
<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 0 0 0] >>
endobj
20 0 obj
<< /Type /EmbeddedFile /Length 0 >>
stream
endstream
endobj
21 0 obj
<< /D [3 0 R /Fit] >>
endobj
22 0 obj
<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 0 >>
stream
endstream
endobj
23 0 obj
<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
    }

    #[test]
    fn test_should_materialize_m1_model_wrappers() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m1_model_pdf()))?;
        let catalog_key = document.catalog.ok_or(crate::ParseError::MissingObject {
            message: crate::BoundedText::unchecked("missing catalog"),
        })?;
        let catalog =
            CatalogModel::new(&document, catalog_key).ok_or(crate::ParseError::MissingObject {
                message: crate::BoundedText::unchecked("missing catalog model"),
            })?;

        let pages =
            PageModel::from_catalog(&document, &catalog, &crate::ResourceLimits::default(), 16)?;
        let page = pages.first().ok_or(crate::ParseError::MissingObject {
            message: crate::BoundedText::unchecked("missing page"),
        })?;
        let fonts = FontModel::from_page(&document, page, 16)?;
        let annotations = AnnotationModel::from_page(&document, page, 16)?;
        let output_intents = OutputIntentModel::from_catalog(&document, &catalog, 16)?;
        let content_streams = ContentStreamModel::from_page(&document, page, 16)?;

        assert_eq!(pages.len(), 1);
        assert_eq!(fonts.len(), 1);
        assert_eq!(annotations.len(), 1);
        assert_eq!(output_intents.len(), 1);
        assert_eq!(content_streams.len(), 1);
        assert_eq!(
            page.property(&PropertyName::new("hasContents")?)?,
            ModelValue::Bool(true)
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_m1_links_lazily_from_model_graph() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m1_model_pdf()))?;
        let limits = crate::ResourceLimits::default();
        let graph = super::ModelGraph::with_all_families(&document, &limits);
        let document_model = super::DocumentModel::new(&document);
        let mut stack = vec![ModelObjectRef::Document(document_model)];
        let mut visited_contexts = Vec::new();

        while let Some(object) = stack.pop() {
            visited_contexts.push(object.context().as_str().to_owned());
            for linked in object.linked_objects(&graph, 16)? {
                stack.push(linked);
            }
        }

        assert!(visited_contexts.iter().any(|value| value == "root/page[0]"));
        assert!(
            visited_contexts
                .iter()
                .any(|value| value == "root/page[0]/font[F1]")
        );
        assert!(
            visited_contexts
                .iter()
                .any(|value| value == "root/page[0]/annotation[0]")
        );
        assert!(
            visited_contexts
                .iter()
                .any(|value| value == "root/catalog[0]/outputIntent[0]")
        );
        assert!(
            visited_contexts
                .iter()
                .any(|value| value == "root/page[0]/contentStream[0]")
        );
        Ok(())
    }

    #[test]
    fn test_should_register_model_family_schema_for_generated_profiles() -> crate::Result<()> {
        let registry = super::ModelRegistry::default_registry();

        for family in [
            "document",
            "catalog",
            "page",
            "resource",
            "font",
            "cMap",
            "image",
            "contentStream",
            "annotation",
            "action",
            "formField",
            "colorSpace",
            "extGState",
            "structureTreeRoot",
            "structureElement",
            "signature",
            "security",
        ] {
            assert!(registry.has_family(&crate::ObjectTypeName::new(family)?));
        }
        assert!(registry.has_family_property(
            &crate::ObjectTypeName::new("structureElement")?,
            &PropertyName::new("parentStandardType")?
        ));
        Ok(())
    }

    #[test]
    fn test_should_materialize_m6_broad_model_families_bounded_iteratively() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m6_model_pdf()))?;
        let limits = crate::ResourceLimits {
            max_objects: 128,
            ..crate::ResourceLimits::default()
        };
        let graph = super::ModelGraph::with_all_families(&document, &limits);
        let mut stack = vec![ModelObjectRef::Document(super::DocumentModel::new(
            &document,
        ))];
        let mut visited = std::collections::HashSet::new();
        let mut families = std::collections::BTreeSet::new();

        while let Some(object) = stack.pop() {
            if !visited.insert(object.identity_key()) {
                continue;
            }
            families.insert(object.object_type().as_str().to_owned());
            for linked in object.linked_objects(&graph, 128)? {
                stack.push(linked);
            }
        }

        for family in [
            "acroForm",
            "structureTreeRoot",
            "optionalContentProperties",
            "names",
            "outline",
            "destination",
            "permissions",
            "pageTree",
            "resource",
            "image",
            "xObject",
            "colorSpace",
            "extGState",
            "cMap",
            "embeddedFontFile",
            "action",
            "signature",
            "security",
        ] {
            assert!(families.contains(family), "missing {family}: {families:?}");
        }
        Ok(())
    }

    #[test]
    fn test_should_validate_m1_linked_objects_through_lazy_traversal() -> crate::Result<()> {
        let profile = linked_object_profile()?;
        let validator =
            Validator::with_profiles(ValidationOptions::default(), Arc::new(StaticRepo(profile)))?;
        let report =
            validator.validate_reader(Cursor::new(m1_model_pdf()), crate::InputName::memory())?;
        let profile =
            report
                .profile_reports
                .first()
                .ok_or(crate::ValidationError::LimitExceeded {
                    limit: "profile_reports",
                })?;
        let contexts = profile
            .failed_assertions
            .iter()
            .filter_map(|assertion| assertion.object_context.as_ref())
            .map(BoundedText::as_str)
            .collect::<Vec<_>>();

        assert_eq!(profile.rules_executed, 5);
        assert!(contexts.contains(&"root/page[0]"));
        assert!(contexts.contains(&"root/page[0]/font[F1]"));
        assert!(contexts.contains(&"root/page[0]/annotation[0]"));
        assert!(contexts.contains(&"root/catalog[0]/outputIntent[0]"));
        assert!(contexts.contains(&"root/page[0]/contentStream[0]"));
        Ok(())
    }

    #[test]
    fn test_should_limit_lazy_link_expansion_before_enqueue() -> crate::Result<()> {
        let limits = ResourceLimits {
            max_objects: 1,
            ..ResourceLimits::default()
        };
        let options = ValidationOptions::builder().resource_limits(limits).build();
        let Err(error) = Validator::new(options)?.validate_reader(
            Cursor::new(simple_catalog_pdf()),
            crate::InputName::memory(),
        ) else {
            return Err(crate::ValidationError::LimitExceeded {
                limit: "expected_error",
            }
            .into());
        };

        assert!(matches!(
            error,
            PdfvError::Validation(crate::ValidationError::LimitExceeded {
                limit: "max_objects"
            })
        ));
        Ok(())
    }

    fn simple_catalog_pdf() -> &'static [u8] {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
    }

    fn linked_object_profile() -> crate::Result<ValidationProfile> {
        Ok(ValidationProfile {
            identity: ProfileIdentity {
                id: Identifier::new("lazy-links")?,
                name: BoundedText::new("lazy links", 64)?,
                version: None,
            },
            flavour: ValidationFlavour::new("pdfa", std::num::NonZeroU32::MIN, "b")?,
            rules: vec![
                false_rule("page-rule", "page", false)?,
                false_rule("font-rule", "font", false)?,
                false_rule("annotation-rule", "annotation", false)?,
                false_rule("output-intent-rule", "outputIntent", false)?,
                false_rule("content-stream-deferred", "contentStream", true)?,
            ],
        })
    }

    fn false_rule(id: &str, object_type: &str, deferred: bool) -> crate::Result<Rule> {
        Ok(Rule {
            id: RuleId(Identifier::new(id)?),
            object_type: crate::ObjectTypeName::new(object_type)?,
            deferred,
            tags: Vec::new(),
            description: BoundedText::new(id, 64)?,
            test: RuleExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(RuleExpr::Bool { value: true }),
                right: Box::new(RuleExpr::Bool { value: false }),
            },
            error: ErrorTemplate {
                message: BoundedText::new(id, 64)?,
            },
            references: Vec::new(),
        })
    }
}
