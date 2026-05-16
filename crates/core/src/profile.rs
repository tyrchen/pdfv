//! Built-in validation profiles and bounded rule expression evaluation.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::{
    AssertionStatus, BoundedText, CosObject, FlavourSelection, Identifier, ObjectKey, ParseFact,
    ProfileError, ProfileIdentity, ResourceLimits, Result, RuleId, StreamFact, ValidationFlavour,
};

const MAX_RULE_INSTRUCTIONS: u64 = 512;
const MAX_RULE_DEPTH: u32 = 32;
const MAX_PROFILE_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILE_XML_ELEMENTS: u64 = 100_000;
const MAX_PROFILE_XML_DEPTH: u32 = 32;
const MAX_PROFILE_XML_ATTRIBUTES: usize = 16;
const MAX_PROFILE_RULES: usize = 10_000;
const MAX_PROFILE_STRING_BYTES: usize = 4096;
const VERA_PDF_A_1B_XML: &str = include_str!(
    "../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1B.\
     xml"
);

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

/// Built-in M0 profile repository.
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
        let generated = import_verapdf_profile_xml(VERA_PDF_A_1B_XML)?;
        Ok(vec![
            ProfileCatalogEntry {
                identity: m0_profile(pdfa_1b_flavour()?)?.identity,
                flavour: pdfa_1b_flavour()?,
            },
            ProfileCatalogEntry {
                identity: generated.profile.identity,
                flavour: generated.profile.flavour,
            },
        ])
    }
}

impl ProfileRepository for BuiltinProfileRepository {
    fn profiles_for(&self, selection: &FlavourSelection) -> Result<Vec<ValidationProfile>> {
        match selection {
            FlavourSelection::Auto { default } => {
                let flavour = match default {
                    Some(flavour) => flavour.clone(),
                    None => pdfa_1b_flavour()?,
                };
                ensure_m0_flavour(&flavour)?;
                Ok(vec![m0_profile(flavour)?])
            }
            FlavourSelection::Explicit { flavour } => {
                ensure_m0_flavour(flavour)?;
                Ok(vec![import_verapdf_profile_xml(VERA_PDF_A_1B_XML)?.profile])
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

/// Profile metadata suitable for listing catalogs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileCatalogEntry {
    /// Profile identity.
    pub identity: ProfileIdentity,
    /// Validation flavour.
    pub flavour: ValidationFlavour,
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
}

/// Bounded built-in function.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum BuiltinFunction {
    /// Returns true when a named parse fact exists.
    HasParseFact,
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
pub struct DefaultRuleEvaluator {
    limits: ResourceLimits,
    instructions: u64,
}

impl DefaultRuleEvaluator {
    /// Creates an evaluator.
    #[must_use]
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            instructions: 0,
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
            RuleExpr::Property { path } => property(object, path),
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
                if args.len() != 1 {
                    return Err(type_mismatch("hasParseFact requires exactly one argument").into());
                }
                let Some(first) = args.first() else {
                    return Err(type_mismatch("hasParseFact requires one argument").into());
                };
                let value = self.eval(object, first, depth.saturating_add(1))?;
                let ModelValue::String(name) = value else {
                    return Err(type_mismatch("hasParseFact requires string").into());
                };
                Ok(ModelValue::Bool(has_parse_fact(
                    object.document().parse_facts.as_slice(),
                    name.as_str(),
                )))
            }
        }
    }
}

impl RuleEvaluator for DefaultRuleEvaluator {
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

fn property(object: &crate::ModelObjectRef<'_>, path: &PropertyPath) -> Result<ModelValue> {
    if path.parts().len() != 1 {
        return Err(ProfileError::UnknownProperty {
            property: BoundedText::unchecked("nested property paths are unsupported in M0"),
        }
        .into());
    }
    let Some(name) = path.parts().first() else {
        return Err(ProfileError::UnknownProperty {
            property: BoundedText::unchecked("empty"),
        }
        .into());
    };
    object.property(name)
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
        _ => false,
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

fn ensure_m0_flavour(flavour: &ValidationFlavour) -> Result<()> {
    if flavour == &pdfa_1b_flavour()? {
        Ok(())
    } else {
        Err(ProfileError::UnsupportedSelection.into())
    }
}

fn m0_profile(flavour: ValidationFlavour) -> Result<ValidationProfile> {
    Ok(ValidationProfile {
        identity: ProfileIdentity {
            id: Identifier::new("pdfv-m0")?,
            name: BoundedText::new("pdfv M0 built-in profile", 128)?,
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
    id: Option<RuleId>,
    description: Option<BoundedText>,
    test: Option<BoundedText>,
    message: Option<BoundedText>,
}

impl XmlRuleBuilder {
    fn from_rule_start(element: &quick_xml::events::BytesStart<'_>) -> Result<Self> {
        let source_object_type = required_attr(element, b"object")?;
        let (object_type, unsupported_reason) = map_verapdf_object_type(&source_object_type)?;
        Ok(Self {
            object_type: Some(object_type),
            unsupported_reason,
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
            deferred: false,
            tags: Vec::new(),
            description,
            test,
            error: ErrorTemplate { message },
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
        b"rule" => matches!(attr, b"object" | b"deferred"),
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
    if parts.len() != 3 || parts.first().copied() != Some("PDFA") {
        return Err(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("expected PDFA_<part>_<conformance>"),
        }
        .into());
    }
    let part = parts
        .get(1)
        .ok_or(ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("missing PDF/A part"),
        })?
        .parse::<u32>()
        .map_err(|_| ProfileError::InvalidField {
            field: "flavour",
            reason: BoundedText::unchecked("PDF/A part is not numeric"),
        })?;
    let part = NonZeroU32::new(part).ok_or(ProfileError::InvalidField {
        field: "flavour",
        reason: BoundedText::unchecked("PDF/A part is zero"),
    })?;
    let conformance = parts.get(2).ok_or(ProfileError::InvalidField {
        field: "flavour",
        reason: BoundedText::unchecked("missing conformance"),
    })?;
    ValidationFlavour::new("pdfa", part, conformance.to_ascii_lowercase()).map_err(Into::into)
}

fn profile_id_for_flavour(flavour: &ValidationFlavour) -> Result<Identifier> {
    Identifier::new(format!(
        "verapdf-pdfa-{}{}",
        flavour.part,
        flavour.conformance.as_str()
    ))
    .map_err(Into::into)
}

fn map_verapdf_object_type(value: &str) -> Result<(ObjectTypeName, Option<BoundedText>)> {
    let mapped = match value {
        "CosDocument" | "PDDocument" => Some("document"),
        "CosStream" => Some("stream"),
        "GFCosMetadata" | "PDMetadata" | "Metadata" => Some("metadata"),
        "PDCatalog" | "Catalog" => Some("catalog"),
        "PDPage" | "Page" => Some("page"),
        "PDFont" | "Font" => Some("font"),
        "PDAnnotation" | "Annotation" => Some("annotation"),
        "OutputIntents" | "OutputIntent" => Some("outputIntent"),
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
    if input.contains('?') || input.contains('%') || input.contains('.') {
        return Err(BoundedText::unchecked(
            "expression operator is not supported",
        ));
    }
    let mut parser = ExprParser::new(input);
    let expr = parser.parse_or()?;
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
        let left = self.parse_primary()?;
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
            let right = self.parse_primary()?;
            Ok(RuleExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_primary(&mut self) -> std::result::Result<RuleExpr, BoundedText> {
        self.skip_ws();
        if self.consume("(") {
            let expr = self.parse_or()?;
            if !self.consume(")") {
                return Err(BoundedText::unchecked("missing closing parenthesis"));
            }
            return Ok(expr);
        }
        if self.consume("!") {
            return Ok(RuleExpr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_primary()?),
            });
        }
        if self.remaining().starts_with('"') {
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
        self.offset = self.offset.saturating_add(1);
        let start = self.offset;
        while let Some(byte) = self.remaining().as_bytes().first() {
            if *byte == b'"' {
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
        let name = map_verapdf_property(&self.input[start..self.offset]);
        Ok(RuleExpr::Property {
            path: PropertyPath::new(vec![
                PropertyName::new(name).map_err(|_| BoundedText::unchecked("invalid property"))?,
            ]),
        })
    }
}

fn map_verapdf_property(value: &str) -> &str {
    match value {
        "Length" => "declaredLength",
        "realLength" => "discoveredLength",
        "isEncrypted" => "encrypted",
        "containsMetadata" => "hasMetadata",
        other => other,
    }
}

impl From<CosObject> for ModelValue {
    fn from(value: CosObject) -> Self {
        match value {
            CosObject::Boolean(value) => Self::Bool(value),
            CosObject::Real(value) => Self::Number(value),
            CosObject::Name(name) => Self::String(BoundedText::unchecked(
                String::from_utf8_lossy(name.as_bytes()).into_owned(),
            )),
            CosObject::String(value) => Self::String(BoundedText::unchecked(
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )),
            CosObject::Reference(value) => Self::ObjectKey(value),
            CosObject::Null
            | CosObject::Integer(_)
            | CosObject::Array(_)
            | CosObject::Dictionary(_)
            | CosObject::Stream(_) => Self::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use super::{BuiltinProfileRepository, DefaultRuleEvaluator, ProfileRepository, RuleEvaluator};
    use crate::{FlavourSelection, Parser, Validator};

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
    fn test_should_return_builtin_profile_for_auto_selection() -> crate::Result<()> {
        let profiles = BuiltinProfileRepository::new().profiles_for(&FlavourSelection::default())?;

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles.first().map(|profile| profile.rules.len()), Some(4));
        Ok(())
    }

    #[cfg(feature = "custom-profiles")]
    #[test]
    fn test_should_import_representative_verapdf_xml_rules() -> crate::Result<()> {
        let import = super::import_verapdf_profile_xml(super::VERA_PDF_A_1B_XML)?;

        assert!(import.profile.rules.len() > 100);
        assert!(import.supported_rules > 0);
        assert!(import.unsupported_rules > 0);
        assert_eq!(import.profile.identity.id.as_str(), "verapdf-pdfa-1b");
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
<< /Length 3 >>
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
