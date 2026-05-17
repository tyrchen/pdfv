//! Built-in validation profiles and bounded rule expression evaluation.

use std::{collections::BTreeMap, num::NonZeroU32};

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::{
    AssertionStatus, BoundedText, CosObject, FlavourSelection, Identifier, ObjectKey, ParseFact,
    ProfileError, ProfileIdentity, ResourceLimits, Result, RuleId, SpecReference, StreamFact,
    ValidationFlavour,
    generated_profiles::{GENERATED_PROFILE_SOURCES, GeneratedProfileSource, VERA_PDF_LIBRARY_PIN},
};

const MAX_RULE_INSTRUCTIONS: u64 = 512;
const MAX_RULE_DEPTH: u32 = 32;
const MAX_REGEX_PATTERN_BYTES: usize = 512;
const MAX_REGEX_HAYSTACK_BYTES: usize = 4096;
const MAX_PROFILE_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILE_XML_ELEMENTS: u64 = 100_000;
const MAX_PROFILE_XML_DEPTH: u32 = 32;
const MAX_PROFILE_XML_ATTRIBUTES: usize = 16;
const MAX_PROFILE_RULES: usize = 10_000;
const MAX_PROFILE_STRING_BYTES: usize = 4096;

/// Repository that resolves validation profiles for a caller selection.
pub trait ProfileRepository {
    /// Returns immutable profiles matching a selection.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when the selection is unsupported or profile data is invalid.
    fn profiles_for(&self, selection: &FlavourSelection) -> Result<Vec<ValidationProfile>>;
}

/// Rule evaluator interface.
pub trait RuleEvaluator {
    /// Evaluates one rule against one model object.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when rule evaluation fails or exceeds budgets.
    fn evaluate(&mut self, object: crate::ModelObjectRef<'_>, rule: &Rule) -> Result<RuleOutcome>;
}

/// Built-in profile repository.
#[derive(Clone, Debug, Default)]
pub struct BuiltinProfileRepository;

impl BuiltinProfileRepository {
    /// Creates a built-in profile repository.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Lists built-in profile metadata.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when built-in generated profile data is invalid.
    pub fn list_profiles(&self) -> Result<Vec<ProfileCatalogEntry>> {
        let mut entries = Vec::with_capacity(GENERATED_PROFILE_SOURCES.len().saturating_add(1));
        let m4 = m4_profile(pdfa_1b_flavour()?)?;
        entries.push(ProfileCatalogEntry::from_profile(
            &m4,
            "pdfv-internal",
            "built-in smoke profile",
            "pdfa-1b",
        )?);
        for source in GENERATED_PROFILE_SOURCES {
            let import = import_generated_profile(source)?;
            entries.push(ProfileCatalogEntry::from_import(source, &import)?);
        }
        Ok(entries)
    }
}

impl ProfileRepository for BuiltinProfileRepository {
    fn profiles_for(&self, selection: &FlavourSelection) -> Result<Vec<ValidationProfile>> {
        match selection {
            FlavourSelection::Auto { default } => {
                let Some(flavour) = default else {
                    return Ok(Vec::new());
                };
                ensure_builtin_flavour(flavour)?;
                Ok(vec![m4_profile(flavour.clone())?])
            }
            FlavourSelection::Explicit { flavour } => {
                let source = builtin_source_for_flavour(flavour)?;
                Ok(vec![import_generated_profile(source)?.profile])
            }
            FlavourSelection::CustomProfile { .. } => {
                #[cfg(feature = "custom-profiles")]
                {
                    let repository = CustomProfileRepository;
                    repository.profiles_for(selection)
                }
                #[cfg(not(feature = "custom-profiles"))]
                {
                    Err(ProfileError::UnsupportedSelection.into())
                }
            }
        }
    }
}

/// Repository that loads one bounded XML profile from disk.
#[cfg(feature = "custom-profiles")]
#[derive(Clone, Debug, Default)]
pub struct CustomProfileRepository;

#[cfg(feature = "custom-profiles")]
impl ProfileRepository for CustomProfileRepository {
    fn profiles_for(&self, selection: &FlavourSelection) -> Result<Vec<ValidationProfile>> {
        let FlavourSelection::CustomProfile { profile_path } = selection else {
            return Err(ProfileError::UnsupportedSelection.into());
        };
        Ok(vec![load_verapdf_profile_path(profile_path)?.profile])
    }
}

/// Summary produced by XML profile import.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileImportSummary {
    /// Imported profile.
    pub profile: ValidationProfile,
    /// Number of rules imported with executable expressions.
    pub supported_rules: u64,
    /// Number of rules imported as unsupported placeholders.
    pub unsupported_rules: u64,
}

/// Executable coverage metadata for a profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(
    clippy::struct_field_names,
    reason = "public report contract mirrors ExpressionCoverage terminology from the spec"
)]
pub struct ProfileCoverage {
    /// Total official rules imported from the source profile.
    pub total_rules: u64,
    /// Rules that lowered to executable Rust IR.
    pub executable_rules: u64,
    /// Required rules retained as unsupported report data.
    pub unsupported_rules: u64,
}

/// Profile metadata suitable for listing catalogs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileCatalogEntry {
    /// Profile identity.
    pub identity: ProfileIdentity,
    /// Validation flavour.
    pub flavour: ValidationFlavour,
    /// CLI/catalog spelling for the flavour.
    pub display_flavour: BoundedText,
    /// Source pin that produced this profile entry.
    pub source_pin: Identifier,
    /// Vendored or internal source description.
    pub source_file: BoundedText,
    /// Executable rule coverage.
    pub coverage: ProfileCoverage,
}

/// Model-schema parity report for generated built-in profiles.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSchemaParityReport {
    /// Vendor pins used by the generated profile catalog.
    pub vendor_pins: BTreeMap<String, String>,
    /// Number of registered validation model families.
    pub registered_families: u64,
    /// Number of registered model properties across families.
    pub registered_properties: u64,
    /// Number of registered model links across families.
    pub registered_links: u64,
    /// Per-profile model-schema coverage.
    pub profiles: Vec<ModelSchemaProfileReport>,
}

/// Per-profile model-schema coverage counters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSchemaProfileReport {
    /// CLI/catalog flavour spelling.
    pub flavour: BoundedText,
    /// Vendored XML source file.
    pub source: BoundedText,
    /// Total rules imported from XML.
    pub total_rules: u64,
    /// Rules whose expression lowered before model-schema checks.
    pub lowered_rules: u64,
    /// Lowered rules whose object, properties, and links bind to the registry.
    pub bound_rules: u64,
    /// Unsupported rules grouped by primary reason category.
    pub unsupported_by_reason: BTreeMap<String, u64>,
}

/// Builds a deterministic model-schema parity report for generated profiles.
///
/// # Errors
///
/// Returns [`crate::PdfvError`] if generated profile XML cannot be imported or
/// bounded report fields cannot be constructed.
pub fn model_schema_parity_report() -> Result<ModelSchemaParityReport> {
    let registry = crate::validation::ModelRegistry::default_registry();
    let mut profiles = Vec::with_capacity(GENERATED_PROFILE_SOURCES.len());
    for source in GENERATED_PROFILE_SOURCES {
        let import = import_verapdf_profile_xml(source.xml)?;
        profiles.push(model_schema_profile_report(
            &registry,
            source,
            &import.profile,
        )?);
    }
    let mut vendor_pins = BTreeMap::new();
    vendor_pins.insert(
        String::from("veraPDF-library"),
        String::from(VERA_PDF_LIBRARY_PIN),
    );
    Ok(ModelSchemaParityReport {
        vendor_pins,
        registered_families: registry.registered_family_count(),
        registered_properties: registry.registered_property_count(),
        registered_links: registry.registered_link_count(),
        profiles,
    })
}

impl ProfileCatalogEntry {
    fn from_import(source: &GeneratedProfileSource, import: &ProfileImportSummary) -> Result<Self> {
        Self::from_profile(
            &import.profile,
            VERA_PDF_LIBRARY_PIN,
            source.source_file,
            source.display_flavour,
        )
        .map(|mut entry| {
            entry.coverage = ProfileCoverage {
                total_rules: import
                    .supported_rules
                    .saturating_add(import.unsupported_rules),
                executable_rules: import.supported_rules,
                unsupported_rules: import.unsupported_rules,
            };
            entry
        })
    }

    fn from_profile(
        profile: &ValidationProfile,
        source_pin: &str,
        source_file: &str,
        display_flavour: &str,
    ) -> Result<Self> {
        Ok(Self {
            identity: profile.identity.clone(),
            flavour: profile.flavour.clone(),
            display_flavour: BoundedText::new(display_flavour, 128)?,
            source_pin: Identifier::new(source_pin)?,
            source_file: BoundedText::new(source_file, 512)?,
            coverage: ProfileCoverage {
                total_rules: u64::try_from(profile.rules.len()).unwrap_or(u64::MAX),
                executable_rules: u64::try_from(
                    profile
                        .rules
                        .iter()
                        .filter(|rule| !matches!(rule.test, RuleExpr::Unsupported { .. }))
                        .count(),
                )
                .unwrap_or(u64::MAX),
                unsupported_rules: u64::try_from(
                    profile
                        .rules
                        .iter()
                        .filter(|rule| matches!(rule.test, RuleExpr::Unsupported { .. }))
                        .count(),
                )
                .unwrap_or(u64::MAX),
            },
        })
    }
}

/// Immutable validation profile.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationProfile {
    /// Profile identity.
    pub identity: ProfileIdentity,
    /// Validation flavour.
    pub flavour: ValidationFlavour,
    /// Rules in deterministic execution order.
    pub rules: Vec<Rule>,
}

/// Validation rule.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    /// Rule id.
    pub id: RuleId,
    /// Object type targeted by this rule.
    pub object_type: ObjectTypeName,
    /// Whether this rule runs after traversal.
    pub deferred: bool,
    /// Rule tags.
    pub tags: Vec<Identifier>,
    /// Human-readable rule description.
    pub description: BoundedText,
    /// Bounded rule expression.
    pub test: RuleExpr,
    /// Error template used for failed assertions.
    pub error: ErrorTemplate,
    /// Specification citations associated with this rule.
    pub references: Vec<SpecReference>,
}

/// Error template for failed assertions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorTemplate {
    /// Bounded failure message.
    pub message: BoundedText,
}

/// Validation model object type name.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectTypeName(Identifier);

impl ObjectTypeName {
    /// Creates an object type name.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ConfigError`] when the identifier violates policy.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, crate::ConfigError> {
        Ok(Self(Identifier::new(value)?))
    }

    /// Returns the type name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn unchecked(value: &'static str) -> Self {
        Self(Identifier::unchecked(value))
    }
}

impl TryFrom<String> for ObjectTypeName {
    type Error = crate::ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ObjectTypeName> for String {
    fn from(value: ObjectTypeName) -> Self {
        value.0.into()
    }
}

/// Validation model property name.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PropertyName(Identifier);

impl PropertyName {
    /// Creates a property name.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ConfigError`] when the identifier violates policy.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, crate::ConfigError> {
        Ok(Self(Identifier::new(value)?))
    }

    /// Returns the property name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn unchecked(value: impl Into<String>) -> Self {
        Self(Identifier::unchecked(value))
    }
}

impl TryFrom<String> for PropertyName {
    type Error = crate::ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PropertyName> for String {
    fn from(value: PropertyName) -> Self {
        value.0.into()
    }
}

/// Dot-separated property path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct PropertyPath(Vec<PropertyName>);

impl PropertyPath {
    /// Creates a property path from names.
    #[must_use]
    pub fn new(parts: Vec<PropertyName>) -> Self {
        Self(parts)
    }

    /// Returns path parts.
    #[must_use]
    pub fn parts(&self) -> &[PropertyName] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for PropertyPath {
    type Error = crate::ConfigError;

    fn try_from(value: Vec<String>) -> std::result::Result<Self, Self::Error> {
        value
            .into_iter()
            .map(PropertyName::new)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(Self)
    }
}

impl From<PropertyPath> for Vec<String> {
    fn from(value: PropertyPath) -> Self {
        value.0.into_iter().map(Into::into).collect()
    }
}

/// Bounded rule expression.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuleExpr {
    /// Boolean literal.
    Bool {
        /// Literal value.
        value: bool,
    },
    /// Number literal.
    Number {
        /// Literal value.
        value: f64,
    },
    /// String literal.
    String {
        /// Literal value.
        value: BoundedText,
    },
    /// Null literal.
    Null,
    /// Property lookup.
    Property {
        /// Property path.
        path: PropertyPath,
    },
    /// Unary operation.
    Unary {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        expr: Box<RuleExpr>,
    },
    /// Binary operation.
    Binary {
        /// Operator.
        op: BinaryOp,
        /// Left operand.
        left: Box<RuleExpr>,
        /// Right operand.
        right: Box<RuleExpr>,
    },
    /// Ternary conditional expression.
    Conditional {
        /// Boolean condition.
        condition: Box<RuleExpr>,
        /// Expression evaluated when condition is true.
        when_true: Box<RuleExpr>,
        /// Expression evaluated when condition is false.
        when_false: Box<RuleExpr>,
    },
    /// Built-in function call.
    Call {
        /// Built-in function.
        function: BuiltinFunction,
        /// Arguments.
        args: Vec<RuleExpr>,
    },
    /// Unsupported source expression retained for report diagnostics.
    Unsupported {
        /// Original bounded expression fragment.
        fragment: BoundedText,
        /// Bounded reason.
        reason: BoundedText,
    },
}

/// Unary expression operator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum UnaryOp {
    /// Boolean negation.
    Not,
}

/// Binary expression operator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum BinaryOp {
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Numeric less-than-or-equal.
    Le,
    /// Numeric greater-than-or-equal.
    Ge,
    /// Numeric less-than.
    Lt,
    /// Numeric greater-than.
    Gt,
    /// Boolean conjunction.
    And,
    /// Boolean disjunction.
    Or,
    /// Numeric addition.
    Add,
    /// Numeric subtraction.
    Sub,
    /// Numeric multiplication.
    Mul,
    /// Numeric division.
    Div,
    /// Numeric modulo.
    Rem,
}

/// Bounded built-in function.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum BuiltinFunction {
    /// Returns true when a named parse fact exists.
    HasParseFact,
    /// Returns collection or string size.
    Size,
    /// Returns true when a collection or string is empty.
    IsEmpty,
    /// Returns true when a collection or string contains a value.
    Contains,
    /// Returns true when every boolean argument is true.
    All,
    /// Returns true when any boolean argument is true.
    Exists,
    /// Returns true when a bounded regex matches a string.
    Matches,
}

/// Model value used by rule evaluation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ModelValue {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Number value.
    Number(f64),
    /// String value.
    String(BoundedText),
    /// Object key value.
    ObjectKey(ObjectKey),
    /// Bounded list value.
    List(Vec<ModelValue>),
}

/// Rule evaluation outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuleOutcome {
    /// Rule assertion passed.
    Passed,
    /// Rule assertion failed.
    Failed,
}

impl RuleOutcome {
    /// Converts the outcome to report assertion status.
    #[must_use]
    pub fn assertion_status(self) -> AssertionStatus {
        match self {
            Self::Passed => AssertionStatus::Passed,
            Self::Failed => AssertionStatus::Failed,
        }
    }
}

/// Default bounded rule evaluator.
#[derive(Clone, Debug)]
pub struct DefaultRuleEvaluator<'a> {
    limits: ResourceLimits,
    instructions: u64,
    graph: Option<&'a crate::validation::ModelGraph<'a>>,
}

impl<'a> DefaultRuleEvaluator<'a> {
    /// Creates an evaluator.
    #[must_use]
    #[cfg(test)]
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            instructions: 0,
            graph: None,
        }
    }

    /// Creates an evaluator that can follow validation model links.
    #[must_use]
    pub(crate) fn with_graph(
        limits: ResourceLimits,
        graph: &'a crate::validation::ModelGraph<'a>,
    ) -> Self {
        Self {
            limits,
            instructions: 0,
            graph: Some(graph),
        }
    }

    fn eval(
        &mut self,
        object: &crate::ModelObjectRef<'_>,
        expr: &RuleExpr,
        depth: u32,
    ) -> Result<ModelValue> {
        self.instructions =
            self.instructions
                .checked_add(1)
                .ok_or(ProfileError::BudgetExceeded {
                    budget: "instructions",
                })?;
        if self.instructions > MAX_RULE_INSTRUCTIONS {
            return Err(ProfileError::BudgetExceeded {
                budget: "instructions",
            }
            .into());
        }
        if depth > MAX_RULE_DEPTH || depth > self.limits.max_object_depth {
            return Err(ProfileError::BudgetExceeded { budget: "depth" }.into());
        }

        match expr {
            RuleExpr::Bool { value } => Ok(ModelValue::Bool(*value)),
            RuleExpr::Number { value } => Ok(ModelValue::Number(*value)),
            RuleExpr::String { value } => Ok(ModelValue::String(value.clone())),
            RuleExpr::Null => Ok(ModelValue::Null),
            RuleExpr::Property { path } => self.property(object, path),
            RuleExpr::Unary { op, expr } => {
                let value = self.eval(object, expr, depth.saturating_add(1))?;
                match (op, value) {
                    (UnaryOp::Not, ModelValue::Bool(value)) => Ok(ModelValue::Bool(!value)),
                    _ => Err(type_mismatch("unary operator requires boolean").into()),
                }
            }
            RuleExpr::Binary { op, left, right } => {
                self.eval_binary(object, *op, left, right, depth)
            }
            RuleExpr::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                if expect_bool(&self.eval(object, condition, depth.saturating_add(1))?)? {
                    self.eval(object, when_true, depth.saturating_add(1))
                } else {
                    self.eval(object, when_false, depth.saturating_add(1))
                }
            }
            RuleExpr::Call { function, args } => self.eval_call(object, *function, args, depth),
            RuleExpr::Unsupported { reason, .. } => Err(ProfileError::UnsupportedRule {
                reason: reason.clone(),
            }
            .into()),
        }
    }

    fn eval_binary(
        &mut self,
        object: &crate::ModelObjectRef<'_>,
        op: BinaryOp,
        left: &RuleExpr,
        right: &RuleExpr,
        depth: u32,
    ) -> Result<ModelValue> {
        if op == BinaryOp::And {
            let left = expect_bool(&self.eval(object, left, depth.saturating_add(1))?)?;
            if !left {
                return Ok(ModelValue::Bool(false));
            }
            let right = expect_bool(&self.eval(object, right, depth.saturating_add(1))?)?;
            return Ok(ModelValue::Bool(right));
        }
        if op == BinaryOp::Or {
            let left = expect_bool(&self.eval(object, left, depth.saturating_add(1))?)?;
            if left {
                return Ok(ModelValue::Bool(true));
            }
            let right = expect_bool(&self.eval(object, right, depth.saturating_add(1))?)?;
            return Ok(ModelValue::Bool(right));
        }

        let left = self.eval(object, left, depth.saturating_add(1))?;
        let right = self.eval(object, right, depth.saturating_add(1))?;
        let result = match op {
            BinaryOp::Eq => values_equal(&left, &right),
            BinaryOp::Ne => !values_equal(&left, &right),
            BinaryOp::Le => expect_number(&left)? <= expect_number(&right)?,
            BinaryOp::Ge => expect_number(&left)? >= expect_number(&right)?,
            BinaryOp::Lt => expect_number(&left)? < expect_number(&right)?,
            BinaryOp::Gt => expect_number(&left)? > expect_number(&right)?,
            BinaryOp::And | BinaryOp::Or => false,
            BinaryOp::Add => {
                return Ok(ModelValue::Number(
                    expect_number(&left)? + expect_number(&right)?,
                ));
            }
            BinaryOp::Sub => {
                return Ok(ModelValue::Number(
                    expect_number(&left)? - expect_number(&right)?,
                ));
            }
            BinaryOp::Mul => {
                return Ok(ModelValue::Number(
                    expect_number(&left)? * expect_number(&right)?,
                ));
            }
            BinaryOp::Div => {
                let divisor = expect_number(&right)?;
                if divisor.abs() < f64::EPSILON {
                    return Err(type_mismatch("division by zero").into());
                }
                return Ok(ModelValue::Number(expect_number(&left)? / divisor));
            }
            BinaryOp::Rem => {
                let divisor = expect_number(&right)?;
                if divisor.abs() < f64::EPSILON {
                    return Err(type_mismatch("modulo by zero").into());
                }
                return Ok(ModelValue::Number(expect_number(&left)? % divisor));
            }
        };
        Ok(ModelValue::Bool(result))
    }

    fn eval_call(
        &mut self,
        object: &crate::ModelObjectRef<'_>,
        function: BuiltinFunction,
        args: &[RuleExpr],
        depth: u32,
    ) -> Result<ModelValue> {
        match function {
            BuiltinFunction::HasParseFact => {
                let value = self.eval_single_arg(object, args, depth, "hasParseFact")?;
                let ModelValue::String(name) = value else {
                    return Err(type_mismatch("hasParseFact requires string").into());
                };
                Ok(ModelValue::Bool(has_parse_fact(
                    object.document().parse_facts.as_slice(),
                    name.as_str(),
                )))
            }
            BuiltinFunction::Size => {
                let value = self.eval_single_arg(object, args, depth, "size")?;
                Ok(ModelValue::Number(usize_to_f64(collection_len(&value)?)?))
            }
            BuiltinFunction::IsEmpty => {
                let value = self.eval_single_arg(object, args, depth, "isEmpty")?;
                Ok(ModelValue::Bool(collection_len(&value)? == 0))
            }
            BuiltinFunction::Contains => {
                if args.len() != 2 {
                    return Err(type_mismatch("contains requires two arguments").into());
                }
                let haystack = self.eval(
                    object,
                    args.first()
                        .ok_or_else(|| type_mismatch("contains requires haystack"))?,
                    depth.saturating_add(1),
                )?;
                let needle = self.eval(
                    object,
                    args.get(1)
                        .ok_or_else(|| type_mismatch("contains requires needle"))?,
                    depth.saturating_add(1),
                )?;
                Ok(ModelValue::Bool(contains_value(&haystack, &needle)?))
            }
            BuiltinFunction::All => {
                let mut result = true;
                for arg in args {
                    result &= expect_bool(&self.eval(object, arg, depth.saturating_add(1))?)?;
                    if !result {
                        break;
                    }
                }
                Ok(ModelValue::Bool(result))
            }
            BuiltinFunction::Exists => {
                let mut result = false;
                for arg in args {
                    result |= expect_bool(&self.eval(object, arg, depth.saturating_add(1))?)?;
                    if result {
                        break;
                    }
                }
                Ok(ModelValue::Bool(result))
            }
            BuiltinFunction::Matches => {
                if args.len() != 2 {
                    return Err(type_mismatch("matches requires pattern and string").into());
                }
                let pattern = self.eval(
                    object,
                    args.first()
                        .ok_or_else(|| type_mismatch("matches requires pattern"))?,
                    depth.saturating_add(1),
                )?;
                let haystack = self.eval(
                    object,
                    args.get(1)
                        .ok_or_else(|| type_mismatch("matches requires string"))?,
                    depth.saturating_add(1),
                )?;
                let (ModelValue::String(pattern), ModelValue::String(haystack)) =
                    (pattern, haystack)
                else {
                    return Err(type_mismatch("matches requires string arguments").into());
                };
                if pattern.as_str().len() > MAX_REGEX_PATTERN_BYTES
                    || haystack.as_str().len() > MAX_REGEX_HAYSTACK_BYTES
                {
                    return Err(ProfileError::BudgetExceeded { budget: "regex" }.into());
                }
                let regex = RegexBuilder::new(pattern.as_str())
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                    .map_err(|error| ProfileError::InvalidField {
                        field: "regex",
                        reason: BoundedText::new(error.to_string(), 512)
                            .unwrap_or_else(|_| BoundedText::unchecked("invalid regex")),
                    })?;
                Ok(ModelValue::Bool(regex.is_match(haystack.as_str())))
            }
        }
    }

    fn eval_single_arg(
        &mut self,
        object: &crate::ModelObjectRef<'_>,
        args: &[RuleExpr],
        depth: u32,
        name: &'static str,
    ) -> Result<ModelValue> {
        if args.len() != 1 {
            return Err(type_mismatch("built-in requires exactly one argument").into());
        }
        let Some(first) = args.first() else {
            return Err(type_mismatch(name).into());
        };
        self.eval(object, first, depth.saturating_add(1))
    }
    fn property(
        &self,
        object: &crate::ModelObjectRef<'a>,
        path: &PropertyPath,
    ) -> Result<ModelValue> {
        if path.parts().is_empty() {
            return Err(ProfileError::UnknownProperty {
                property: BoundedText::unchecked("empty"),
            }
            .into());
        }
        let Some((terminal, links)) = path.parts().split_last() else {
            return Err(ProfileError::UnknownProperty {
                property: BoundedText::unchecked("empty"),
            }
            .into());
        };
        if links.is_empty() {
            return object.property(terminal);
        }
        let Some(graph) = self.graph else {
            return Err(ProfileError::UnsupportedRule {
                reason: BoundedText::unchecked("nested property path requires validation graph"),
            }
            .into());
        };
        let mut current = vec![object.clone()];
        let registry = crate::validation::ModelRegistry::default_registry();
        let max_objects = usize::try_from(self.limits.max_objects).unwrap_or(usize::MAX);
        for link in links {
            let mut next = Vec::new();
            for candidate in &current {
                let Some(target) = registry.link_target(&candidate.object_type(), link) else {
                    return Err(ProfileError::UnsupportedRule {
                        reason: BoundedText::new(
                            format!(
                                "unknown validation model link {} on {}",
                                link.as_str(),
                                candidate.object_type().as_str()
                            ),
                            512,
                        )?,
                    }
                    .into());
                };
                if !candidate
                    .links()
                    .iter()
                    .any(|name| name.as_str() == link.as_str())
                {
                    return Err(ProfileError::UnsupportedRule {
                        reason: BoundedText::new(
                            format!(
                                "unmaterialized validation model link {} on {}",
                                link.as_str(),
                                candidate.object_type().as_str()
                            ),
                            512,
                        )?,
                    }
                    .into());
                }
                for linked in candidate.linked_objects(graph, max_objects)? {
                    if linked.object_type() == target {
                        next.push(linked);
                    }
                }
            }
            if next.len() > max_objects {
                return Err(ProfileError::BudgetExceeded {
                    budget: "linked_objects",
                }
                .into());
            }
            current = next;
        }
        if current.is_empty() {
            return Ok(ModelValue::Null);
        }
        let mut values = Vec::with_capacity(current.len());
        for candidate in current {
            values.push(candidate.property(terminal)?);
        }
        if values.len() == 1 {
            values.into_iter().next().ok_or_else(|| {
                ProfileError::BudgetExceeded {
                    budget: "linked_objects",
                }
                .into()
            })
        } else {
            Ok(ModelValue::List(values))
        }
    }
}

impl RuleEvaluator for DefaultRuleEvaluator<'_> {
    fn evaluate(&mut self, object: crate::ModelObjectRef<'_>, rule: &Rule) -> Result<RuleOutcome> {
        self.instructions = 0;
        let value = self.eval(&object, &rule.test, 0)?;
        if expect_bool(&value)? {
            Ok(RuleOutcome::Passed)
        } else {
            Ok(RuleOutcome::Failed)
        }
    }
}

fn expect_bool(value: &ModelValue) -> Result<bool> {
    match value {
        ModelValue::Bool(value) => Ok(*value),
        _ => Err(type_mismatch("expected boolean").into()),
    }
}

fn expect_number(value: &ModelValue) -> Result<f64> {
    match value {
        ModelValue::Number(value) => Ok(*value),
        _ => Err(type_mismatch("expected number").into()),
    }
}

fn values_equal(left: &ModelValue, right: &ModelValue) -> bool {
    match (left, right) {
        (ModelValue::Null, ModelValue::Null) => true,
        (ModelValue::Bool(left), ModelValue::Bool(right)) => left == right,
        (ModelValue::Number(left), ModelValue::Number(right)) => {
            (left - right).abs() < f64::EPSILON
        }
        (ModelValue::String(left), ModelValue::String(right)) => left == right,
        (ModelValue::ObjectKey(left), ModelValue::ObjectKey(right)) => left == right,
        (ModelValue::List(left), ModelValue::List(right)) => left == right,
        _ => false,
    }
}

fn collection_len(value: &ModelValue) -> Result<usize> {
    match value {
        ModelValue::String(value) => Ok(value.as_str().len()),
        ModelValue::List(value) => Ok(value.len()),
        _ => Err(type_mismatch("expected collection or string").into()),
    }
}

fn usize_to_f64(value: usize) -> Result<f64> {
    let value = u32::try_from(value).map_err(|_| ProfileError::BudgetExceeded {
        budget: "collection_size",
    })?;
    Ok(f64::from(value))
}

fn contains_value(haystack: &ModelValue, needle: &ModelValue) -> Result<bool> {
    match (haystack, needle) {
        (ModelValue::String(haystack), ModelValue::String(needle)) => {
            Ok(haystack.as_str().contains(needle.as_str()))
        }
        (ModelValue::List(values), needle) => {
            Ok(values.iter().any(|value| values_equal(value, needle)))
        }
        _ => Err(type_mismatch("contains requires compatible arguments").into()),
    }
}

fn type_mismatch(message: &'static str) -> ProfileError {
    ProfileError::TypeMismatch {
        message: BoundedText::unchecked(message),
    }
}

fn has_parse_fact(facts: &[ParseFact], name: &str) -> bool {
    facts.iter().any(|fact| match (name, fact) {
        ("header", ParseFact::Header { .. })
        | (
            "encryption",
            ParseFact::Encryption {
                encrypted: true, ..
            },
        ) => true,
        (
            "streamLengthMismatch",
            ParseFact::Stream {
                fact:
                    StreamFact::Length {
                        declared,
                        discovered,
                    },
                ..
            },
        ) => declared != discovered,
        _ => false,
    })
}

fn pdfa_1b_flavour() -> Result<ValidationFlavour> {
    Ok(ValidationFlavour::new("pdfa", NonZeroU32::MIN, "b")?)
}

fn ensure_builtin_flavour(flavour: &ValidationFlavour) -> Result<()> {
    builtin_source_for_flavour(flavour).map(|_| ())
}

fn import_generated_profile(source: &GeneratedProfileSource) -> Result<ProfileImportSummary> {
    let mut import = import_verapdf_profile_xml(source.xml)?;
    import.profile.identity.id = Identifier::new(source.id)?;
    import.profile.identity.version = Some(Identifier::new("verapdf-generated")?);
    import.profile.flavour = parse_display_flavour(source.display_flavour)?;
    apply_model_schema_checks(&mut import)?;
    Ok(import)
}

fn apply_model_schema_checks(import: &mut ProfileImportSummary) -> Result<()> {
    let registry = crate::validation::ModelRegistry::default_registry();
    let mut supported_rules = 0_u64;
    let mut unsupported_rules = 0_u64;
    for rule in &mut import.profile.rules {
        if matches!(rule.test, RuleExpr::Unsupported { .. }) {
            unsupported_rules = unsupported_rules.saturating_add(1);
            continue;
        }
        let unsupported_reason = if registry.has_family(&rule.object_type) {
            unsupported_property_reason(&registry, &rule.object_type, &rule.test)?
        } else {
            Some(BoundedText::new(
                format!(
                    "unknown validation model family {}",
                    rule.object_type.as_str()
                ),
                512,
            )?)
        };
        if let Some(reason) = unsupported_reason {
            let fragment = BoundedText::new(format!("{:?}", rule.test), MAX_PROFILE_STRING_BYTES)
                .unwrap_or_else(|_| BoundedText::unchecked("rule expression exceeds limit"));
            rule.test = RuleExpr::Unsupported { fragment, reason };
            unsupported_rules = unsupported_rules.saturating_add(1);
        } else {
            supported_rules = supported_rules.saturating_add(1);
        }
    }
    import.supported_rules = supported_rules;
    import.unsupported_rules = unsupported_rules;
    Ok(())
}

fn model_schema_profile_report(
    registry: &crate::validation::ModelRegistry,
    source: &GeneratedProfileSource,
    profile: &ValidationProfile,
) -> Result<ModelSchemaProfileReport> {
    let mut lowered_rules = 0_u64;
    let mut bound_rules = 0_u64;
    let mut unsupported_by_reason = BTreeMap::new();
    for rule in &profile.rules {
        if matches!(rule.test, RuleExpr::Unsupported { .. }) {
            increment_reason(&mut unsupported_by_reason, "unsupportedExpression");
            continue;
        }
        lowered_rules = lowered_rules.saturating_add(1);
        let reason =
            if let Some(reason) = missing_semantic_family_reason(&rule.object_type, &rule.test) {
                Some(reason)
            } else if registry.has_family(&rule.object_type) {
                unsupported_property_reason(registry, &rule.object_type, &rule.test)?
                    .map(|reason| unsupported_reason_category(reason.as_str()))
            } else {
                Some("missingObjectType")
            };
        if let Some(reason) = reason {
            increment_reason(&mut unsupported_by_reason, reason);
        } else {
            bound_rules = bound_rules.saturating_add(1);
        }
    }
    Ok(ModelSchemaProfileReport {
        flavour: BoundedText::new(source.display_flavour, 128)?,
        source: BoundedText::new(source.source_file, 512)?,
        total_rules: u64::try_from(profile.rules.len()).unwrap_or(u64::MAX),
        lowered_rules,
        bound_rules,
        unsupported_by_reason,
    })
}

fn unsupported_reason_category(reason: &str) -> &'static str {
    if reason.contains("unknown validation model link") {
        "missingLink"
    } else if reason.contains("unknown validation model property") {
        "missingProperty"
    } else if reason.contains("unknown validation model family") {
        "missingObjectType"
    } else {
        "unsupportedExpression"
    }
}

fn missing_semantic_family_reason(
    object_type: &ObjectTypeName,
    expr: &RuleExpr,
) -> Option<&'static str> {
    let mut properties = Vec::new();
    collect_property_paths(expr, &mut properties);
    properties
        .into_iter()
        .any(|path| {
            path.parts().iter().any(|property| {
                matches!(
                    property.as_str(),
                    "numberOfColumnWithWrongRowSpan"
                        | "numberOfRowWithWrongColumnSpan"
                        | "wrongColumnSpan"
                )
            })
        })
        .then_some("missingSemanticFamily")
        .or_else(|| {
            matches!(object_type.as_str(), "undefinedOperator").then_some("missingSemanticFamily")
        })
}

fn increment_reason(reasons: &mut BTreeMap<String, u64>, reason: &'static str) {
    reasons
        .entry(String::from(reason))
        .and_modify(|count| *count = count.saturating_add(1))
        .or_insert(1);
}

fn unsupported_property_reason(
    registry: &crate::validation::ModelRegistry,
    object_type: &ObjectTypeName,
    expr: &RuleExpr,
) -> Result<Option<BoundedText>> {
    let mut properties = Vec::new();
    collect_property_paths(expr, &mut properties);
    for property in properties {
        if let Some(reason) = registry.unsupported_property_path_reason(object_type, property)? {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

fn collect_property_paths<'a>(expr: &'a RuleExpr, properties: &mut Vec<&'a PropertyPath>) {
    match expr {
        RuleExpr::Property { path } => properties.push(path),
        RuleExpr::Unary { expr, .. } => collect_property_paths(expr, properties),
        RuleExpr::Binary { left, right, .. } => {
            collect_property_paths(left, properties);
            collect_property_paths(right, properties);
        }
        RuleExpr::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            collect_property_paths(condition, properties);
            collect_property_paths(when_true, properties);
            collect_property_paths(when_false, properties);
        }
        RuleExpr::Call { args, .. } => {
            for arg in args {
                collect_property_paths(arg, properties);
            }
        }
        RuleExpr::Bool { .. }
        | RuleExpr::Number { .. }
        | RuleExpr::String { .. }
        | RuleExpr::Null
        | RuleExpr::Unsupported { .. } => {}
    }
}

fn builtin_source_for_flavour(
    flavour: &ValidationFlavour,
) -> Result<&'static GeneratedProfileSource> {
    let display = display_flavour(flavour)?;
    GENERATED_PROFILE_SOURCES
        .iter()
        .find(|source| source.display_flavour == display.as_str())
        .ok_or_else(|| ProfileError::UnsupportedSelection.into())
}

/// Returns the stable CLI/catalog spelling for a validation flavour.
///
/// # Errors
///
/// Returns [`crate::PdfvError`] when the flavour cannot be represented by the
/// built-in profile catalog.
pub fn display_flavour(flavour: &ValidationFlavour) -> Result<BoundedText> {
    let family = flavour.family.as_str();
    let conformance = flavour.conformance.as_str();
    let value = match family {
        "pdfa" if conformance == "none" => format!("pdfa-{}", flavour.part),
        "pdfa" => format!("pdfa-{}{}", flavour.part, conformance),
        "pdfua" if flavour.part.get() == 1 && conformance == "none" => String::from("pdfua-1"),
        "pdfua" if flavour.part.get() == 2 && conformance == "iso32005" => {
            String::from("pdfua-2-iso32005")
        }
        "wtpdf" if matches!(conformance, "reuse" | "accessibility") => {
            format!("wtpdf-1-0-{conformance}")
        }
        _ => return Err(ProfileError::UnsupportedSelection.into()),
    };
    Ok(BoundedText::new(value, 128)?)
}

fn parse_display_flavour(value: &str) -> Result<ValidationFlavour> {
    if let Some(rest) = value.strip_prefix("pdfa-") {
        return parse_display_pdfa_flavour(rest);
    }
    if let Some(rest) = value.strip_prefix("pdfua-") {
        return parse_display_pdfua_flavour(rest);
    }
    if let Some(level) = value.strip_prefix("wtpdf-1-0-")
        && matches!(level, "reuse" | "accessibility")
    {
        return ValidationFlavour::new("wtpdf", NonZeroU32::MIN, level).map_err(Into::into);
    }
    Err(ProfileError::UnsupportedSelection.into())
}

fn parse_display_pdfa_flavour(rest: &str) -> Result<ValidationFlavour> {
    let split_at = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    let (part, conformance) = rest.split_at(split_at);
    let part = part
        .parse::<u32>()
        .map_err(|_| ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("PDF/A part is not numeric"),
        })?;
    let part = NonZeroU32::new(part).ok_or(ProfileError::InvalidField {
        field: "flavour",
        reason: BoundedText::unchecked("PDF/A part is zero"),
    })?;
    let conformance = if conformance.is_empty() {
        "none"
    } else {
        conformance
    };
    ValidationFlavour::new("pdfa", part, conformance).map_err(Into::into)
}

fn parse_display_pdfua_flavour(rest: &str) -> Result<ValidationFlavour> {
    match rest {
        "1" => ValidationFlavour::new("pdfua", NonZeroU32::MIN, "none").map_err(Into::into),
        "2-iso32005" => ValidationFlavour::new(
            "pdfua",
            NonZeroU32::new(2).ok_or(ProfileError::UnsupportedSelection)?,
            "iso32005",
        )
        .map_err(Into::into),
        _ => Err(ProfileError::UnsupportedSelection.into()),
    }
}

fn m4_profile(flavour: ValidationFlavour) -> Result<ValidationProfile> {
    Ok(ValidationProfile {
        identity: ProfileIdentity {
            id: Identifier::new("pdfv-m4")?,
            name: BoundedText::new("pdfv M4 built-in profile", 128)?,
            version: Some(Identifier::new("0.1.0")?),
        },
        flavour,
        rules: vec![
            rule(
                "m0-header-offset-zero",
                "document",
                "PDF header must start at byte zero",
                property_expr("headerOffset")?,
                BinaryOp::Eq,
                RuleExpr::Number { value: 0.0 },
            )?,
            rule(
                "m0-document-not-encrypted",
                "document",
                "Encrypted documents are not validated in M0",
                property_expr("encrypted")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: false },
            )?,
            rule(
                "m0-catalog-present",
                "document",
                "Trailer must reference a catalog",
                property_expr("hasCatalog")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-page-contents-present",
                "page",
                "Page dictionaries must contain contents",
                property_expr("hasContents")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-page-resources-present",
                "page",
                "Page dictionaries must contain resources",
                property_expr("hasResources")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-font-subtype-present",
                "font",
                "Font dictionaries must contain a Subtype entry",
                property_expr("hasSubtype")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-annotation-subtype-present",
                "annotation",
                "Annotation dictionaries must contain a Subtype entry",
                property_expr("hasSubtype")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-output-intent-profile-present",
                "outputIntent",
                "Output intent dictionaries must contain a destination output profile",
                property_expr("hasDestOutputProfile")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
            rule(
                "m4-content-stream-length-non-negative",
                "contentStream",
                "Page content streams must expose a non-negative declared or discovered length",
                property_expr("declaredLength")?,
                BinaryOp::Ge,
                RuleExpr::Number { value: 0.0 },
            )?,
            rule(
                "m0-stream-length-matches",
                "stream",
                "Stream declared length must match discovered length",
                property_expr("lengthMatches")?,
                BinaryOp::Eq,
                RuleExpr::Bool { value: true },
            )?,
        ],
    })
}

fn rule(
    id: &str,
    object_type: &str,
    description: &str,
    left: RuleExpr,
    op: BinaryOp,
    right: RuleExpr,
) -> Result<Rule> {
    Ok(Rule {
        id: RuleId(Identifier::new(id)?),
        object_type: ObjectTypeName::new(object_type)?,
        deferred: false,
        tags: Vec::new(),
        description: BoundedText::new(description, 256)?,
        test: RuleExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        error: ErrorTemplate {
            message: BoundedText::new(description, 256)?,
        },
        references: Vec::new(),
    })
}

fn property_expr(name: &str) -> Result<RuleExpr> {
    Ok(RuleExpr::Property {
        path: PropertyPath::new(vec![PropertyName(Identifier::new(name)?)]),
    })
}

#[cfg(feature = "custom-profiles")]
#[allow(
    clippy::disallowed_methods,
    reason = "custom profile loading is a synchronous library API matching validate_path"
)]
fn load_verapdf_profile_path(path: &std::path::Path) -> Result<ProfileImportSummary> {
    let metadata = std::fs::metadata(path).map_err(|source| crate::PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    if metadata.len() > MAX_PROFILE_XML_BYTES {
        return Err(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("profile XML exceeds byte limit"),
        }
        .into());
    }
    let xml = std::fs::read_to_string(path).map_err(|source| crate::PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    import_verapdf_profile_xml(&xml)
}

/// Imports a veraPDF validation profile XML document into bounded profile data.
///
/// Unsupported expressions are retained as rules that report `unsupportedRules`
/// during validation instead of being silently skipped.
///
/// # Errors
///
/// Returns [`crate::PdfvError`] when XML, identifiers, or bounded strings are invalid.
pub fn import_verapdf_profile_xml(xml: &str) -> Result<ProfileImportSummary> {
    import_verapdf_profile_xml_impl(xml)
}

#[allow(
    clippy::too_many_lines,
    reason = "event-driven XML import keeps parser state local and explicit"
)]
fn import_verapdf_profile_xml_impl(xml: &str) -> Result<ProfileImportSummary> {
    use quick_xml::{Reader, events::Event};

    if u64::try_from(xml.len()).map_err(|_| ProfileError::InvalidXml {
        reason: BoundedText::unchecked("profile XML length overflow"),
    })? > MAX_PROFILE_XML_BYTES
    {
        return Err(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("profile XML exceeds byte limit"),
        }
        .into());
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut elements = 0_u64;
    let mut profile_name: Option<BoundedText> = None;
    let mut flavour: Option<ValidationFlavour> = None;
    let mut rules = Vec::new();
    let mut current_rule: Option<XmlRuleBuilder> = None;
    let mut current_text = XmlTextTarget::None;
    let mut depth = 0_u32;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| ProfileError::InvalidXml {
                reason: BoundedText::new(error.to_string(), 512)
                    .unwrap_or_else(|_| BoundedText::unchecked("XML parser error")),
            })?;
        match event {
            Event::Start(element) => {
                validate_element(&element)?;
                depth = depth.checked_add(1).ok_or(ProfileError::InvalidXml {
                    reason: BoundedText::unchecked("profile XML depth overflow"),
                })?;
                if depth > MAX_PROFILE_XML_DEPTH {
                    return Err(ProfileError::InvalidXml {
                        reason: BoundedText::unchecked("profile XML exceeds depth limit"),
                    }
                    .into());
                }
                elements = elements.checked_add(1).ok_or(ProfileError::InvalidXml {
                    reason: BoundedText::unchecked("profile XML element count overflow"),
                })?;
                if elements > MAX_PROFILE_XML_ELEMENTS {
                    return Err(ProfileError::InvalidXml {
                        reason: BoundedText::unchecked("profile XML exceeds element limit"),
                    }
                    .into());
                }
                match element.name().as_ref() {
                    b"profile" => {
                        flavour = profile_flavour_attr(&element)?;
                    }
                    b"rule" => {
                        if rules.len() >= MAX_PROFILE_RULES {
                            return Err(ProfileError::InvalidXml {
                                reason: BoundedText::unchecked("profile XML exceeds rule limit"),
                            }
                            .into());
                        }
                        current_rule = Some(XmlRuleBuilder::from_rule_start(&element)?);
                    }
                    b"name" if current_rule.is_none() => current_text = XmlTextTarget::ProfileName,
                    b"description" if current_rule.is_some() => {
                        current_text = XmlTextTarget::RuleDescription;
                    }
                    b"test" if current_rule.is_some() => current_text = XmlTextTarget::RuleTest,
                    b"message" if current_rule.is_some() => {
                        current_text = XmlTextTarget::RuleMessage;
                    }
                    b"id" if current_rule.is_some() => {
                        if let Some(rule) = current_rule.as_mut() {
                            rule.id = Some(rule_id_from_attrs(&element)?);
                        }
                    }
                    b"reference" if current_rule.is_some() => {
                        if let Some(rule) = current_rule.as_mut() {
                            rule.references.push(reference_from_attrs(&element)?);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(text) => {
                let decoded = text.decode().map_err(|error| ProfileError::InvalidXml {
                    reason: BoundedText::new(error.to_string(), 512)
                        .unwrap_or_else(|_| BoundedText::unchecked("XML text decode error")),
                })?;
                let bounded = BoundedText::new(decoded.into_owned(), MAX_PROFILE_STRING_BYTES)?;
                match current_text {
                    XmlTextTarget::ProfileName => profile_name = Some(bounded),
                    XmlTextTarget::RuleDescription => {
                        if let Some(rule) = current_rule.as_mut() {
                            rule.description = Some(bounded);
                        }
                    }
                    XmlTextTarget::RuleTest => {
                        if let Some(rule) = current_rule.as_mut() {
                            rule.test = Some(bounded);
                        }
                    }
                    XmlTextTarget::RuleMessage => {
                        if let Some(rule) = current_rule.as_mut() {
                            rule.message = Some(bounded);
                        }
                    }
                    XmlTextTarget::None => {}
                }
            }
            Event::End(element) => {
                match element.name().as_ref() {
                    b"name" | b"description" | b"test" | b"message" => {
                        current_text = XmlTextTarget::None;
                    }
                    b"rule" => {
                        let Some(builder) = current_rule.take() else {
                            return Err(ProfileError::InvalidXml {
                                reason: BoundedText::unchecked("closing rule without start"),
                            }
                            .into());
                        };
                        rules.push(builder.finish()?);
                    }
                    _ => {}
                }
                depth = depth.checked_sub(1).ok_or(ProfileError::InvalidXml {
                    reason: BoundedText::unchecked("profile XML depth underflow"),
                })?;
            }
            Event::Empty(element) => {
                validate_element(&element)?;
                elements = elements.checked_add(1).ok_or(ProfileError::InvalidXml {
                    reason: BoundedText::unchecked("profile XML element count overflow"),
                })?;
                if elements > MAX_PROFILE_XML_ELEMENTS {
                    return Err(ProfileError::InvalidXml {
                        reason: BoundedText::unchecked("profile XML exceeds element limit"),
                    }
                    .into());
                }
                if element.name().as_ref() == b"id"
                    && let Some(rule) = current_rule.as_mut()
                {
                    rule.id = Some(rule_id_from_attrs(&element)?);
                }
                if element.name().as_ref() == b"reference"
                    && let Some(rule) = current_rule.as_mut()
                {
                    rule.references.push(reference_from_attrs(&element)?);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let flavour = flavour.ok_or(ProfileError::InvalidXml {
        reason: BoundedText::unchecked("profile flavour is missing"),
    })?;
    let profile_id = profile_id_for_flavour(&flavour)?;
    let mut supported_rules = 0_u64;
    let mut unsupported_rules = 0_u64;
    for rule in &rules {
        if matches!(rule.test, RuleExpr::Unsupported { .. }) {
            unsupported_rules = unsupported_rules.saturating_add(1);
        } else {
            supported_rules = supported_rules.saturating_add(1);
        }
    }

    Ok(ProfileImportSummary {
        profile: ValidationProfile {
            identity: ProfileIdentity {
                id: profile_id,
                name: profile_name.unwrap_or_else(|| BoundedText::unchecked("veraPDF profile")),
                version: Some(Identifier::new("verapdf-xml")?),
            },
            flavour,
            rules,
        },
        supported_rules,
        unsupported_rules,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum XmlTextTarget {
    None,
    ProfileName,
    RuleDescription,
    RuleTest,
    RuleMessage,
}

#[derive(Debug, Default)]
struct XmlRuleBuilder {
    object_type: Option<ObjectTypeName>,
    unsupported_reason: Option<BoundedText>,
    deferred: bool,
    id: Option<RuleId>,
    description: Option<BoundedText>,
    test: Option<BoundedText>,
    message: Option<BoundedText>,
    references: Vec<SpecReference>,
}

impl XmlRuleBuilder {
    fn from_rule_start(element: &quick_xml::events::BytesStart<'_>) -> Result<Self> {
        let source_object_type = required_attr(element, b"object")?;
        let (object_type, unsupported_reason) = map_verapdf_object_type(&source_object_type)?;
        Ok(Self {
            object_type: Some(object_type),
            unsupported_reason,
            deferred: optional_bool_attr(element, b"deferred")?,
            ..Self::default()
        })
    }

    fn finish(self) -> Result<Rule> {
        let id = self.id.ok_or(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("rule id is missing"),
        })?;
        let object_type = self.object_type.ok_or(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("rule object type is missing"),
        })?;
        let description = self
            .description
            .unwrap_or_else(|| BoundedText::unchecked("Imported veraPDF rule"));
        let source_test = self.test.ok_or(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("rule test is missing"),
        })?;
        let test = if let Some(reason) = self.unsupported_reason {
            RuleExpr::Unsupported {
                fragment: source_test.clone(),
                reason,
            }
        } else {
            parse_imported_expr(source_test.as_str()).unwrap_or_else(|reason| {
                RuleExpr::Unsupported {
                    fragment: source_test.clone(),
                    reason,
                }
            })
        };
        let message = self.message.unwrap_or_else(|| description.clone());
        Ok(Rule {
            id,
            object_type,
            deferred: self.deferred,
            tags: Vec::new(),
            description,
            test,
            error: ErrorTemplate { message },
            references: self.references,
        })
    }
}

fn validate_element(element: &quick_xml::events::BytesStart<'_>) -> Result<()> {
    let name = element.name();
    let name = name.as_ref();
    if !matches!(
        name,
        b"profile"
            | b"details"
            | b"name"
            | b"description"
            | b"hash"
            | b"rules"
            | b"rule"
            | b"id"
            | b"test"
            | b"error"
            | b"message"
            | b"arguments"
            | b"argument"
            | b"references"
            | b"reference"
            | b"variables"
            | b"variable"
            | b"defaultValue"
            | b"value"
    ) {
        return Err(ProfileError::InvalidXml {
            reason: BoundedText::new(
                format!(
                    "unknown profile XML element {}",
                    String::from_utf8_lossy(name)
                ),
                512,
            )
            .unwrap_or_else(|_| BoundedText::unchecked("unknown profile XML element")),
        }
        .into());
    }
    let mut attributes = 0_usize;
    for attr in element.attributes().with_checks(true) {
        let attr = attr.map_err(|error| ProfileError::InvalidXml {
            reason: BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XML attribute error")),
        })?;
        attributes = attributes.checked_add(1).ok_or(ProfileError::InvalidXml {
            reason: BoundedText::unchecked("profile XML attribute count overflow"),
        })?;
        if attributes > MAX_PROFILE_XML_ATTRIBUTES {
            return Err(ProfileError::InvalidXml {
                reason: BoundedText::unchecked("profile XML exceeds attribute limit"),
            }
            .into());
        }
        validate_attribute(name, attr.key.as_ref())?;
    }
    Ok(())
}

fn validate_attribute(element: &[u8], attr: &[u8]) -> Result<()> {
    let allowed = match element {
        b"profile" => matches!(attr, b"flavour" | b"xmlns"),
        b"details" => matches!(attr, b"creator" | b"created"),
        b"rule" => matches!(attr, b"object" | b"deferred" | b"tags"),
        b"id" => matches!(attr, b"specification" | b"clause" | b"testNumber"),
        b"reference" => matches!(attr, b"specification" | b"clause"),
        b"variable" => matches!(attr, b"name" | b"object"),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(ProfileError::InvalidXml {
            reason: BoundedText::new(
                format!(
                    "unknown profile XML attribute {}",
                    String::from_utf8_lossy(attr)
                ),
                512,
            )
            .unwrap_or_else(|_| BoundedText::unchecked("unknown profile XML attribute")),
        }
        .into())
    }
}

fn profile_flavour_attr(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<Option<ValidationFlavour>> {
    for attr in element.attributes().with_checks(true) {
        let attr = attr.map_err(|error| ProfileError::InvalidXml {
            reason: BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XML attribute error")),
        })?;
        if attr.key.as_ref() == b"flavour" {
            let value = String::from_utf8_lossy(attr.value.as_ref()).into_owned();
            return Ok(Some(parse_verapdf_flavour(&value)?));
        }
    }
    Ok(None)
}

fn required_attr(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Result<String> {
    for attr in element.attributes().with_checks(true) {
        let attr = attr.map_err(|error| ProfileError::InvalidXml {
            reason: BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XML attribute error")),
        })?;
        if attr.key.as_ref() == name {
            return Ok(String::from_utf8_lossy(attr.value.as_ref()).into_owned());
        }
    }
    Err(ProfileError::InvalidXml {
        reason: BoundedText::unchecked("required XML attribute is missing"),
    }
    .into())
}

fn optional_bool_attr(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Result<bool> {
    for attr in element.attributes().with_checks(true) {
        let attr = attr.map_err(|error| ProfileError::InvalidXml {
            reason: BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XML attribute error")),
        })?;
        if attr.key.as_ref() == name {
            return match attr.value.as_ref() {
                b"true" => Ok(true),
                b"false" => Ok(false),
                _ => Err(ProfileError::InvalidField {
                    field: "deferred",
                    reason: BoundedText::unchecked("expected true or false"),
                }
                .into()),
            };
        }
    }
    Ok(false)
}

fn rule_id_from_attrs(element: &quick_xml::events::BytesStart<'_>) -> Result<RuleId> {
    let specification = required_attr(element, b"specification")?;
    let clause = required_attr(element, b"clause")?;
    let test_number = required_attr(element, b"testNumber")?;
    let text = format!(
        "{}-{}-{}",
        identifier_fragment(&specification),
        identifier_fragment(&clause),
        identifier_fragment(&test_number)
    );
    Ok(RuleId(Identifier::new(text)?))
}

fn reference_from_attrs(element: &quick_xml::events::BytesStart<'_>) -> Result<SpecReference> {
    Ok(SpecReference {
        specification: BoundedText::new(required_attr(element, b"specification")?, 512)?,
        clause: BoundedText::new(required_attr(element, b"clause")?, 512)?,
    })
}

fn identifier_fragment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

fn parse_verapdf_flavour(value: &str) -> Result<ValidationFlavour> {
    let parts = value.split('_').collect::<Vec<_>>();
    let Some(family) = parts.first().copied() else {
        return Err(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("profile flavour is empty"),
        }
        .into());
    };
    match family {
        "PDFA" => parse_numbered_flavour("pdfa", &parts, "none"),
        "PDFUA" => parse_pdfua_xml_flavour(&parts),
        "WTPDF" => parse_wtpdf_flavour(&parts),
        _ => Err(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("unsupported profile flavour family"),
        }
        .into()),
    }
}

fn parse_pdfua_xml_flavour(parts: &[&str]) -> Result<ValidationFlavour> {
    let part = parts
        .get(1)
        .ok_or(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("missing PDF/UA part"),
        })?
        .parse::<u32>()
        .map_err(|_| ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("PDF/UA part is not numeric"),
        })?;
    let part = NonZeroU32::new(part).ok_or(ProfileError::InvalidField {
        field: "flavour",
        reason: BoundedText::unchecked("PDF/UA part is zero"),
    })?;
    let conformance = if part.get() == 2 { "iso32005" } else { "none" };
    ValidationFlavour::new("pdfua", part, conformance).map_err(Into::into)
}

fn parse_numbered_flavour(
    family: &str,
    parts: &[&str],
    default_conformance: &str,
) -> Result<ValidationFlavour> {
    let part = parts
        .get(1)
        .ok_or(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("missing flavour part"),
        })?
        .parse::<u32>()
        .map_err(|_| ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("flavour part is not numeric"),
        })?;
    let part = NonZeroU32::new(part).ok_or(ProfileError::InvalidField {
        field: "flavour",
        reason: BoundedText::unchecked("flavour part is zero"),
    })?;
    let conformance = parts
        .get(2)
        .copied()
        .unwrap_or(default_conformance)
        .to_ascii_lowercase();
    ValidationFlavour::new(family, part, conformance).map_err(Into::into)
}

fn parse_wtpdf_flavour(parts: &[&str]) -> Result<ValidationFlavour> {
    if parts.len() != 4 || parts.get(1).copied() != Some("1") || parts.get(2).copied() != Some("0")
    {
        return Err(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("expected WTPDF_1_0_<level>"),
        }
        .into());
    }
    let conformance = parts
        .get(3)
        .ok_or(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("missing WTPDF level"),
        })?
        .to_ascii_lowercase();
    ValidationFlavour::new("wtpdf", NonZeroU32::MIN, conformance).map_err(Into::into)
}

fn profile_id_for_flavour(flavour: &ValidationFlavour) -> Result<Identifier> {
    let display = display_flavour(flavour)?;
    Identifier::new(format!("verapdf-{}", display.as_str())).map_err(Into::into)
}

#[allow(
    clippy::too_many_lines,
    reason = "veraPDF object taxonomy mapping is intentionally centralized for schema checks"
)]
fn map_verapdf_object_type(value: &str) -> Result<(ObjectTypeName, Option<BoundedText>)> {
    let mapped = match value {
        "CosDocument" | "PDDocument" | "CosXRef" | "CosTrailer" | "CosIndirect" | "CosInfo" => {
            Some("document")
        }
        "CosStream" => Some("stream"),
        "CosArray"
        | "CosDict"
        | "CosInteger"
        | "CosName"
        | "CosReal"
        | "CosString"
        | "CosTextString"
        | "CosUnicodeName"
        | "CosLang"
        | "CosBBox"
        | "CosActualText"
        | "CosAlt"
        | "CosBM"
        | "CosRenderingIntent"
        | "CosFileSpecification"
        | "CosFilter"
        | "CosIIFilter" => Some("object"),
        "GFCosMetadata"
        | "PDMetadata"
        | "Metadata"
        | "PDFAIdentification"
        | "PDFUAIdentification"
        | "XMPPackage"
        | "MainXMPPackage"
        | "XMPProperty"
        | "XMPLangAlt"
        | "ExtensionSchemaValueType"
        | "ExtensionSchemaDefinition"
        | "ExtensionSchemaProperty"
        | "ExtensionSchemaField"
        | "ExtensionSchemasContainer"
        | "ExtensionSchemaObject" => Some("metadata"),
        "PDCatalog" | "Catalog" => Some("catalog"),
        "PDPage" | "Page" => Some("page"),
        "PDFont"
        | "Font"
        | "PDSimpleFont"
        | "PDTrueTypeFont"
        | "PDType0Font"
        | "PDType1Font"
        | "PDCIDFont"
        | "TrueTypeFontProgram"
        | "Glyph" => Some("font"),
        "PDCMap" | "PDReferencedCMap" | "CMapFile" => Some("cMap"),
        "EmbeddedFile" => Some("embeddedFontFile"),
        "PDAnnotation"
        | "Annotation"
        | "PDAnnot"
        | "PDWidgetAnnot"
        | "PDLinkAnnot"
        | "PDMarkupAnnot"
        | "PDTrapNetAnnot"
        | "PDPrinterMarkAnnot"
        | "PDWatermarkAnnot"
        | "PDSoundAnnot"
        | "PDScreenAnnot"
        | "PDPopupAnnot"
        | "PDMovieAnnot"
        | "PDFileAttachmentAnnot"
        | "PDRubberStampAnnot"
        | "PDRichMediaAnnot"
        | "PD3DAnnot"
        | "PDInkAnnot" => Some("annotation"),
        "PDAction" | "PDNamedAction" | "PDGoToAction" | "PDAdditionalActions" => Some("action"),
        "PDAcroForm" => Some("acroForm"),
        "PDFormField" | "PDTextField" => Some("formField"),
        "OutputIntents" | "OutputIntent" | "PDOutputIntent" => Some("outputIntent"),
        "PDXObject" | "PD3DStream" | "PDMediaClip" | "PDRichMedia" => Some("xObject"),
        "PDXForm" => Some("formXObject"),
        "PDXImage" | "JPEG2000" | "PDMaskImage" => Some("image"),
        "PDContentStream" | "Op_q_gsave" => Some("contentStream"),
        "Op_Undefined" => Some("undefinedOperator"),
        "PDOCConfig" => Some("optionalContentProperties"),
        "PDPerms" => Some("permissions"),
        "PDOutline" => Some("outline"),
        "PDDestination" => Some("destination"),
        "PDExtGState" => Some("extGState"),
        "PDDeviceN" | "PDICCBasedCMYK" | "PDDeviceRGB" | "PDDeviceGray" | "PDDeviceCMYK"
        | "PDSeparation" | "PDHalftone" | "PDGroup" => Some("colorSpace"),
        "ICCProfile" | "ICCOutputProfile" | "ICCInputProfile" => Some("iccProfile"),
        "PDPattern" | "PDTilingPattern" | "PDShadingPattern" => Some("pattern"),
        "PDShading" => Some("shading"),
        "PDFunction" => Some("function"),
        "PDStructTreeRoot" => Some("structureTreeRoot"),
        "PDStructElem"
        | "SEDocument"
        | "SEDocumentFragment"
        | "SEPart"
        | "SEArt"
        | "SESect"
        | "SEDiv"
        | "SEBlockQuote"
        | "SECaption"
        | "SETOC"
        | "SETOCI"
        | "SEIndex"
        | "SENonStruct"
        | "SEPrivate"
        | "SEP"
        | "SEH"
        | "SEHn"
        | "SEH1"
        | "SEH2"
        | "SEH3"
        | "SEH4"
        | "SEH5"
        | "SEH6"
        | "SEL"
        | "SELI"
        | "SELbl"
        | "SELBody"
        | "SETable"
        | "SETR"
        | "SETH"
        | "SETD"
        | "SETHead"
        | "SETBody"
        | "SETFoot"
        | "SESpan"
        | "SEQuote"
        | "SENote"
        | "SEReference"
        | "SEBibEntry"
        | "SECode"
        | "SELink"
        | "SEAnnot"
        | "SERuby"
        | "SEWarichu"
        | "SEFigure"
        | "SEFormula"
        | "SEForm"
        | "SEArtifact"
        | "SEStrong"
        | "SEEm"
        | "SETitle"
        | "SEFENote"
        | "SEAside"
        | "SESub"
        | "SEMathMLStructElem"
        | "SEMarkedContent"
        | "SESimpleContentItem"
        | "SEGraphicContentItem"
        | "SETableCell"
        | "SENonStandard"
        | "SETextItem"
        | "SEWT"
        | "SEWP"
        | "SERT"
        | "SERP"
        | "SERB" => Some("structureElement"),
        "PDSignature" | "PDSigRef" | "PKCSDataObject" => Some("signature"),
        "PDEncryption" => Some("security"),
        _ => None,
    };
    if let Some(mapped) = mapped {
        Ok((ObjectTypeName::new(mapped)?, None))
    } else {
        Ok((
            ObjectTypeName::new("document")?,
            Some(BoundedText::new(
                format!("unsupported veraPDF object type {value}"),
                512,
            )?),
        ))
    }
}

fn parse_imported_expr(input: &str) -> std::result::Result<RuleExpr, BoundedText> {
    let mut parser = ExprParser::new(input);
    let expr = parser.parse_conditional()?;
    if matches!(expr, RuleExpr::Unsupported { .. }) {
        return Ok(expr);
    }
    parser.skip_ws();
    if parser.remaining().is_empty() {
        Ok(expr)
    } else {
        Err(BoundedText::unchecked("trailing expression input"))
    }
}

#[derive(Debug)]
struct ExprParser<'a> {
    input: &'a str,
    offset: usize,
}

impl<'a> ExprParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, offset: 0 }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.offset..]
    }

    fn skip_ws(&mut self) {
        while self
            .remaining()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.offset = self.offset.saturating_add(1);
        }
    }

    fn consume(&mut self, token: &str) -> bool {
        self.skip_ws();
        if self.remaining().starts_with(token) {
            self.offset = self.offset.saturating_add(token.len());
            true
        } else {
            false
        }
    }

    fn parse_conditional(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let condition = self.parse_or()?;
        if self.consume("?") {
            let when_true = self.parse_conditional()?;
            if !self.consume(":") {
                return Err(BoundedText::unchecked("missing ternary separator"));
            }
            let when_false = self.parse_conditional()?;
            Ok(RuleExpr::Conditional {
                condition: Box::new(condition),
                when_true: Box::new(when_true),
                when_false: Box::new(when_false),
            })
        } else {
            Ok(condition)
        }
    }

    fn parse_or(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let mut expr = self.parse_and()?;
        while self.consume("||") {
            let right = self.parse_and()?;
            expr = RuleExpr::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let mut expr = self.parse_comparison()?;
        while self.consume("&&") {
            let right = self.parse_comparison()?;
            expr = RuleExpr::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let left = self.parse_additive()?;
        let op = if self.consume("==") {
            Some(BinaryOp::Eq)
        } else if self.consume("!=") {
            Some(BinaryOp::Ne)
        } else if self.consume("<=") {
            Some(BinaryOp::Le)
        } else if self.consume(">=") {
            Some(BinaryOp::Ge)
        } else if self.consume("<") {
            Some(BinaryOp::Lt)
        } else if self.consume(">") {
            Some(BinaryOp::Gt)
        } else {
            None
        };
        if let Some(op) = op {
            let right = self.parse_additive()?;
            Ok(RuleExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_additive(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let mut expr = self.parse_multiplicative()?;
        loop {
            let op = if self.consume("+") {
                Some(BinaryOp::Add)
            } else if self.consume("-") {
                Some(BinaryOp::Sub)
            } else {
                None
            };
            let Some(op) = op else {
                return Ok(expr);
            };
            expr = RuleExpr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(self.parse_multiplicative()?),
            };
        }
    }

    fn parse_multiplicative(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.consume("*") {
                Some(BinaryOp::Mul)
            } else if self.consume("/") {
                Some(BinaryOp::Div)
            } else if self.consume("%") {
                Some(BinaryOp::Rem)
            } else {
                None
            };
            let Some(op) = op else {
                return Ok(expr);
            };
            expr = RuleExpr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(self.parse_unary()?),
            };
        }
    }

    fn parse_unary(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        self.skip_ws();
        if self.consume("!") {
            return Ok(RuleExpr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let expr = self.parse_primary()?;
        self.skip_ws();
        if self.consume(".") {
            if self.consume("length") && self.consume("(") && self.consume(")") {
                return Ok(RuleExpr::Call {
                    function: BuiltinFunction::Size,
                    args: vec![expr],
                });
            }
            if self.consume("test") && self.consume("(") {
                let arg = self.parse_conditional()?;
                if !self.consume(")") {
                    return Err(BoundedText::unchecked("missing call closing parenthesis"));
                }
                return Ok(RuleExpr::Call {
                    function: BuiltinFunction::Matches,
                    args: vec![expr, arg],
                });
            }
            return Ok(RuleExpr::Unsupported {
                fragment: BoundedText::new(self.input, MAX_PROFILE_STRING_BYTES)
                    .map_err(|_| BoundedText::unchecked("expression exceeds limit"))?,
                reason: BoundedText::unchecked(
                    "nested property path has no bound model link in this phase",
                ),
            });
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        self.skip_ws();
        if self.consume("(") {
            let expr = self.parse_conditional()?;
            if !self.consume(")") {
                return Err(BoundedText::unchecked("missing closing parenthesis"));
            }
            return Ok(expr);
        }
        if self.remaining().starts_with('/') {
            return self.parse_regex_literal();
        }
        if self.remaining().starts_with('"') || self.remaining().starts_with('\'') {
            return self.parse_string();
        }
        if self.remaining().starts_with("true") {
            self.offset = self.offset.saturating_add(4);
            return Ok(RuleExpr::Bool { value: true });
        }
        if self.remaining().starts_with("false") {
            self.offset = self.offset.saturating_add(5);
            return Ok(RuleExpr::Bool { value: false });
        }
        if self.remaining().starts_with("null") {
            self.offset = self.offset.saturating_add(4);
            return Ok(RuleExpr::Null);
        }
        if self
            .remaining()
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'-')
        {
            return self.parse_number();
        }
        self.parse_property()
    }

    fn parse_string(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let quote = *self
            .remaining()
            .as_bytes()
            .first()
            .ok_or_else(|| BoundedText::unchecked("expected string quote"))?;
        self.offset = self.offset.saturating_add(1);
        let start = self.offset;
        while let Some(byte) = self.remaining().as_bytes().first() {
            if *byte == quote {
                let value = &self.input[start..self.offset];
                self.offset = self.offset.saturating_add(1);
                return Ok(RuleExpr::String {
                    value: BoundedText::new(value, MAX_PROFILE_STRING_BYTES)
                        .map_err(|_| BoundedText::unchecked("string literal exceeds limit"))?,
                });
            }
            self.offset = self.offset.saturating_add(1);
        }
        Err(BoundedText::unchecked("unterminated string literal"))
    }

    fn parse_regex_literal(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        self.offset = self.offset.saturating_add(1);
        let start = self.offset;
        let mut escaped = false;
        while let Some(byte) = self.remaining().as_bytes().first() {
            if *byte == b'/' && !escaped {
                let value = &self.input[start..self.offset];
                self.offset = self.offset.saturating_add(1);
                return Ok(RuleExpr::String {
                    value: BoundedText::new(value, MAX_REGEX_PATTERN_BYTES)
                        .map_err(|_| BoundedText::unchecked("regex literal exceeds limit"))?,
                });
            }
            escaped = *byte == b'\\' && !escaped;
            if *byte != b'\\' {
                escaped = false;
            }
            self.offset = self.offset.saturating_add(1);
        }
        Err(BoundedText::unchecked("unterminated regex literal"))
    }

    fn parse_number(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let start = self.offset;
        while let Some(byte) = self.remaining().as_bytes().first() {
            if byte.is_ascii_digit() || matches!(*byte, b'-' | b'.') {
                self.offset = self.offset.saturating_add(1);
            } else {
                break;
            }
        }
        let value = self.input[start..self.offset]
            .parse::<f64>()
            .map_err(|_| BoundedText::unchecked("invalid number literal"))?;
        Ok(RuleExpr::Number { value })
    }

    fn parse_property(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        let first = self.parse_identifier()?;
        if self.consume("(") {
            let mut args = Vec::new();
            if !self.consume(")") {
                loop {
                    args.push(self.parse_conditional()?);
                    if self.consume(")") {
                        break;
                    }
                    if !self.consume(",") {
                        return Err(BoundedText::unchecked(
                            "missing function argument separator",
                        ));
                    }
                }
            }
            return Ok(RuleExpr::Call {
                function: builtin_function(&first)?,
                args,
            });
        }
        let mut parts = vec![property_name_from_source(&first)?];
        while self.consume(".") {
            let member = self.parse_identifier()?;
            parts.push(property_name_from_source(&member)?);
        }
        Ok(RuleExpr::Property {
            path: PropertyPath::new(parts),
        })
    }

    fn parse_identifier(&mut self) -> std::result::Result<String, BoundedText> {
        let start = self.offset;
        while let Some(byte) = self.remaining().as_bytes().first() {
            if byte.is_ascii_alphanumeric() || *byte == b'_' {
                self.offset = self.offset.saturating_add(1);
            } else {
                break;
            }
        }
        if start == self.offset {
            return Err(BoundedText::unchecked("expected expression"));
        }
        Ok(self.input[start..self.offset].to_owned())
    }
}

fn property_name_from_source(value: &str) -> std::result::Result<PropertyName, BoundedText> {
    PropertyName::new(map_verapdf_property(value))
        .map_err(|_| BoundedText::unchecked("invalid property"))
}

fn builtin_function(value: &str) -> std::result::Result<BuiltinFunction, BoundedText> {
    match value {
        "hasParseFact" => Ok(BuiltinFunction::HasParseFact),
        "size" => Ok(BuiltinFunction::Size),
        "isEmpty" => Ok(BuiltinFunction::IsEmpty),
        "contains" => Ok(BuiltinFunction::Contains),
        "all" => Ok(BuiltinFunction::All),
        "exists" => Ok(BuiltinFunction::Exists),
        "matches" => Ok(BuiltinFunction::Matches),
        _ => Err(BoundedText::unchecked("unsupported built-in function")),
    }
}

fn map_verapdf_property(value: &str) -> &str {
    match value {
        "Length" => "declaredLength",
        "realLength" => "discoveredLength",
        "isEncrypted" => "encrypted",
        "containsMetadata" => "hasMetadata",
        "isCatalogMetadata" => "catalogMetadata",
        other => other,
    }
}

impl From<CosObject> for ModelValue {
    fn from(value: CosObject) -> Self {
        match value {
            CosObject::Boolean(value) => Self::Bool(value),
            CosObject::Integer(value) => {
                i32::try_from(value).map_or(Self::Null, |bounded| Self::Number(f64::from(bounded)))
            }
            CosObject::Real(value) => Self::Number(value),
            CosObject::Name(name) => Self::String(BoundedText::unchecked(
                String::from_utf8_lossy(name.as_bytes()).into_owned(),
            )),
            CosObject::String(value) => Self::String(BoundedText::unchecked(
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )),
            CosObject::Reference(value) => Self::ObjectKey(value),
            CosObject::Array(values) => {
                Self::List(values.into_iter().map(ModelValue::from).collect())
            }
            CosObject::Null | CosObject::Dictionary(_) | CosObject::Stream(_) => Self::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use super::{BuiltinProfileRepository, DefaultRuleEvaluator, ProfileRepository, RuleEvaluator};
    use crate::{FlavourSelection, Parser, Validator};

    const MINIMAL_PDF: &[u8] = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";

    #[derive(Debug)]
    struct StaticRepo(super::ValidationProfile);

    impl super::ProfileRepository for StaticRepo {
        fn profiles_for(
            &self,
            _selection: &crate::FlavourSelection,
        ) -> crate::Result<Vec<super::ValidationProfile>> {
            Ok(vec![self.0.clone()])
        }
    }

    #[test]
    fn test_should_return_builtin_profile_for_default_auto_selection() -> crate::Result<()> {
        let profiles = BuiltinProfileRepository::new().profiles_for(&FlavourSelection::default())?;

        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles.first().map(|profile| profile.rules.len()),
            Some(10)
        );
        assert_eq!(
            profiles.first().map(|profile| profile.identity.id.as_str()),
            Some("pdfv-m4")
        );
        Ok(())
    }

    #[test]
    fn test_should_return_no_builtin_profile_for_auto_without_default() -> crate::Result<()> {
        let profiles = BuiltinProfileRepository::new()
            .profiles_for(&FlavourSelection::Auto { default: None })?;

        assert!(profiles.is_empty());
        Ok(())
    }

    #[cfg(feature = "custom-profiles")]
    #[test]
    fn test_should_import_representative_verapdf_xml_rules() -> crate::Result<()> {
        let import = super::import_verapdf_profile_xml(
            crate::generated_profiles::GENERATED_PROFILE_SOURCES
                .iter()
                .find(|source| source.display_flavour == "pdfa-1b")
                .ok_or(crate::ProfileError::UnsupportedSelection)?
                .xml,
        )?;

        assert!(import.profile.rules.len() > 100);
        assert!(import.supported_rules > 0);
        assert!(import.unsupported_rules > 0);
        assert_eq!(import.profile.identity.id.as_str(), "verapdf-pdfa-1b");
        assert!(
            import
                .profile
                .rules
                .iter()
                .any(|rule| !rule.references.is_empty())
        );
        Ok(())
    }

    #[cfg(feature = "custom-profiles")]
    #[test]
    fn test_should_map_verapdf_undefined_operator_to_sparse_family() -> crate::Result<()> {
        let import = super::import_verapdf_profile_xml(
            crate::generated_profiles::GENERATED_PROFILE_SOURCES
                .iter()
                .find(|source| source.display_flavour == "pdfa-1b")
                .ok_or(crate::ProfileError::UnsupportedSelection)?
                .xml,
        )?;

        let rule = import
            .profile
            .rules
            .iter()
            .find(|rule| rule.id.0.as_str() == "iso-19005-1-6-2-10-1")
            .ok_or(crate::ProfileError::UnsupportedSelection)?;

        assert_eq!(rule.object_type.as_str(), "undefinedOperator");
        Ok(())
    }

    #[test]
    fn test_should_list_every_generated_builtin_profile_with_coverage() -> crate::Result<()> {
        let profiles = BuiltinProfileRepository::new().list_profiles()?;

        assert_eq!(
            profiles.len(),
            crate::generated_profiles::GENERATED_PROFILE_SOURCES.len() + 1
        );
        assert!(profiles.iter().any(|profile| {
            profile.identity.id.as_str() == "verapdf-pdfua-2-iso32005"
                && profile.display_flavour.as_str() == "pdfua-2-iso32005"
                && profile.coverage.total_rules > 0
        }));
        assert!(profiles.iter().any(|profile| {
            profile.identity.id.as_str() == "verapdf-wtpdf-1-0-reuse"
                && profile.source_pin.as_str() == crate::generated_profiles::VERA_PDF_LIBRARY_PIN
        }));
        Ok(())
    }

    #[test]
    fn test_should_improve_m6_official_profile_coverage_for_accessibility_profiles()
    -> crate::Result<()> {
        let profiles = BuiltinProfileRepository::new().list_profiles()?;

        for display_flavour in [
            "pdfua-2-iso32005",
            "wtpdf-1-0-accessibility",
            "wtpdf-1-0-reuse",
        ] {
            let profile = profiles
                .iter()
                .find(|profile| profile.display_flavour.as_str() == display_flavour)
                .ok_or(crate::ProfileError::UnsupportedSelection)?;
            assert!(
                profile.coverage.executable_rules.saturating_mul(100)
                    >= profile.coverage.total_rules.saturating_mul(90),
                "{display_flavour} coverage is {:?}",
                profile.coverage
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_reject_inexact_pdfua_2_flavour_selection() -> crate::Result<()> {
        let flavour = crate::ValidationFlavour::new(
            "pdfua",
            std::num::NonZeroU32::new(2).ok_or(crate::ProfileError::UnsupportedSelection)?,
            "wrong",
        )?;
        let result =
            BuiltinProfileRepository::new().profiles_for(&FlavourSelection::Explicit { flavour });

        assert!(matches!(
            result,
            Err(crate::PdfvError::Profile(
                crate::ProfileError::UnsupportedSelection
            ))
        ));
        Ok(())
    }

    #[test]
    fn test_should_load_and_validate_every_generated_builtin_profile() -> crate::Result<()> {
        for source in crate::generated_profiles::GENERATED_PROFILE_SOURCES {
            let flavour = super::parse_display_flavour(source.display_flavour)?;
            let report = Validator::new(
                crate::ValidationOptions::builder()
                    .flavour(FlavourSelection::Explicit { flavour })
                    .build(),
            )?
            .validate_reader(Cursor::new(MINIMAL_PDF), crate::InputName::memory())?;

            assert_eq!(report.status, crate::ValidationStatus::Incomplete);
            assert_eq!(
                report
                    .profile_reports
                    .first()
                    .map(|profile| profile.profile.id.as_str()),
                Some(source.id)
            );
            assert!(
                report
                    .profile_reports
                    .first()
                    .is_some_and(|profile| !profile.unsupported_rules.is_empty())
            );
            assert!(report.profile_reports.first().is_some_and(|profile| {
                profile
                    .unsupported_rules
                    .iter()
                    .any(|rule| !rule.references.is_empty())
            }));
        }
        Ok(())
    }

    #[cfg(feature = "custom-profiles")]
    #[test]
    fn test_should_load_custom_xml_profile() -> crate::Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<profile flavour="PDFA_1_B">
  <details><name>Custom smoke profile</name></details>
  <rules>
    <rule object="CosDocument">
      <id specification="LOCAL" clause="1" testNumber="1"/>
      <description>Catalog must be present</description>
      <test>hasCatalog == true</test>
      <error><message>Catalog is missing</message></error>
    </rule>
  </rules>
</profile>"#;
        let import = super::import_verapdf_profile_xml(xml)?;

        assert_eq!(import.profile.rules.len(), 1);
        assert_eq!(import.supported_rules, 1);
        assert_eq!(import.unsupported_rules, 0);
        Ok(())
    }

    #[test]
    fn test_should_evaluate_m0_document_rules() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let document = Parser::default().parse(Cursor::new(bytes))?;
        let model = crate::validation::DocumentModel::new(&document);
        let object = crate::ModelObjectRef::Document(model);
        let profile = BuiltinProfileRepository::new()
            .profiles_for(&FlavourSelection::default())?
            .remove(0);
        let mut evaluator = DefaultRuleEvaluator::new(crate::ResourceLimits::default());

        for rule in profile
            .rules
            .iter()
            .filter(|rule| rule.object_type.as_str() == "document")
        {
            let outcome = evaluator.evaluate(object.clone(), rule)?;
            assert_eq!(outcome, super::RuleOutcome::Passed);
        }
        Ok(())
    }

    #[test]
    fn test_should_validate_reader_end_to_end() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let report = Validator::new(crate::ValidationOptions::default())?
            .validate_reader(Cursor::new(bytes), crate::InputName::memory())?;

        assert_eq!(report.status, crate::ValidationStatus::Valid);
        Ok(())
    }

    #[test]
    fn test_should_validate_stream_with_declared_length_and_eol() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
2 0 obj
<< /Length 4 >>
stream
abc
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let report = Validator::new(crate::ValidationOptions::default())?
            .validate_reader(Cursor::new(bytes), crate::InputName::memory())?;

        assert_eq!(report.status, crate::ValidationStatus::Valid);
        Ok(())
    }

    #[test]
    fn test_should_apply_m4_feature_fact_rules_to_linked_objects() -> crate::Result<()> {
        let report = Validator::new(crate::ValidationOptions::default())?
            .validate_reader(Cursor::new(m4_feature_pdf()), crate::InputName::memory())?;
        let profile =
            report
                .profile_reports
                .first()
                .ok_or(crate::ValidationError::LimitExceeded {
                    limit: "profile_reports",
                })?;

        assert_eq!(
            report.status,
            crate::ValidationStatus::Valid,
            "{profile:#?}"
        );
        assert_eq!(profile.rules_executed, 12);
        Ok(())
    }

    #[test]
    fn test_should_report_imported_derived_property_as_unsupported() -> crate::Result<()> {
        let rule = super::Rule {
            id: crate::RuleId(crate::Identifier::new("derived-font-name")?),
            object_type: super::ObjectTypeName::new("font")?,
            deferred: false,
            tags: Vec::new(),
            description: crate::BoundedText::new("derived font name", 64)?,
            test: super::RuleExpr::Binary {
                op: super::BinaryOp::Eq,
                left: Box::new(super::property_expr("fontName")?),
                right: Box::new(super::RuleExpr::Null),
            },
            error: super::ErrorTemplate {
                message: crate::BoundedText::new("derived font name", 64)?,
            },
            references: Vec::new(),
        };
        let profile = super::ValidationProfile {
            identity: crate::ProfileIdentity {
                id: crate::Identifier::new("derived-property")?,
                name: crate::BoundedText::new("derived property", 64)?,
                version: None,
            },
            flavour: super::pdfa_1b_flavour()?,
            rules: vec![rule],
        };
        let validator = Validator::with_profiles(
            crate::ValidationOptions::default(),
            Arc::new(StaticRepo(profile)),
        )?;
        let report =
            validator.validate_reader(Cursor::new(m4_feature_pdf()), crate::InputName::memory())?;
        let profile =
            report
                .profile_reports
                .first()
                .ok_or(crate::ValidationError::LimitExceeded {
                    limit: "profile_reports",
                })?;

        assert_eq!(report.status, crate::ValidationStatus::Incomplete);
        assert_eq!(profile.unsupported_rules.len(), 1);
        Ok(())
    }

    #[test]
    fn test_should_fail_m4_feature_fact_rule_on_invalid_font() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>
endobj
4 0 obj
<< /Type /Font /BaseFont /Helvetica >>
endobj
5 0 obj
<< /Length 4 >>
stream
q Q
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let report = Validator::new(crate::ValidationOptions::default())?
            .validate_reader(Cursor::new(bytes), crate::InputName::memory())?;
        let profile =
            report
                .profile_reports
                .first()
                .ok_or(crate::ValidationError::LimitExceeded {
                    limit: "profile_reports",
                })?;

        assert_eq!(report.status, crate::ValidationStatus::Invalid);
        assert!(profile.failed_assertions.iter().any(|assertion| {
            assertion.rule_id.0.as_str() == "m4-font-subtype-present"
                && assertion
                    .object_context
                    .as_ref()
                    .is_some_and(|context| context.as_str() == "root/page[0]/font[F1]")
        }));
        Ok(())
    }

    fn m4_feature_pdf() -> &'static [u8] {
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
<< /Length 0 >>
stream
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

    #[test]
    fn test_should_reject_unsupported_rule_ir_silently_fallbacks() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let document = Parser::default().parse(Cursor::new(bytes))?;
        let model = crate::validation::DocumentModel::new(&document);
        let object = crate::ModelObjectRef::Document(model);
        let mut evaluator = DefaultRuleEvaluator::new(crate::ResourceLimits::default());
        let nested_rule = super::Rule {
            id: crate::RuleId(crate::Identifier::new("bad-nested")?),
            object_type: super::ObjectTypeName::new("document")?,
            deferred: false,
            tags: Vec::new(),
            description: crate::BoundedText::new("nested", 32)?,
            test: super::RuleExpr::Property {
                path: super::PropertyPath::new(vec![
                    super::PropertyName::new("headerOffset")?,
                    super::PropertyName::new("extra")?,
                ]),
            },
            error: super::ErrorTemplate {
                message: crate::BoundedText::new("nested", 32)?,
            },
            references: Vec::new(),
        };
        let arity_rule = super::Rule {
            id: crate::RuleId(crate::Identifier::new("bad-arity")?),
            object_type: super::ObjectTypeName::new("document")?,
            deferred: false,
            tags: Vec::new(),
            description: crate::BoundedText::new("arity", 32)?,
            test: super::RuleExpr::Call {
                function: super::BuiltinFunction::HasParseFact,
                args: vec![
                    super::RuleExpr::String {
                        value: crate::BoundedText::new("header", 32)?,
                    },
                    super::RuleExpr::String {
                        value: crate::BoundedText::new("extra", 32)?,
                    },
                ],
            },
            error: super::ErrorTemplate {
                message: crate::BoundedText::new("arity", 32)?,
            },
            references: Vec::new(),
        };

        assert!(evaluator.evaluate(object.clone(), &nested_rule).is_err());
        assert!(evaluator.evaluate(object, &arity_rule).is_err());
        Ok(())
    }

    #[test]
    fn test_should_report_unsupported_rule_as_incomplete() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let rule = super::Rule {
            id: crate::RuleId(crate::Identifier::new("unsupported")?),
            object_type: super::ObjectTypeName::new("document")?,
            deferred: false,
            tags: Vec::new(),
            description: crate::BoundedText::new("unsupported", 64)?,
            test: super::RuleExpr::Property {
                path: super::PropertyPath::new(vec![
                    super::PropertyName::new("headerOffset")?,
                    super::PropertyName::new("extra")?,
                ]),
            },
            error: super::ErrorTemplate {
                message: crate::BoundedText::new("unsupported", 64)?,
            },
            references: Vec::new(),
        };
        let profile = super::ValidationProfile {
            identity: crate::ProfileIdentity {
                id: crate::Identifier::new("test")?,
                name: crate::BoundedText::new("test", 64)?,
                version: None,
            },
            flavour: super::pdfa_1b_flavour()?,
            rules: vec![rule],
        };
        let validator = Validator::with_profiles(
            crate::ValidationOptions::default(),
            Arc::new(StaticRepo(profile)),
        )?;
        let report = validator.validate_reader(Cursor::new(bytes), crate::InputName::memory())?;

        assert_eq!(report.status, crate::ValidationStatus::Incomplete);
        assert_eq!(
            report
                .profile_reports
                .first()
                .map(|profile| profile.unsupported_rules.len()),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn test_should_parse_phase_13_expression_surface() -> crate::Result<()> {
        let modulo = super::parse_imported_expr("hexCount % 2 == 0")
            .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?;
        let ternary = super::parse_imported_expr(
            "gPageOutputCS == null ? gDocumentOutputCS == 'RGB ' : gPageOutputCS == 'RGB '",
        )
        .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?;
        let regex = super::parse_imported_expr(r"/^%PDF-2\.[0-9]$/.test(header)")
            .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?;
        let call = super::parse_imported_expr("contains(entries, 'UR3') == false")
            .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?;

        assert!(matches!(modulo, super::RuleExpr::Binary { .. }));
        assert!(matches!(ternary, super::RuleExpr::Conditional { .. }));
        assert!(matches!(regex, super::RuleExpr::Call { .. }));
        assert!(matches!(call, super::RuleExpr::Binary { .. }));
        Ok(())
    }

    #[test]
    fn test_should_parse_nested_property_paths() -> crate::Result<()> {
        let expr = super::parse_imported_expr("metadata.schema.part == 1")
            .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?;

        assert!(matches!(expr, super::RuleExpr::Binary { .. }));
        Ok(())
    }

    #[test]
    fn test_should_evaluate_nested_property_path_through_model_link() -> crate::Result<()> {
        let profile = super::ValidationProfile {
            identity: crate::ProfileIdentity {
                id: crate::Identifier::new("nested-path")?,
                name: crate::BoundedText::new("nested path", 64)?,
                version: None,
            },
            flavour: super::pdfa_1b_flavour()?,
            rules: vec![super::Rule {
                id: crate::RuleId(crate::Identifier::new("catalog-no-metadata")?),
                object_type: super::ObjectTypeName::new("document")?,
                deferred: false,
                tags: Vec::new(),
                description: crate::BoundedText::new("catalog has no metadata", 64)?,
                test: super::parse_imported_expr("catalog.hasMetadata == false")
                    .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?,
                error: super::ErrorTemplate {
                    message: crate::BoundedText::new("catalog has metadata", 64)?,
                },
                references: Vec::new(),
            }],
        };
        let validator = Validator::with_profiles(
            crate::ValidationOptions::default(),
            Arc::new(StaticRepo(profile)),
        )?;
        let report =
            validator.validate_reader(Cursor::new(MINIMAL_PDF), crate::InputName::memory())?;

        assert_eq!(report.status, crate::ValidationStatus::Valid);
        Ok(())
    }

    #[test]
    fn test_should_evaluate_arithmetic_ternary_and_regex_builtins() -> crate::Result<()> {
        let bytes = br"%PDF-2.0
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let document = Parser::default().parse(Cursor::new(bytes))?;
        let model = crate::validation::DocumentModel::new(&document);
        let object = crate::ModelObjectRef::Document(model);
        let mut evaluator = DefaultRuleEvaluator::new(crate::ResourceLimits::default());
        let rule = super::Rule {
            id: crate::RuleId(crate::Identifier::new("expr-surface")?),
            object_type: super::ObjectTypeName::new("document")?,
            deferred: false,
            tags: Vec::new(),
            description: crate::BoundedText::new("expr", 32)?,
            test: super::RuleExpr::Binary {
                op: super::BinaryOp::And,
                left: Box::new(
                    super::parse_imported_expr("5 % 2 == 1")
                        .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?,
                ),
                right: Box::new(
                    super::parse_imported_expr(r"/^%PDF-2\.[0-9]$/.test(header)")
                        .map_err(|reason| crate::ProfileError::UnsupportedRule { reason })?,
                ),
            },
            error: super::ErrorTemplate {
                message: crate::BoundedText::new("expr", 32)?,
            },
            references: Vec::new(),
        };

        assert_eq!(
            evaluator.evaluate(object, &rule)?,
            super::RuleOutcome::Passed
        );
        Ok(())
    }

    #[test]
    fn test_should_apply_failed_assertion_cap_per_rule() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let mut rules = Vec::new();
        for id in ["fail-a", "fail-b"] {
            rules.push(super::Rule {
                id: crate::RuleId(crate::Identifier::new(id)?),
                object_type: super::ObjectTypeName::new("document")?,
                deferred: false,
                tags: Vec::new(),
                description: crate::BoundedText::new(id, 64)?,
                test: super::RuleExpr::Bool { value: false },
                error: super::ErrorTemplate {
                    message: crate::BoundedText::new(id, 64)?,
                },
                references: Vec::new(),
            });
        }
        let profile = super::ValidationProfile {
            identity: crate::ProfileIdentity {
                id: crate::Identifier::new("test")?,
                name: crate::BoundedText::new("test", 64)?,
                version: None,
            },
            flavour: super::pdfa_1b_flavour()?,
            rules,
        };
        let validator = Validator::with_profiles(
            crate::ValidationOptions::default(),
            Arc::new(StaticRepo(profile)),
        )?;
        let report = validator.validate_reader(Cursor::new(bytes), crate::InputName::memory())?;

        assert_eq!(
            report
                .profile_reports
                .first()
                .map(|profile| profile.failed_assertions.len()),
            Some(2)
        );
        Ok(())
    }
}
