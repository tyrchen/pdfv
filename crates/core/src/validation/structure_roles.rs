//! Rule scope for structure roles declared in the imported profiles.

use crate::accessibility::{AccessibilityNode, MATH_NAMESPACE};

const ROLES: &[&str] = &[
    "SEAnnot",
    "SEArt",
    "SEArtifact",
    "SEAside",
    "SEBibEntry",
    "SEBlockQuote",
    "SECaption",
    "SECode",
    "SEDiv",
    "SEDocument",
    "SEDocumentFragment",
    "SEEm",
    "SEFENote",
    "SEFigure",
    "SEForm",
    "SEFormula",
    "SEH",
    "SEIndex",
    "SEL",
    "SELBody",
    "SELI",
    "SELbl",
    "SELink",
    "SENonStruct",
    "SENote",
    "SEP",
    "SEPart",
    "SEPrivate",
    "SEQuote",
    "SERB",
    "SERP",
    "SERT",
    "SEReference",
    "SERuby",
    "SESect",
    "SESpan",
    "SEStrong",
    "SESub",
    "SETBody",
    "SETD",
    "SETFoot",
    "SETH",
    "SETHead",
    "SETOC",
    "SETOCI",
    "SETR",
    "SETable",
    "SETitle",
    "SEWP",
    "SEWT",
    "SEWarichu",
];

const GROUPS: &[&str] = &["SEHn", "SETableCell", "SEMathMLStructElem", "SENonStandard"];

pub(super) fn families() -> impl Iterator<Item = &'static str> {
    ROLES.iter().chain(GROUPS).copied()
}

pub(crate) fn rule_family(name: &str) -> Option<&'static str> {
    families().find(|family| *family == name)
}

pub(super) fn is_pass_through(node: &AccessibilityNode) -> bool {
    !node.non_standard_role && matches!(node.normalized_role.as_str(), "NonStruct" | "Div" | "Part")
}

pub(super) fn applicable(node: &AccessibilityNode) -> impl Iterator<Item = &'static str> {
    let role = node.normalized_role.as_str();
    let primary = if node.standard_namespace_url.as_deref() == Some(MATH_NAMESPACE) {
        "SEMathMLStructElem"
    } else if node.non_standard_role {
        "SENonStandard"
    } else if role.strip_prefix('H').is_some_and(|suffix| {
        suffix
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b'1'..=b'9'))
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        "SEHn"
    } else {
        ROLES
            .iter()
            .copied()
            .find(|family| family.strip_prefix("SE") == Some(role))
            .unwrap_or("SENonStandard")
    };
    let table_cell = matches!(primary, "SETH" | "SETD").then_some("SETableCell");
    std::iter::once(primary).chain(table_cell)
}
