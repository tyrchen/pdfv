//! Built-in validation profiles and bounded rule expression evaluation.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::{
    AssertionStatus, BoundedText, CosObject, FlavourSelection, Identifier, ObjectKey, ParseFact,
    ProfileError, ProfileIdentity, ResourceLimits, Result, RuleId, StreamFact, ValidationFlavour,
};

const MAX_RULE_INSTRUCTIONS: u64 = 512;
const MAX_RULE_DEPTH: u32 = 32;

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
                Ok(vec![m0_profile(flavour.clone())?])
            }
            FlavourSelection::CustomProfile { .. } => {
                Err(ProfileError::UnsupportedSelection.into())
            }
        }
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
        object: crate::ModelObjectRef<'_>,
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
        }
    }

    fn eval_binary(
        &mut self,
        object: crate::ModelObjectRef<'_>,
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
            BinaryOp::And | BinaryOp::Or => false,
        };
        Ok(ModelValue::Bool(result))
    }

    fn eval_call(
        &mut self,
        object: crate::ModelObjectRef<'_>,
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
        let value = self.eval(object, &rule.test, 0)?;
        if expect_bool(&value)? {
            Ok(RuleOutcome::Passed)
        } else {
            Ok(RuleOutcome::Failed)
        }
    }
}

fn property(object: crate::ModelObjectRef<'_>, path: &PropertyPath) -> Result<ModelValue> {
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
        let object = crate::ModelObjectRef::Document(&model);
        let profile = BuiltinProfileRepository::new()
            .profiles_for(&FlavourSelection::default())?
            .remove(0);
        let mut evaluator = DefaultRuleEvaluator::new(crate::ResourceLimits::default());

        for rule in profile
            .rules
            .iter()
            .filter(|rule| rule.object_type.as_str() == "document")
        {
            let outcome = evaluator.evaluate(object, rule)?;
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
        let object = crate::ModelObjectRef::Document(&model);
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

        assert!(evaluator.evaluate(object, &nested_rule).is_err());
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
