//! End-to-end validation session and M0 model graph.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use crate::{
    Assertion, BoundedText, BuiltinProfileRepository, ENGINE_VERSION, ErrorArgument, Identifier,
    IndirectObject, InputKind, InputSummary, ModelValue, ObjectKey, ObjectLocation, ObjectTypeName,
    ParsedDocument, Parser, PdfvError, ProfileReport, ProfileRepository, PropertyName,
    ResourceLimits, Result, Rule, RuleEvaluator, RuleId, RuleOutcome, TaskDuration,
    UnsupportedRule, ValidationError, ValidationOptions, ValidationReport, ValidationStatus,
    profile::DefaultRuleEvaluator,
};

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
    /// Creates a validator with the built-in M0 profile repository.
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
        let parsed = match parser.parse(source) {
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

        let profiles = self.profiles.profiles_for(&self.options.flavour)?;
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
        let graph = ModelGraph::new(&self.document);
        let document_model = DocumentModel::new(&self.document);
        let catalog_model = self
            .document
            .catalog
            .and_then(|key| CatalogModel::new(&self.document, key));
        let metadata_model = catalog_model
            .as_ref()
            .and_then(|catalog| MetadataModel::new(&self.document, catalog.metadata));
        let stream_models = self
            .document
            .objects
            .values()
            .filter_map(|object| StreamModel::from_indirect_with_document(&self.document, object))
            .collect::<Vec<_>>();
        let mut evaluator = DefaultRuleEvaluator::new(self.limits.clone());
        let mut state = ProfileState::new(
            profile.identity.clone(),
            self.max_failed_assertions_per_rule,
            self.record_passed_assertions,
        );
        let mut stack = Vec::from([ModelObjectRef::Document(&document_model)]);
        let mut visited = HashSet::new();
        let mut deferred = Vec::new();

        while let Some(object) = stack.pop() {
            let visited_key = object.identity_key();
            if !visited.insert(visited_key) {
                continue;
            }
            let object_rules = index.rules_for(object);
            for rule in object_rules {
                if rule.deferred {
                    deferred.push((object, rule));
                } else {
                    state.apply_rule(object, rule, &mut evaluator)?;
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
            for linked in object.linked_objects(
                &graph,
                catalog_model.as_ref(),
                metadata_model.as_ref(),
                &stream_models,
            )? {
                stack.push(linked);
            }
        }
        for (object, rule) in deferred {
            state.apply_rule(object, rule, &mut evaluator)?;
        }
        Ok(state.finish())
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
        &'a self,
        graph: &'a ModelGraph<'a>,
        catalog: Option<&'a CatalogModel<'a>>,
        metadata: Option<&'a MetadataModel<'a>>,
        streams: &'a [StreamModel<'a>],
    ) -> Result<Vec<ModelObjectRef<'a>>>;
}

/// Borrowed validation model object reference.
#[derive(Clone, Copy, Debug)]
pub enum ModelObjectRef<'a> {
    /// Document root object.
    Document(&'a DocumentModel<'a>),
    /// Catalog object.
    Catalog(&'a CatalogModel<'a>),
    /// Metadata stream object.
    Metadata(&'a MetadataModel<'a>),
    /// Basic stream object.
    Stream(&'a StreamModel<'a>),
}

impl<'a> ModelObjectRef<'a> {
    /// Returns the parsed document backing this model object.
    #[must_use]
    pub fn document(self) -> &'a ParsedDocument {
        match self {
            Self::Document(model) => model.document,
            Self::Catalog(model) => model.document,
            Self::Metadata(model) => model.document,
            Self::Stream(model) => model.document,
        }
    }

    /// Returns this object's type.
    #[must_use]
    pub fn object_type(self) -> ObjectTypeName {
        match self {
            Self::Document(model) => model.object_type(),
            Self::Catalog(model) => model.object_type(),
            Self::Metadata(model) => model.object_type(),
            Self::Stream(model) => model.object_type(),
        }
    }

    /// Looks up a property.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when the property is unknown.
    pub fn property(self, name: &PropertyName) -> Result<ModelValue> {
        match self {
            Self::Document(model) => model.property(name),
            Self::Catalog(model) => model.property(name),
            Self::Metadata(model) => model.property(name),
            Self::Stream(model) => model.property(name),
        }
    }

    fn location(self) -> ObjectLocation {
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
            Self::Stream(model) => ObjectLocation {
                object: Some(model.key),
                offset: Some(model.offset),
                path: Some(BoundedText::unchecked(format!(
                    "root/stream[{}]",
                    model.key.number
                ))),
            },
        }
    }

    fn context(self) -> BoundedText {
        match self {
            Self::Document(_) => BoundedText::unchecked("root"),
            Self::Catalog(_) => BoundedText::unchecked("root/catalog[0]"),
            Self::Metadata(_) => BoundedText::unchecked("root/catalog[0]/metadata[0]"),
            Self::Stream(model) => {
                BoundedText::unchecked(format!("root/stream[{}]", model.key.number))
            }
        }
    }

    fn identity_key(self) -> String {
        match self {
            Self::Document(_) => String::from("document"),
            Self::Catalog(model) => {
                format!("catalog:{}:{}", model.key.number, model.key.generation)
            }
            Self::Metadata(model) => {
                format!("metadata:{}:{}", model.key.number, model.key.generation)
            }
            Self::Stream(model) => format!("stream:{}:{}", model.key.number, model.key.generation),
        }
    }

    fn linked_objects(
        self,
        graph: &'a ModelGraph<'a>,
        catalog: Option<&'a CatalogModel<'a>>,
        metadata: Option<&'a MetadataModel<'a>>,
        streams: &'a [StreamModel<'a>],
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        match self {
            Self::Document(model) => model.linked_objects(graph, catalog, metadata, streams),
            Self::Catalog(model) => model.linked_objects(graph, catalog, metadata, streams),
            Self::Metadata(model) => model.linked_objects(graph, catalog, metadata, streams),
            Self::Stream(model) => model.linked_objects(graph, catalog, metadata, streams),
        }
    }
}

/// Document model wrapper.
#[derive(Debug)]
pub struct ModelGraph<'a> {
    document: &'a ParsedDocument,
}

impl<'a> ModelGraph<'a> {
    fn new(document: &'a ParsedDocument) -> Self {
        Self { document }
    }
}

/// Document model wrapper.
#[derive(Debug)]
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
            "encrypted" => Ok(ModelValue::Bool(self.document.is_encrypted())),
            "hasCatalog" => Ok(ModelValue::Bool(self.document.catalog.is_some())),
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
        &'a self,
        graph: &'a ModelGraph<'a>,
        catalog: Option<&'a CatalogModel<'a>>,
        _metadata: Option<&'a MetadataModel<'a>>,
        streams: &'a [StreamModel<'a>],
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut objects = Vec::new();
        if let Some(catalog) = catalog {
            objects.push(ModelObjectRef::Catalog(catalog));
        }
        for stream in streams.iter().rev() {
            if Some(stream.key) != graph.document.catalog {
                objects.push(ModelObjectRef::Stream(stream));
            }
        }
        Ok(objects)
    }
}

/// Catalog model wrapper.
#[derive(Debug)]
pub struct CatalogModel<'a> {
    document: &'a ParsedDocument,
    key: ObjectKey,
    offset: u64,
    metadata: Option<ObjectKey>,
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
        Some(Self {
            document,
            key,
            offset: object.offset,
            metadata,
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
        &'a self,
        _graph: &'a ModelGraph<'a>,
        _catalog: Option<&'a CatalogModel<'a>>,
        metadata: Option<&'a MetadataModel<'a>>,
        _streams: &'a [StreamModel<'a>],
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(metadata
            .map(ModelObjectRef::Metadata)
            .into_iter()
            .collect::<Vec<_>>())
    }
}

/// Metadata stream model wrapper.
#[derive(Debug)]
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
            "present" => Ok(ModelValue::Bool(true)),
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
        &'a self,
        _graph: &'a ModelGraph<'a>,
        _catalog: Option<&'a CatalogModel<'a>>,
        _metadata: Option<&'a MetadataModel<'a>>,
        _streams: &'a [StreamModel<'a>],
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        Ok(Vec::new())
    }
}

/// Stream model wrapper.
#[derive(Debug)]
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
        &'a self,
        _graph: &'a ModelGraph<'a>,
        _catalog: Option<&'a CatalogModel<'a>>,
        _metadata: Option<&'a MetadataModel<'a>>,
        _streams: &'a [StreamModel<'a>],
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

    fn rules_for(&self, object: ModelObjectRef<'_>) -> Vec<&'a Rule> {
        let mut rules = self
            .by_type
            .get(object.object_type().as_str())
            .cloned()
            .unwrap_or_default();
        let supertypes = match object {
            ModelObjectRef::Document(model) => model.super_types(),
            ModelObjectRef::Catalog(model) => model.super_types(),
            ModelObjectRef::Metadata(model) => model.super_types(),
            ModelObjectRef::Stream(model) => model.super_types(),
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
        object: ModelObjectRef<'_>,
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
        let outcome = match evaluator.evaluate(object, rule) {
            Ok(outcome) => outcome,
            Err(PdfvError::Profile(error)) => {
                self.unsupported_rules.push(UnsupportedRule {
                    profile_id: self.profile.id.clone(),
                    rule_id: rule.id.clone(),
                    expression_fragment: Some(BoundedText::unchecked(format!("{:?}", rule.test))),
                    reason: BoundedText::new(error.to_string(), 512)?,
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

    fn assertion(
        &mut self,
        object: ModelObjectRef<'_>,
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

fn u64_to_f64(value: u64) -> Result<f64> {
    let bounded = u32::try_from(value).map_err(|_| ValidationError::LimitExceeded {
        limit: "numeric_property",
    })?;
    Ok(f64::from(bounded))
}
