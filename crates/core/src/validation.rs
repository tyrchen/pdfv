//! End-to-end validation session and validation model graph.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Instant,
};

use crate::{
    Assertion, BoundedText, BuiltinProfileRepository, ENGINE_VERSION, ErrorArgument, FeatureObject,
    FeatureReport, FeatureValue, Identifier, IndirectObject, InputKind, InputSummary, ModelValue,
    ObjectKey, ObjectLocation, ObjectTypeName, ParseError, ParsedDocument, Parser, PdfName,
    PdfvError, PolicyOperator, PolicyReport, PolicyRule, PolicyRuleResult, PolicySet, PolicyValue,
    ProfileReport, ProfileRepository, PropertyName, ResourceLimits, Result, Rule, RuleEvaluator,
    RuleId, RuleOutcome, TaskDuration, UnsupportedRule, ValidationError, ValidationFlavour,
    ValidationOptions, ValidationReport, ValidationStatus,
    accessibility::{
        AccessibilityArtifact, AccessibilityGraph, AccessibilityMarkedContent, AccessibilityNode,
        AccessibilityNodeId, AccessibilityObjectReference,
    },
    content::{ContentStreamSummary, MarkedContentSpan, OperatorFact, ResourceFamily, ResourceUse},
    profile::DefaultRuleEvaluator,
    xmp::{FlavourDetector, parse_document_xmp},
};

const CATALOG_DIRECT_PROPERTIES: &[&str] = &[
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
const METADATA_DIRECT_PROPERTIES: &[&str] = &["Type", "Subtype", "Filter", "Length"];
const PAGE_INHERITED_PROPERTIES: &[&str] = &[
    "Resources",
    "MediaBox",
    "CropBox",
    "BleedBox",
    "TrimBox",
    "ArtBox",
    "Rotate",
];
const PAGE_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Parent",
    "Contents",
    "Resources",
    "Annots",
    "MediaBox",
    "CropBox",
    "BleedBox",
    "TrimBox",
    "ArtBox",
    "Rotate",
    "UserUnit",
    "Tabs",
    "StructParents",
    "AA",
];
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
    "DescendantFonts",
    "CharProcs",
    "Resources",
    "FontMatrix",
    "FontBBox",
    "DW",
    "W",
    "CIDSystemInfo",
];
const ANNOTATION_DIRECT_PROPERTIES: &[&str] = &[
    "Type", "Subtype", "F", "C", "IC", "AP", "FT", "CA", "A", "AA", "FS",
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
const OUTLINES_DIRECT_PROPERTIES: &[&str] = &[
    "Type", "First", "Last", "Next", "Prev", "Parent", "Count", "Dest", "A",
];
const DESTINATION_DIRECT_PROPERTIES: &[&str] = &["D", "Dest", "A"];
const ACTION_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "S",
    "D",
    "URI",
    "Next",
    "NewWindow",
    "F",
    "FS",
    "Win",
    "Unix",
    "Mac",
];
const FORM_FIELD_DIRECT_PROPERTIES: &[&str] = &[
    "FT", "T", "TU", "TM", "Ff", "V", "DV", "Kids", "Parent", "AA", "A", "AP", "F",
];
const FILE_SPEC_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "FS",
    "F",
    "UF",
    "DOS",
    "Mac",
    "Unix",
    "EF",
    "Desc",
    "CI",
    "AFRelationship",
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
const PATTERN_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "PatternType",
    "PaintType",
    "TilingType",
    "BBox",
    "XStep",
    "YStep",
    "Resources",
    "Matrix",
    "Shading",
];
const SHADING_DIRECT_PROPERTIES: &[&str] = &[
    "ShadingType",
    "ColorSpace",
    "Background",
    "BBox",
    "Function",
    "AntiAlias",
];
const FUNCTION_DIRECT_PROPERTIES: &[&str] = &[
    "FunctionType",
    "Domain",
    "Range",
    "Size",
    "BitsPerSample",
    "Order",
    "Encode",
    "Decode",
];
const CMAP_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "CMapName",
    "CIDSystemInfo",
    "WMode",
    "UseCMap",
];
const COLOR_SPACE_DIRECT_PROPERTIES: &[&str] = &[
    "Type",
    "N",
    "Alternate",
    "Range",
    "Metadata",
    "Filter",
    "Family",
    "Base",
    "HiVal",
    "Lookup",
    "TintTransform",
    "Colorants",
    "Process",
    "Components",
];
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
    "AFRelationship",
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
    "FS",
    "FT",
    "Ff",
    "Fields",
    "Filter",
    "First",
    "FirstChar",
    "Font",
    "FontDescriptor",
    "FontFile",
    "FontFile2",
    "FontFile3",
    "Group",
    "Height",
    "IC",
    "Length1",
    "Length2",
    "Length3",
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
    "Prev",
    "Pattern",
    "PatternType",
    "ProcSet",
    "Process",
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
    "ShadingType",
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
    "UF",
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
    "MediaBox",
    "CropBox",
    "BleedBox",
    "TrimBox",
    "ArtBox",
    "Rotate",
    "UserUnit",
    "Tabs",
    "StructParents",
    "AA",
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
    "embeddedProgramBytes",
    "embeddedProgramCapped",
    "hasSubtype",
    "fontFamily",
    "hasFontDescriptor",
    "hasToUnicode",
    "hasEncoding",
    "hasWidths",
    "hasCIDSystemInfo",
    "hasCharProcs",
    "hasResources",
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
    "DescendantFonts",
    "CharProcs",
    "Resources",
    "FontMatrix",
    "FontBBox",
    "DW",
    "W",
    "CIDSystemInfo",
];
const FONT_DESCRIPTOR_PROPERTIES: &[&str] = &[
    "Type",
    "FontName",
    "Flags",
    "FontBBox",
    "ItalicAngle",
    "Ascent",
    "Descent",
    "CapHeight",
    "StemV",
    "FontFile",
    "FontFile2",
    "FontFile3",
    "hasFontFile",
    "fontProgramBytes",
    "fontProgramCapped",
];
const FONT_PROGRAM_PROPERTIES: &[&str] = &[
    "Type",
    "Subtype",
    "Filter",
    "Length",
    "Length1",
    "Length2",
    "Length3",
    "declaredLength",
    "discoveredLength",
    "capped",
];
const CMAP_PROPERTIES: &[&str] = &[
    "hasCIDSystemInfo",
    "hasUseCMap",
    "embedded",
    "Type",
    "Subtype",
    "CMapName",
    "CIDSystemInfo",
    "WMode",
    "UseCMap",
];
const IMAGE_PROPERTIES: &[&str] = IMAGE_DIRECT_PROPERTIES;
const XOBJECT_PROPERTIES: &[&str] = XOBJECT_DIRECT_PROPERTIES;
const FORM_XOBJECT_PROPERTIES: &[&str] = XOBJECT_DIRECT_PROPERTIES;
const POSTSCRIPT_XOBJECT_PROPERTIES: &[&str] = XOBJECT_DIRECT_PROPERTIES;
const PATTERN_PROPERTIES: &[&str] = PATTERN_DIRECT_PROPERTIES;
const SHADING_PROPERTIES: &[&str] = SHADING_DIRECT_PROPERTIES;
const FUNCTION_PROPERTIES: &[&str] = FUNCTION_DIRECT_PROPERTIES;
const CONTENT_STREAM_PROPERTIES: &[&str] = &[
    "nrOperators",
    "hasText",
    "hasMarkedContent",
    "hasInlineImage",
    "hasUnknownOperators",
    "truncated",
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
const OPERATOR_PROPERTIES: &[&str] = &[
    "op",
    "family",
    "operandCount",
    "location",
    "isUnknown",
    "textBytes",
];
const MARKED_CONTENT_PROPERTIES: &[&str] = &["tag", "hasProperties", "nestingDepth", "location"];
const INLINE_IMAGE_PROPERTIES: &[&str] = &[
    "width",
    "height",
    "filters",
    "colorSpace",
    "bitsPerComponent",
    "location",
];
const RESOURCE_USE_PROPERTIES: &[&str] = &[
    "family",
    "name",
    "operator",
    "location",
    "status",
    "resolvedFamily",
    "resolvedObject",
];
const UNDEFINED_OPERATOR_PROPERTIES: &[&str] = &["name", "op", "operandCount", "location"];
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
    "FS",
];
const ACTION_PROPERTIES: &[&str] = ACTION_DIRECT_PROPERTIES;
const FORM_FIELD_PROPERTIES: &[&str] = FORM_FIELD_DIRECT_PROPERTIES;
const FILE_SPEC_PROPERTIES: &[&str] = FILE_SPEC_DIRECT_PROPERTIES;
const COLOR_SPACE_PROPERTIES: &[&str] = &[
    "family",
    "componentCount",
    "hasAlternate",
    "hasTintTransform",
    "hasICCProfile",
    "Type",
    "N",
    "Alternate",
    "Range",
    "Metadata",
    "Filter",
    "Family",
    "Base",
    "HiVal",
    "Lookup",
    "TintTransform",
    "Colorants",
    "Process",
    "Components",
];
const ICC_PROFILE_PROPERTIES: &[&str] = &[
    "profileSize",
    "version",
    "deviceClass",
    "colorSpace",
    "pcs",
    "renderingIntent",
    "tagCount",
    "capped",
    "N",
    "Alternate",
    "Range",
    "Metadata",
    "Filter",
];
const EXT_GSTATE_PROPERTIES: &[&str] = &[
    "hasSoftMask",
    "hasBlendMode",
    "alphaSource",
    "Type",
    "BM",
    "CA",
    "ca",
    "SMask",
    "AIS",
    "OP",
    "op",
    "OPM",
];
const ACCESSIBILITY_DOCUMENT_PROPERTIES: &[&str] = &[
    "isTagged",
    "language",
    "hasStructTreeRoot",
    "roleMapPresent",
    "classMapPresent",
    "idTreeEntries",
    "parentTreeEntries",
    "parentTreeNextKey",
    "structureElementCount",
    "markedContentAssociationCount",
    "artifactCount",
    "warningCount",
    "truncated",
];
const STRUCTURE_PROPERTIES: &[&str] = &[
    "Type",
    "K",
    "ParentTree",
    "ParentTreeNextKey",
    "RoleMap",
    "ClassMap",
    "IDTree",
    "isTagged",
    "roleMapPresent",
    "classMapPresent",
    "parentTreeEntries",
    "idTreeEntries",
    "truncated",
];
const STRUCTURE_ELEMENT_PROPERTIES: &[&str] = &[
    "role",
    "normalizedRole",
    "page",
    "pageIndex",
    "childCount",
    "contentItemCount",
    "associatedMarkedContentCount",
    "associatedAnnotationCount",
    "hasAltText",
    "altTextBytes",
    "hasActualText",
    "actualTextBytes",
    "hasLanguage",
    "hasAttributes",
    "hasClass",
    "hasId",
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
const TEXT_CHUNK_PROPERTIES: &[&str] = &[
    "tag",
    "mcid",
    "pageIndex",
    "structureRole",
    "normalizedRole",
    "textBytes",
    "rawText",
    "isArtifact",
];
const IMAGE_CHUNK_PROPERTIES: &[&str] = &[
    "tag",
    "mcid",
    "pageIndex",
    "structureRole",
    "normalizedRole",
    "hasAltText",
    "altTextBytes",
    "hasActualText",
    "actualTextBytes",
    "isArtifact",
];
const ACCESSIBILITY_ANNOTATION_PROPERTIES: &[&str] = &[
    "pageIndex",
    "structureRole",
    "normalizedRole",
    "isLink",
    "object",
];
const ARTIFACT_PROPERTIES: &[&str] = &["tag", "pageIndex", "structureRole"];
const TABLE_PROPERTIES: &[&str] = &[
    "role",
    "normalizedRole",
    "pageIndex",
    "childCount",
    "hasAttributes",
    "numberOfColumnWithWrongRowSpan",
    "numberOfRowWithWrongColumnSpan",
    "wrongColumnSpan",
];
const LIST_PROPERTIES: &[&str] = &[
    "role",
    "normalizedRole",
    "pageIndex",
    "childCount",
    "ListNumbering",
    "containsLabels",
];
const HEADING_PROPERTIES: &[&str] = &["role", "normalizedRole", "pageIndex", "level"];
const LINK_PROPERTIES: &[&str] = &[
    "role",
    "normalizedRole",
    "pageIndex",
    "associatedAnnotationCount",
    "differentTargetAnnotObjectKey",
];
const SIGNATURE_PROPERTIES: &[&str] = SIGNATURE_DIRECT_PROPERTIES;
const SECURITY_PROPERTIES: &[&str] = SECURITY_DIRECT_PROPERTIES;
const OUTPUT_INTENT_PROPERTIES: &[&str] = &[
    "hasDestOutputProfile",
    "iccProfileSize",
    "iccVersion",
    "iccDeviceClass",
    "iccColorSpace",
    "iccPcs",
    "iccRenderingIntent",
    "iccTagCount",
    "iccCapped",
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

const SAFE_FEATURE_STRING_PROPERTIES: &[&str] = &[
    "BaseFont",
    "CIDToGIDMap",
    "CMapName",
    "Encoding",
    "FT",
    "Filter",
    "family",
    "S",
    "Subtype",
    "Type",
    "colorSpace",
    "deviceClass",
    "iccColorSpace",
    "iccDeviceClass",
    "iccPcs",
    "iccVersion",
    "name",
    "normalizedRole",
    "operator",
    "pcs",
    "conformance",
    "conformancePrefix",
    "header",
    "partPrefix",
    "role",
    "structureRole",
    "resolvedFamily",
    "revPrefix",
    "status",
    "version",
];

const EMPTY_LINK_NAMES: &[(&str, &str)] = &[];
const DOCUMENT_LINKS: &[(&str, &str)] = &[
    ("catalog", "catalog"),
    ("streams", "stream"),
    ("accessibility", "accessibilityDocument"),
    ("security", "security"),
    ("signatures", "signature"),
];
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
    ("additionalActions", "action"),
];
const CONTENT_STREAM_LINKS: &[(&str, &str)] = &[
    ("operators", "operator"),
    ("markedContent", "markedContent"),
    ("inlineImages", "inlineImage"),
    ("usedResources", "resourceUse"),
];
const OPERATOR_LINKS: &[(&str, &str)] = &[
    ("resource", "resourceUse"),
    ("markedContentProperties", "object"),
];
const ANNOTATION_LINKS: &[(&str, &str)] = &[
    ("action", "action"),
    ("additionalActions", "action"),
    ("fileSpec", "fileSpec"),
    ("formField", "formField"),
];
const ACTION_LINKS: &[(&str, &str)] = &[
    ("next", "action"),
    ("fileSpec", "fileSpec"),
    ("destination", "destination"),
];
const FORM_FIELD_LINKS: &[(&str, &str)] = &[
    ("kids", "formField"),
    ("parent", "formField"),
    ("action", "action"),
    ("additionalActions", "action"),
    ("fileSpec", "fileSpec"),
];
const ACRO_FORM_LINKS: &[(&str, &str)] = &[("fields", "formField")];
const OUTLINE_LINKS: &[(&str, &str)] = &[
    ("first", "outline"),
    ("last", "outline"),
    ("next", "outline"),
    ("previous", "outline"),
    ("action", "action"),
    ("destination", "destination"),
];
const NAMES_LINKS: &[(&str, &str)] = &[("destinations", "destination"), ("files", "fileSpec")];
const DESTINATION_LINKS: &[(&str, &str)] = &[("action", "action")];
const XOBJECT_LINKS: &[(&str, &str)] = &[("contentStreams", "contentStream")];
const ACCESSIBILITY_DOCUMENT_LINKS: &[(&str, &str)] = &[
    ("structureElements", "structureElement"),
    ("textChunks", "textChunk"),
    ("imageChunks", "imageChunk"),
    ("annotations", "accessibilityAnnotation"),
    ("artifacts", "artifact"),
    ("tables", "table"),
    ("lists", "list"),
    ("headings", "heading"),
    ("links", "link"),
];
const STRUCTURE_ELEMENT_LINKS: &[(&str, &str)] = &[
    ("children", "structureElement"),
    ("textChunks", "textChunk"),
    ("imageChunks", "imageChunk"),
    ("annotations", "accessibilityAnnotation"),
];

/// Feature extraction selection.
#[derive(Clone, Debug, Default, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FeatureSelection {
    /// Do not extract features.
    #[default]
    None,
    /// Extract all built-in feature families.
    All,
    /// Extract selected feature families.
    Families {
        /// Selected validation-model family names.
        families: Vec<ObjectTypeName>,
    },
}

impl FeatureSelection {
    /// Returns true when feature extraction is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::None)
    }
}

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
        validate_feature_configuration(&validator.options)?;
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
        validate_feature_configuration(&validator.options)?;
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

    #[allow(
        clippy::too_many_lines,
        reason = "the facade keeps parse, validation, feature, and policy task ordering in one \
                  place so report construction remains auditable"
    )]
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

        let mut parsed = parsed;
        if parsed.is_encrypted() {
            let xmp = parse_document_xmp(&parsed, &self.options.resource_limits, false)?;
            parsed.parse_facts.extend(xmp.parse_facts);
            parsed.warnings.extend(xmp.warnings);
            return base_report(
                source_summary,
                ValidationStatus::Encrypted,
                Vec::new(),
                parsed,
                started.elapsed(),
            );
        }

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
                let xmp = parse_document_xmp(&parsed, &self.options.resource_limits, false)?;
                parsed.parse_facts.extend(xmp.parse_facts);
                parsed.warnings.extend(xmp.warnings);
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
        let mut status = if profile_reports
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

        let needs_features =
            self.options.feature_selection.is_enabled() || self.options.policy.is_some();
        let feature_started = Instant::now();
        let feature_report = if needs_features {
            Some(session.extract_features(&self.options.feature_selection)?)
        } else {
            None
        };
        let feature_duration = needs_features.then(|| {
            TaskDuration::from_duration(
                Identifier::unchecked("featureExtraction"),
                feature_started.elapsed(),
            )
        });
        let policy_started = Instant::now();
        let policy_report = match (&self.options.policy, feature_report.as_ref()) {
            (Some(policy), Some(features)) => {
                policy.validate()?;
                let report = evaluate_policy(policy, features)?;
                if !report.is_compliant && matches!(status, ValidationStatus::Valid) {
                    status = ValidationStatus::Invalid;
                }
                Some(report)
            }
            (Some(_), None) => {
                return Err(crate::PolicyError::Evaluation {
                    reason: BoundedText::unchecked("policy evaluation requires feature report"),
                }
                .into());
            }
            (None, _) => None,
        };
        let policy_duration = self.options.policy.is_some().then(|| {
            TaskDuration::from_duration(Identifier::unchecked("policy"), policy_started.elapsed())
        });
        let parse_facts = session.document.parse_facts.clone();
        let warnings = session.document.warnings.clone();
        let mut task_durations = vec![TaskDuration::from_duration(
            Identifier::new("validate")?,
            started.elapsed(),
        )];
        if let Some(duration) = feature_duration {
            task_durations.push(duration);
        }
        if let Some(duration) = policy_duration {
            task_durations.push(duration);
        }
        Ok(ValidationReport::builder()
            .engine_version(ENGINE_VERSION.to_owned())
            .source(source_summary)
            .status(status)
            .flavours(flavours)
            .profile_reports(profile_reports)
            .parse_facts(parse_facts)
            .warnings(warnings)
            .feature_report(feature_report)
            .policy_report(policy_report)
            .task_durations(task_durations)
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
        let graph = ModelGraph::for_rules(
            &self.document,
            &self.limits,
            &profile.rules,
            &profile.flavour,
        );
        let mut evaluator = DefaultRuleEvaluator::with_graph(self.limits.clone(), &graph);
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

    fn extract_features(&self, selection: &FeatureSelection) -> Result<FeatureReport> {
        let registry = ModelRegistry::default_registry();
        let selected = selected_feature_families(selection, &registry)?;
        let graph = ModelGraph::with_all_families(&self.document, &self.limits);
        let mut stack = Vec::from([ModelObjectRef::Document(DocumentModel::new(&self.document))]);
        let mut visited = HashSet::new();
        let mut objects = Vec::new();
        let mut truncated = false;

        while let Some(object) = stack.pop() {
            let visited_key = object.identity_key();
            if !visited.insert(visited_key) {
                continue;
            }
            let object_type = object.object_type();
            if selected.contains(&object_type)
                && let Some(feature) = feature_object(&registry, &object, &object_type)?
            {
                objects.push(feature);
            }
            if u64::try_from(visited.len()).map_err(|_| ValidationError::LimitExceeded {
                limit: "max_objects",
            })? > self.limits.max_objects
            {
                truncated = true;
                break;
            }
            let object_budget =
                match remaining_object_budget(&self.limits, visited.len(), stack.len()) {
                    Ok(budget) => budget,
                    Err(error) if is_object_limit_error(&error) => {
                        truncated = true;
                        break;
                    }
                    Err(error) => return Err(error),
                };
            let linked_objects = match object.linked_objects(&graph, object_budget) {
                Ok(objects) => objects,
                Err(error) if is_object_limit_error(&error) => {
                    truncated = true;
                    break;
                }
                Err(error) => return Err(error),
            };
            for linked in linked_objects {
                stack.push(linked);
            }
        }
        let visited_objects = u64::try_from(visited.len()).unwrap_or(u64::MAX);
        Ok(FeatureReport::builder()
            .objects(objects)
            .visited_objects(visited_objects)
            .selected_families(selected.into_iter().collect())
            .truncated(truncated)
            .build())
    }
}

fn selected_feature_families(
    selection: &FeatureSelection,
    registry: &ModelRegistry,
) -> Result<BTreeSet<ObjectTypeName>> {
    match selection {
        FeatureSelection::None | FeatureSelection::All => {
            Ok(registry.family_names().cloned().collect())
        }
        FeatureSelection::Families { families } => {
            let mut selected = BTreeSet::new();
            for family in families {
                if !registry.has_family(family) {
                    return Err(crate::ConfigError::InvalidValue {
                        field: "extract",
                        reason: BoundedText::unchecked("unknown feature family"),
                    }
                    .into());
                }
                selected.insert(family.clone());
            }
            Ok(selected)
        }
    }
}

fn is_object_limit_error(error: &PdfvError) -> bool {
    matches!(
        error,
        PdfvError::Validation(ValidationError::LimitExceeded {
            limit: "max_objects"
        })
    )
}

fn validate_feature_configuration(options: &ValidationOptions) -> Result<()> {
    let registry = ModelRegistry::default_registry();
    let _selected = selected_feature_families(&options.feature_selection, &registry)?;
    if let Some(policy) = &options.policy {
        policy.validate()?;
        validate_policy_schema(policy, &registry)?;
    }
    Ok(())
}

fn validate_policy_schema(policy: &PolicySet, registry: &ModelRegistry) -> Result<()> {
    for rule in &policy.rules {
        if !registry.has_family(&rule.family) {
            return Err(policy_invalid("family", "unknown policy feature family"));
        }
        if !registry.has_family_property(&rule.family, &rule.field) {
            return Err(policy_invalid(
                "field",
                "unknown policy feature field for family",
            ));
        }
        match rule.operator {
            PolicyOperator::Exists | PolicyOperator::Absent => {
                if rule.value.is_some() {
                    return Err(policy_invalid(
                        "value",
                        "exists and absent operators do not accept values",
                    ));
                }
            }
            PolicyOperator::Equals | PolicyOperator::NotEquals => {
                if rule.value.is_none() {
                    return Err(policy_invalid(
                        "value",
                        "comparison operator requires a value",
                    ));
                }
            }
            PolicyOperator::Min | PolicyOperator::Max => {
                if !matches!(rule.value, Some(PolicyValue::Number(_))) {
                    return Err(policy_invalid(
                        "value",
                        "numeric operator requires a number value",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn policy_invalid(field: &'static str, reason: &'static str) -> PdfvError {
    crate::PolicyError::InvalidField {
        field,
        reason: BoundedText::unchecked(reason),
    }
    .into()
}

fn feature_object(
    registry: &ModelRegistry,
    object: &ModelObjectRef<'_>,
    object_type: &ObjectTypeName,
) -> Result<Option<FeatureObject>> {
    let Some(properties) = registry.family_property_names(object_type) else {
        return Ok(None);
    };
    let mut values = BTreeMap::new();
    for property in properties {
        match object.property(&property) {
            Ok(value) => {
                values.insert(property.clone(), safe_feature_value(&property, value));
            }
            Err(PdfvError::Profile(crate::ProfileError::UnknownProperty { .. })) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(Some(
        FeatureObject::builder()
            .family(object_type.clone())
            .location(object.location())
            .context(object.context())
            .properties(values)
            .build(),
    ))
}

impl From<ModelValue> for FeatureValue {
    fn from(value: ModelValue) -> Self {
        match value {
            ModelValue::Null => Self::Null,
            ModelValue::Bool(value) => Self::Bool(value),
            ModelValue::Number(value) => Self::Number(value),
            ModelValue::String(value) => Self::String(value),
            ModelValue::ObjectKey(value) => Self::ObjectKey(value),
            ModelValue::List(values) => {
                Self::List(values.into_iter().map(FeatureValue::from).collect())
            }
        }
    }
}

fn safe_feature_value(property: &PropertyName, value: ModelValue) -> FeatureValue {
    match value {
        ModelValue::String(value)
            if !SAFE_FEATURE_STRING_PROPERTIES.contains(&property.as_str()) =>
        {
            FeatureValue::RedactedString {
                bytes: u64::try_from(value.as_str().len()).unwrap_or(u64::MAX),
            }
        }
        ModelValue::List(values) => FeatureValue::List(
            values
                .into_iter()
                .map(|value| safe_feature_value(property, value))
                .collect(),
        ),
        other => FeatureValue::from(other),
    }
}

fn evaluate_policy(policy: &PolicySet, features: &FeatureReport) -> Result<PolicyReport> {
    let results = policy
        .rules
        .iter()
        .map(|rule| evaluate_policy_rule(rule, features))
        .collect::<Result<Vec<_>>>()?;
    let is_compliant = results.iter().all(|result| result.passed);
    Ok(PolicyReport::builder()
        .name(policy.name.clone())
        .is_compliant(is_compliant)
        .results(results)
        .build())
}

fn evaluate_policy_rule(rule: &PolicyRule, features: &FeatureReport) -> Result<PolicyRuleResult> {
    let matches = features
        .objects
        .iter()
        .filter(|object| object.family == rule.family)
        .collect::<Vec<_>>();
    let values = matches
        .iter()
        .filter_map(|object| object.properties.get(&rule.field))
        .collect::<Vec<_>>();
    let passed = match rule.operator {
        PolicyOperator::Exists => !values.is_empty(),
        PolicyOperator::Absent => values.is_empty(),
        PolicyOperator::Equals => {
            let expected = required_policy_value(rule)?;
            values
                .iter()
                .any(|actual| policy_value_matches(actual, expected))
        }
        PolicyOperator::NotEquals => {
            let expected = required_policy_value(rule)?;
            values
                .iter()
                .all(|actual| !policy_value_matches(actual, expected))
        }
        PolicyOperator::Min => {
            let expected = required_policy_number(rule)?;
            values
                .iter()
                .filter_map(|value| feature_number(value))
                .any(|actual| actual >= expected)
        }
        PolicyOperator::Max => {
            let expected = required_policy_number(rule)?;
            values
                .iter()
                .filter_map(|value| feature_number(value))
                .any(|actual| actual <= expected)
        }
    };
    let matches = u64::try_from(matches.len()).unwrap_or(u64::MAX);
    Ok(PolicyRuleResult::builder()
        .id(rule.id.clone())
        .description(rule.description.clone())
        .passed(passed)
        .matches(matches)
        .message(policy_message(rule, passed, matches)?)
        .build())
}

fn required_policy_value(rule: &PolicyRule) -> Result<&PolicyValue> {
    rule.value.as_ref().ok_or_else(|| {
        crate::PolicyError::InvalidField {
            field: "value",
            reason: BoundedText::unchecked("operator requires a comparison value"),
        }
        .into()
    })
}

fn required_policy_number(rule: &PolicyRule) -> Result<f64> {
    match required_policy_value(rule)? {
        PolicyValue::Number(value) => Ok(f64::from(*value)),
        _ => Err(crate::PolicyError::InvalidField {
            field: "value",
            reason: BoundedText::unchecked("operator requires a numeric comparison value"),
        }
        .into()),
    }
}

fn policy_value_matches(actual: &FeatureValue, expected: &PolicyValue) -> bool {
    match (actual, expected) {
        (FeatureValue::Bool(actual), PolicyValue::Bool(expected)) => actual == expected,
        (FeatureValue::Number(actual), PolicyValue::Number(expected)) => {
            (*actual - f64::from(*expected)).abs() < f64::EPSILON
        }
        (FeatureValue::String(actual), PolicyValue::String(expected)) => actual == expected,
        _ => false,
    }
}

fn feature_number(value: &FeatureValue) -> Option<f64> {
    match value {
        FeatureValue::Number(value) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn policy_message(
    rule: &PolicyRule,
    passed: bool,
    matches: u64,
) -> std::result::Result<BoundedText, crate::ConfigError> {
    let status = if passed { "passed" } else { "failed" };
    BoundedText::new(
        format!(
            "policy rule {} {status} with {matches} matching feature objects",
            rule.id.as_str()
        ),
        256,
    )
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
    #[allow(
        clippy::too_many_lines,
        reason = "the registry is a declarative list of model families kept in one place for \
                  parity checks"
    )]
    pub(crate) fn default_registry() -> Self {
        let families = [
            family("document", DOCUMENT_PROPERTIES, DOCUMENT_LINKS),
            family("catalog", CATALOG_PROPERTIES, CATALOG_LINKS),
            family("metadata", METADATA_PROPERTIES, EMPTY_LINK_NAMES),
            family("page", PAGE_PROPERTIES, PAGE_LINKS),
            family("pageTree", PAGE_TREE_PROPERTIES, EMPTY_LINK_NAMES),
            family("resource", RESOURCE_PROPERTIES, EMPTY_LINK_NAMES),
            family("names", NAMES_PROPERTIES, NAMES_LINKS),
            family("outline", OUTLINE_PROPERTIES, OUTLINE_LINKS),
            family("destination", DESTINATION_PROPERTIES, DESTINATION_LINKS),
            family("acroForm", ACRO_FORM_PROPERTIES, ACRO_FORM_LINKS),
            family(
                "optionalContentProperties",
                OPTIONAL_CONTENT_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("permissions", PERMISSIONS_PROPERTIES, EMPTY_LINK_NAMES),
            family("font", FONT_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "fontDescriptor",
                FONT_DESCRIPTOR_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("fontProgram", FONT_PROGRAM_PROPERTIES, EMPTY_LINK_NAMES),
            family("cMap", CMAP_PROPERTIES, EMPTY_LINK_NAMES),
            family("embeddedFontFile", STREAM_PROPERTIES, EMPTY_LINK_NAMES),
            family("image", IMAGE_PROPERTIES, EMPTY_LINK_NAMES),
            family("xObject", XOBJECT_PROPERTIES, XOBJECT_LINKS),
            family("formXObject", FORM_XOBJECT_PROPERTIES, XOBJECT_LINKS),
            family(
                "postScriptXObject",
                POSTSCRIPT_XOBJECT_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family(
                "contentStream",
                CONTENT_STREAM_PROPERTIES,
                CONTENT_STREAM_LINKS,
            ),
            family("operator", OPERATOR_PROPERTIES, OPERATOR_LINKS),
            family("markedContent", MARKED_CONTENT_PROPERTIES, EMPTY_LINK_NAMES),
            family("inlineImage", INLINE_IMAGE_PROPERTIES, EMPTY_LINK_NAMES),
            family("resourceUse", RESOURCE_USE_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "undefinedOperator",
                UNDEFINED_OPERATOR_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("annotation", ANNOTATION_PROPERTIES, ANNOTATION_LINKS),
            family("action", ACTION_PROPERTIES, ACTION_LINKS),
            family("formField", FORM_FIELD_PROPERTIES, FORM_FIELD_LINKS),
            family("fileSpec", FILE_SPEC_PROPERTIES, EMPTY_LINK_NAMES),
            family("colorSpace", COLOR_SPACE_PROPERTIES, EMPTY_LINK_NAMES),
            family("iccProfile", ICC_PROFILE_PROPERTIES, EMPTY_LINK_NAMES),
            family("extGState", EXT_GSTATE_PROPERTIES, EMPTY_LINK_NAMES),
            family("pattern", PATTERN_PROPERTIES, EMPTY_LINK_NAMES),
            family("shading", SHADING_PROPERTIES, EMPTY_LINK_NAMES),
            family("function", FUNCTION_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "accessibilityDocument",
                ACCESSIBILITY_DOCUMENT_PROPERTIES,
                ACCESSIBILITY_DOCUMENT_LINKS,
            ),
            family(
                "structureTreeRoot",
                STRUCTURE_PROPERTIES,
                ACCESSIBILITY_DOCUMENT_LINKS,
            ),
            family(
                "structureElement",
                STRUCTURE_ELEMENT_PROPERTIES,
                STRUCTURE_ELEMENT_LINKS,
            ),
            family("textChunk", TEXT_CHUNK_PROPERTIES, EMPTY_LINK_NAMES),
            family("imageChunk", IMAGE_CHUNK_PROPERTIES, EMPTY_LINK_NAMES),
            family(
                "accessibilityAnnotation",
                ACCESSIBILITY_ANNOTATION_PROPERTIES,
                EMPTY_LINK_NAMES,
            ),
            family("artifact", ARTIFACT_PROPERTIES, EMPTY_LINK_NAMES),
            family("table", TABLE_PROPERTIES, EMPTY_LINK_NAMES),
            family("list", LIST_PROPERTIES, EMPTY_LINK_NAMES),
            family("heading", HEADING_PROPERTIES, EMPTY_LINK_NAMES),
            family("link", LINK_PROPERTIES, EMPTY_LINK_NAMES),
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

    /// Returns the target family for a link on a registered family schema.
    #[must_use]
    pub(crate) fn link_target(
        &self,
        family: &ObjectTypeName,
        link: &PropertyName,
    ) -> Option<ObjectTypeName> {
        self.families.get(family).and_then(|family| {
            family
                .link_schema()
                .iter()
                .find(|spec| spec.name.as_str() == link.as_str())
                .map(|spec| spec.target.clone())
        })
    }

    /// Returns an unsupported reason when a property path does not bind to the schema.
    pub(crate) fn unsupported_property_path_reason(
        &self,
        object_type: &ObjectTypeName,
        path: &crate::PropertyPath,
    ) -> Result<Option<BoundedText>> {
        let Some((terminal, links)) = path.parts().split_last() else {
            return Ok(Some(BoundedText::unchecked("empty model property path")));
        };
        let mut family = object_type.clone();
        for link in links {
            let Some(target) = self.link_target(&family, link) else {
                return Ok(Some(BoundedText::new(
                    format!(
                        "unknown validation model link {} on {}",
                        link.as_str(),
                        family.as_str()
                    ),
                    512,
                )?));
            };
            family = target;
        }
        if !self.has_family_property(&family, terminal) {
            return Ok(Some(BoundedText::new(
                format!(
                    "unknown validation model property {} on {}",
                    terminal.as_str(),
                    family.as_str()
                ),
                512,
            )?));
        }
        Ok(None)
    }

    fn family_property_names(&self, family: &ObjectTypeName) -> Option<Vec<PropertyName>> {
        self.families.get(family).map(|family| {
            family
                .property_schema()
                .iter()
                .map(|property| property.name.clone())
                .collect()
        })
    }

    /// Iterates registered family names.
    pub(crate) fn family_names(&self) -> impl Iterator<Item = &ObjectTypeName> {
        self.families.keys()
    }

    pub(crate) fn registered_family_count(&self) -> u64 {
        u64::try_from(self.families.len()).unwrap_or(u64::MAX)
    }

    pub(crate) fn registered_property_count(&self) -> u64 {
        self.families
            .values()
            .map(|family| u64::try_from(family.property_schema().len()).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add)
    }

    pub(crate) fn registered_link_count(&self) -> u64 {
        self.families
            .values()
            .map(|family| u64::try_from(family.link_schema().len()).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add)
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

    /// Returns the link name text.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
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
    /// Content-stream operator object.
    Operator(OperatorModel<'a>),
    /// Marked-content object.
    MarkedContent(MarkedContentModel<'a>),
    /// Inline image object.
    InlineImage(InlineImageModel<'a>),
    /// Resource-use object.
    ResourceUse(ResourceUseModel<'a>),
    /// Accessibility semantic object.
    Accessibility(AccessibilityModel<'a>),
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
            Self::Operator(model) => model.document,
            Self::MarkedContent(model) => model.document,
            Self::InlineImage(model) => model.document,
            Self::ResourceUse(model) => model.document,
            Self::Accessibility(model) => model.document,
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
            Self::Operator(model) => model.object_type(),
            Self::MarkedContent(model) => model.object_type(),
            Self::InlineImage(model) => model.object_type(),
            Self::ResourceUse(model) => model.object_type(),
            Self::Accessibility(model) => model.object_type(),
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
            Self::Operator(model) => model.property(name),
            Self::MarkedContent(model) => model.property(name),
            Self::InlineImage(model) => model.property(name),
            Self::ResourceUse(model) => model.property(name),
            Self::Accessibility(model) => model.property(name),
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
                    "{}/contentStream[{}]",
                    model.context_prefix, model.ordinal
                ))),
            },
            Self::Operator(model) => model.fact.location().clone(),
            Self::MarkedContent(model) => model.span.location.clone(),
            Self::InlineImage(model) => model.fact.location().clone(),
            Self::ResourceUse(model) => model.use_fact.location.clone(),
            Self::Accessibility(model) => model.location(),
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
                "{}/contentStream[{}]",
                model.context_prefix, model.ordinal
            )),
            Self::Operator(model) => BoundedText::unchecked(format!(
                "root/page[{}]/contentStream[{}]/operator[{}]",
                model.page_ordinal, model.stream_ordinal, model.ordinal
            )),
            Self::MarkedContent(model) => BoundedText::unchecked(format!(
                "root/page[{}]/contentStream[{}]/markedContent[{}]",
                model.page_ordinal, model.stream_ordinal, model.ordinal
            )),
            Self::InlineImage(model) => BoundedText::unchecked(format!(
                "root/page[{}]/contentStream[{}]/inlineImage[{}]",
                model.page_ordinal, model.stream_ordinal, model.ordinal
            )),
            Self::ResourceUse(model) => BoundedText::unchecked(format!(
                "root/page[{}]/contentStream[{}]/resourceUse[{}]",
                model.page_ordinal, model.stream_ordinal, model.ordinal
            )),
            Self::Accessibility(model) => model.context(),
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
            Self::Operator(model) => format!(
                "operator:{}:{}:{}:{}",
                model.page_ordinal, model.stream_ordinal, model.source.number, model.ordinal
            ),
            Self::MarkedContent(model) => format!(
                "markedContent:{}:{}:{}:{}",
                model.page_ordinal, model.stream_ordinal, model.source.number, model.ordinal
            ),
            Self::InlineImage(model) => format!(
                "inlineImage:{}:{}:{}:{}",
                model.page_ordinal, model.stream_ordinal, model.source.number, model.ordinal
            ),
            Self::ResourceUse(model) => format!(
                "resourceUse:{}:{}:{}:{}",
                model.page_ordinal, model.stream_ordinal, model.source.number, model.ordinal
            ),
            Self::Accessibility(model) => model.identity_key(),
            Self::Stream(model) => format!("stream:{}:{}", model.key.number, model.key.generation),
            Self::Generic(model) => generic_identity_key(
                model.object_type.as_str(),
                model.key,
                model.ordinal,
                model.context.as_str(),
            ),
        }
    }

    pub(crate) fn links(&self) -> &[LinkName] {
        match self {
            Self::Document(model) => model.links(),
            Self::Catalog(model) => model.links(),
            Self::Metadata(model) => model.links(),
            Self::Page(model) => model.links(),
            Self::Font(model) => model.links(),
            Self::Annotation(model) => model.links(),
            Self::OutputIntent(model) => model.links(),
            Self::ContentStream(model) => model.links(),
            Self::Operator(model) => model.links(),
            Self::MarkedContent(model) => model.links(),
            Self::InlineImage(model) => model.links(),
            Self::ResourceUse(model) => model.links(),
            Self::Accessibility(model) => model.links(),
            Self::Stream(model) => model.links(),
            Self::Generic(model) => model.links(),
        }
    }

    pub(crate) fn linked_objects(
        &self,
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        match self {
            Self::Document(model) => model.linked_objects(graph, max_objects),
            Self::Catalog(model) => model.linked_objects(graph, max_objects),
            Self::Metadata(model) => model.linked_objects(graph, max_objects),
            Self::Page(model) => page_linked_objects(model, graph, max_objects),
            Self::Font(model) => model.linked_objects(graph, max_objects),
            Self::Annotation(model) => model.linked_objects(graph, max_objects),
            Self::OutputIntent(model) => model.linked_objects(graph, max_objects),
            Self::ContentStream(model) => model.linked_objects(graph, max_objects),
            Self::Operator(model) => model.linked_objects(graph, max_objects),
            Self::MarkedContent(model) => model.linked_objects(graph, max_objects),
            Self::InlineImage(model) => model.linked_objects(graph, max_objects),
            Self::ResourceUse(model) => model.linked_objects(graph, max_objects),
            Self::Accessibility(model) => model.linked_objects(graph, max_objects),
            Self::Stream(model) => model.linked_objects(graph, max_objects),
            Self::Generic(model) => generic_linked_objects(model, graph, max_objects),
        }
    }
}

/// Document model wrapper.
#[derive(Debug)]
pub struct ModelGraph<'a> {
    document: &'a ParsedDocument,
    limits: &'a ResourceLimits,
    materialized_families: BTreeSet<ObjectTypeName>,
    active_flavour: Option<ValidationFlavour>,
    accessibility_cache: OnceLock<Arc<AccessibilityGraph>>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceResolutionStatus {
    Resolved,
    Missing,
    WrongType,
    Cyclic,
}

impl ResourceResolutionStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Missing => "missing",
            Self::WrongType => "wrongType",
            Self::Cyclic => "cyclic",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedResourceUse {
    status: ResourceResolutionStatus,
    family: ResourceFamily,
    object: Option<ObjectKey>,
    object_family: Option<&'static str>,
}

#[derive(Clone, Debug)]
struct EffectiveResources<'a> {
    dictionaries: Vec<&'a crate::Dictionary>,
    cyclic: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IccHeader {
    profile_size: u64,
    version: String,
    device_class: String,
    color_space: String,
    pcs: String,
    rendering_intent: u64,
    tag_count: u64,
}

impl<'a> ModelGraph<'a> {
    fn for_rules(
        document: &'a ParsedDocument,
        limits: &'a ResourceLimits,
        rules: &[Rule],
        active_flavour: &ValidationFlavour,
    ) -> Self {
        let registry = ModelRegistry::default_registry();
        let mut materialized_families = BTreeSet::new();
        for rule in rules
            .iter()
            .filter(|rule| !matches!(rule.test, crate::RuleExpr::Unsupported { .. }))
        {
            materialized_families.insert(rule.object_type.clone());
            collect_rule_link_targets(
                &registry,
                &rule.object_type,
                &rule.test,
                &mut materialized_families,
            );
        }
        Self {
            document,
            limits,
            materialized_families,
            active_flavour: Some(active_flavour.clone()),
            accessibility_cache: OnceLock::new(),
        }
    }

    fn with_all_families(document: &'a ParsedDocument, limits: &'a ResourceLimits) -> Self {
        Self {
            document,
            limits,
            materialized_families: ModelRegistry::default_registry()
                .family_names()
                .cloned()
                .collect(),
            active_flavour: None,
            accessibility_cache: OnceLock::new(),
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
                    | "operator"
                    | "markedContent"
                    | "inlineImage"
                    | "resourceUse"
                    | "accessibilityDocument"
                    | "structureElement"
                    | "textChunk"
                    | "imageChunk"
                    | "accessibilityAnnotation"
                    | "artifact"
                    | "table"
                    | "list"
                    | "heading"
                    | "link"
                    | "stream"
                    | "object"
            )
        })
    }

    fn materializes_accessibility(&self) -> bool {
        self.materialized_families.iter().any(|family| {
            matches!(
                family.as_str(),
                "accessibilityDocument"
                    | "structureTreeRoot"
                    | "structureElement"
                    | "textChunk"
                    | "imageChunk"
                    | "accessibilityAnnotation"
                    | "artifact"
                    | "table"
                    | "list"
                    | "heading"
                    | "link"
            )
        })
    }

    fn accessibility_graph(&self) -> Result<Arc<AccessibilityGraph>> {
        if let Some(graph) = self.accessibility_cache.get() {
            return Ok(Arc::clone(graph));
        }
        let graph = Arc::new(crate::accessibility::build_accessibility_graph(
            self.document,
            self.limits,
        )?);
        let _ = self.accessibility_cache.set(Arc::clone(&graph));
        Ok(graph)
    }

    fn catalog(&self) -> Option<CatalogModel<'a>> {
        self.document
            .catalog
            .and_then(|key| CatalogModel::new(self.document, key))
    }

    fn metadata(&self, catalog: &CatalogModel<'_>) -> Option<MetadataModel<'a>> {
        MetadataModel::new(
            self.document,
            catalog.metadata,
            self.active_flavour.as_ref(),
        )
    }

    fn pages(&self, catalog: &CatalogModel<'_>, max_objects: usize) -> Result<Vec<PageModel<'a>>> {
        PageModel::from_catalog(self.document, catalog, self.limits, max_objects)
    }

    fn fonts(&self, page: &PageModel<'a>, max_objects: usize) -> Result<Vec<FontModel<'a>>> {
        FontModel::from_page(self.document, page, max_objects)
    }

    fn annotations(
        &self,
        page: &PageModel<'a>,
        max_objects: usize,
    ) -> Result<Vec<AnnotationModel<'a>>> {
        AnnotationModel::from_page(self.document, page, max_objects)
    }

    fn output_intents(
        &self,
        catalog: &CatalogModel<'_>,
        max_objects: usize,
    ) -> Result<Vec<OutputIntentModel<'a>>> {
        OutputIntentModel::from_catalog(self.document, catalog, self.limits, max_objects)
    }

    fn content_streams(
        &self,
        page: &PageModel<'a>,
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
        let effective = effective_page_resources(self.document, page)?;
        for (resource_ordinal, resources) in effective.dictionaries.iter().enumerate() {
            push_generic_model(
                models,
                GenericModel::new(
                    self.document,
                    "resource",
                    None,
                    None,
                    resources,
                    resource_ordinal,
                    format!("root/page[{}]/resources[{resource_ordinal}]", page.ordinal),
                ),
                max_objects,
            )?;
            for (resource_name, family) in [
                ("XObject", "xObject"),
                ("ColorSpace", "colorSpace"),
                ("ExtGState", "extGState"),
                ("Pattern", "pattern"),
                ("Shading", "shading"),
            ] {
                self.push_resource_collection(
                    ResourceCollection {
                        resources,
                        resource_name,
                        family,
                        context_prefix: "root/page",
                        page_ordinal: page.ordinal,
                        max_objects,
                    },
                    models,
                )?;
            }
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

fn collect_rule_link_targets(
    registry: &ModelRegistry,
    object_type: &ObjectTypeName,
    expr: &crate::RuleExpr,
    families: &mut BTreeSet<ObjectTypeName>,
) {
    match expr {
        crate::RuleExpr::Property { path } => {
            let mut family = object_type.clone();
            let Some((_terminal, links)) = path.parts().split_last() else {
                return;
            };
            for link in links {
                let Some(target) = registry.link_target(&family, link) else {
                    return;
                };
                families.insert(target.clone());
                family = target;
            }
        }
        crate::RuleExpr::Unary { expr, .. } => {
            collect_rule_link_targets(registry, object_type, expr, families);
        }
        crate::RuleExpr::Binary { left, right, .. } => {
            collect_rule_link_targets(registry, object_type, left, families);
            collect_rule_link_targets(registry, object_type, right, families);
        }
        crate::RuleExpr::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            collect_rule_link_targets(registry, object_type, condition, families);
            collect_rule_link_targets(registry, object_type, when_true, families);
            collect_rule_link_targets(registry, object_type, when_false, families);
        }
        crate::RuleExpr::Call { args, .. } => {
            for arg in args {
                collect_rule_link_targets(registry, object_type, arg, families);
            }
        }
        crate::RuleExpr::Bool { .. }
        | crate::RuleExpr::Number { .. }
        | crate::RuleExpr::String { .. }
        | crate::RuleExpr::Null
        | crate::RuleExpr::Unsupported { .. } => {}
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
            links: DOCUMENT_LINKS
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
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
        if graph.materializes_accessibility() {
            push_linked(
                &mut objects,
                ModelObjectRef::Accessibility(AccessibilityModel::document(
                    graph.document,
                    graph.accessibility_graph()?,
                )),
                max_objects,
            )?;
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
            links: CATALOG_LINKS
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
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
            "language" => self
                .document
                .objects
                .get(&self.key)
                .and_then(|object| object.object.as_dictionary())
                .and_then(|dictionary| dictionary.get("Lang"))
                .cloned()
                .map_or(Ok(ModelValue::Null), |value| Ok(ModelValue::from(value))),
            "permissions" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| dictionary.get("Perms"))
                    .is_some(),
            )),
            "Marked" => Ok(ModelValue::Bool(
                self.document
                    .objects
                    .get(&self.key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(|dictionary| {
                        resolve_dictionary_value(self.document, dictionary.get("MarkInfo"))
                    })
                    .and_then(|mark_info| mark_info.get("Marked"))
                    .is_some_and(|value| matches!(value, crate::CosObject::Boolean(true))),
            )),
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
        let mut generic_models = Vec::new();
        graph.push_catalog_generic_models(
            self,
            max_objects.saturating_sub(objects.len()),
            &mut generic_models,
        )?;
        generic_models.reverse();
        for model in generic_models {
            if graph.materializes(model.object_type.as_str()) {
                push_linked(&mut objects, ModelObjectRef::Generic(model), max_objects)?;
            }
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
    active_family: Option<Identifier>,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> MetadataModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        key: Option<ObjectKey>,
        active_flavour: Option<&ValidationFlavour>,
    ) -> Option<Self> {
        let key = key?;
        let object = document.objects.get(&key)?;
        if !matches!(object.object, crate::CosObject::Stream(_)) {
            return None;
        }
        Some(Self {
            document,
            key,
            offset: object.offset,
            active_family: active_flavour.map(|flavour| flavour.family.clone()),
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
        let family = self.active_family.as_ref().map(Identifier::as_str);
        match name.as_str() {
            "present" | "catalogMetadata" => Ok(ModelValue::Bool(true)),
            "containsPDFAIdentification" => {
                Ok(ModelValue::Bool(contains_xmp_family(self.document, "pdfa")))
            }
            "containsPDFUAIdentification" => Ok(ModelValue::Bool(contains_xmp_family(
                self.document,
                "pdfua",
            ))),
            "part" => Ok(ModelValue::Number(
                xmp_part(self.document, family).unwrap_or(0.0),
            )),
            "partPrefix" => Ok(xmp_prefix(self.document, family, XmpPrefixProperty::Part)
                .map_or(ModelValue::Null, |prefix| {
                    ModelValue::String(BoundedText::unchecked(prefix))
                })),
            "conformance" => Ok(xmp_conformance(self.document, family)
                .map_or(ModelValue::Null, |value| {
                    ModelValue::String(BoundedText::unchecked(value))
                })),
            "conformancePrefix" => {
                Ok(
                    xmp_prefix(self.document, family, XmpPrefixProperty::Conformance)
                        .map_or(ModelValue::Null, |prefix| {
                            ModelValue::String(BoundedText::unchecked(prefix))
                        }),
                )
            }
            "revPrefix" => Ok(xmp_prefix(self.document, family, XmpPrefixProperty::Rev)
                .map_or(ModelValue::Null, |prefix| {
                    ModelValue::String(BoundedText::unchecked(prefix))
                })),
            "amdPrefix" | "corrPrefix" => Ok(xmp_identification_claim(self.document, family)
                .map_or(ModelValue::Null, |claim| {
                    let prefix = match claim.family.as_str() {
                        "pdfua" => "pdfuaid",
                        _ => "pdfaid",
                    };
                    ModelValue::String(BoundedText::unchecked(prefix))
                })),
            "rev" => Ok(
                xmp_rev(self.document, family).map_or(ModelValue::Null, |rev| {
                    ModelValue::String(BoundedText::unchecked(rev))
                }),
            ),
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
    limits: &'a ResourceLimits,
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
        limits: &'a ResourceLimits,
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
                        limits,
                        key,
                        offset: object.offset,
                        ordinal: pages.len(),
                        dictionary,
                        object_type: ObjectTypeName::unchecked("page"),
                        supertypes: vec![ObjectTypeName::unchecked("object")],
                        links: PAGE_LINKS
                            .iter()
                            .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                            .collect(),
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
            "hasResources" => Ok(ModelValue::Bool(
                inherited_page_value(self.document, self.key, "Resources", self.limits)?.is_some(),
            )),
            "annotationCount" => Ok(ModelValue::Number(usize_to_f64(
                object_refs_or_direct_count(self.dictionary.get("Annots")),
            )?)),
            _ if PAGE_INHERITED_PROPERTIES.contains(&name.as_str()) => {
                Ok(
                    inherited_page_value(self.document, self.key, name.as_str(), self.limits)?
                        .cloned()
                        .map_or(ModelValue::Null, ModelValue::from),
                )
            }
            _ => dictionary_property(self.dictionary, name, PAGE_DIRECT_PROPERTIES),
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

fn page_linked_objects<'a>(
    page: &PageModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = Vec::new();
    let mut content_streams = graph.content_streams(page, max_objects)?;
    content_streams.reverse();
    for content_stream in content_streams {
        push_linked(
            &mut objects,
            ModelObjectRef::ContentStream(content_stream),
            max_objects,
        )?;
    }
    let mut annotations = graph.annotations(page, max_objects.saturating_sub(objects.len()))?;
    annotations.reverse();
    for annotation in annotations {
        push_linked(
            &mut objects,
            ModelObjectRef::Annotation(annotation),
            max_objects,
        )?;
    }
    let mut fonts = graph.fonts(page, max_objects.saturating_sub(objects.len()))?;
    fonts.reverse();
    for font in fonts {
        push_linked(&mut objects, ModelObjectRef::Font(font), max_objects)?;
    }
    let mut actions = action_models_from_dictionary_entries(
        graph.document,
        page.dictionary,
        &["AA"],
        page.ordinal,
        "root/page",
        max_objects.saturating_sub(objects.len()),
    )?;
    actions.reverse();
    for action in actions {
        push_linked(&mut objects, ModelObjectRef::Generic(action), max_objects)?;
    }
    Ok(objects)
}

/// Font dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct FontModel<'a> {
    document: &'a ParsedDocument,
    limits: &'a ResourceLimits,
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
        page: &PageModel<'a>,
        max_objects: usize,
    ) -> Result<Vec<Self>> {
        let mut fonts = Vec::new();
        let effective = effective_page_resources(document, page)?;
        let mut seen_names = BTreeSet::new();
        for resources in effective.dictionaries.iter().rev() {
            let Some(crate::CosObject::Dictionary(fonts_dictionary)) = resources.get("Font") else {
                continue;
            };
            for (name, value) in fonts_dictionary.iter() {
                if !seen_names.insert(name.clone()) {
                    continue;
                }
                if let Some((key, offset, dictionary)) = resolve_named_dictionary(document, value) {
                    if fonts.len() >= max_objects {
                        return Err(ValidationError::LimitExceeded {
                            limit: "max_objects",
                        }
                        .into());
                    }
                    fonts.push(Self {
                        document,
                        limits: page.limits,
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
            "embedded" => Ok(ModelValue::Bool(font_embedded(
                self.document,
                self.dictionary,
            ))),
            "embeddedProgramBytes" => Ok(ModelValue::Number(u64_to_f64(font_program_bytes(
                self.document,
                self.dictionary,
            )?)?)),
            "embeddedProgramCapped" => Ok(ModelValue::Bool(
                font_program_bytes(self.document, self.dictionary)?
                    > self.limits.max_embedded_font_bytes,
            )),
            "hasSubtype" => Ok(ModelValue::Bool(self.dictionary.get("Subtype").is_some())),
            "fontFamily" => Ok(ModelValue::String(BoundedText::unchecked(
                classify_font_dictionary(self.dictionary).unwrap_or("font"),
            ))),
            "hasFontDescriptor" => Ok(ModelValue::Bool(
                self.dictionary.get("FontDescriptor").is_some(),
            )),
            "hasToUnicode" => Ok(ModelValue::Bool(self.dictionary.get("ToUnicode").is_some())),
            "hasEncoding" => Ok(ModelValue::Bool(self.dictionary.get("Encoding").is_some())),
            "hasWidths" => Ok(ModelValue::Bool(
                self.dictionary.get("Widths").is_some()
                    || self.dictionary.get("W").is_some()
                    || self.dictionary.get("DW").is_some(),
            )),
            "hasCIDSystemInfo" => Ok(ModelValue::Bool(
                self.dictionary.get("CIDSystemInfo").is_some()
                    || descendant_font_dictionary(self.document, self.dictionary)
                        .is_some_and(|dictionary| dictionary.get("CIDSystemInfo").is_some()),
            )),
            "hasCharProcs" => Ok(ModelValue::Bool(self.dictionary.get("CharProcs").is_some())),
            "hasResources" => Ok(ModelValue::Bool(self.dictionary.get("Resources").is_some())),
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
        page: &PageModel<'a>,
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
                    links: ANNOTATION_LINKS
                        .iter()
                        .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                        .collect(),
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
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut objects = Vec::new();
        if self.dictionary.get("Subtype").is_some_and(
            |value| matches!(value, crate::CosObject::Name(name) if name.matches("Widget")),
        ) {
            push_linked(
                &mut objects,
                ModelObjectRef::Generic(GenericModel::new(
                    graph.document,
                    "formField",
                    self.key,
                    self.offset,
                    self.dictionary,
                    self.ordinal,
                    format!(
                        "root/page[{}]/annotation[{}]/formField[0]",
                        self.page_ordinal, self.ordinal
                    ),
                )),
                max_objects,
            )?;
        }
        let mut actions = action_models_from_dictionary_entries(
            graph.document,
            self.dictionary,
            &["A", "AA"],
            self.ordinal,
            "root/annotation",
            max_objects.saturating_sub(objects.len()),
        )?;
        actions.reverse();
        for action in actions {
            push_linked(&mut objects, ModelObjectRef::Generic(action), max_objects)?;
        }
        if let Some(file_spec) = file_spec_model_from_value(
            graph.document,
            self.dictionary.get("FS"),
            self.ordinal,
            format!(
                "root/page[{}]/annotation[{}]/fileSpec[0]",
                self.page_ordinal, self.ordinal
            ),
        ) {
            push_linked(
                &mut objects,
                ModelObjectRef::Generic(file_spec),
                max_objects,
            )?;
        }
        Ok(objects)
    }
}

/// Output intent dictionary model wrapper.
#[derive(Clone, Debug)]
pub struct OutputIntentModel<'a> {
    document: &'a ParsedDocument,
    limits: &'a ResourceLimits,
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
        limits: &'a ResourceLimits,
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
                    limits,
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
            "iccProfileSize" => optional_u64_model_value(
                output_intent_icc_header(self.document, self.dictionary, self.limits)?
                    .map(|header| header.profile_size),
            ),
            "iccVersion" => {
                Ok(
                    output_intent_icc_header(self.document, self.dictionary, self.limits)?
                        .map_or(ModelValue::Null, |header| {
                            ModelValue::String(BoundedText::unchecked(header.version))
                        }),
                )
            }
            "iccDeviceClass" => {
                Ok(
                    output_intent_icc_header(self.document, self.dictionary, self.limits)?
                        .map_or(ModelValue::Null, |header| {
                            ModelValue::String(BoundedText::unchecked(header.device_class))
                        }),
                )
            }
            "iccColorSpace" => {
                Ok(
                    output_intent_icc_header(self.document, self.dictionary, self.limits)?
                        .map_or(ModelValue::Null, |header| {
                            ModelValue::String(BoundedText::unchecked(header.color_space))
                        }),
                )
            }
            "iccPcs" => Ok(
                output_intent_icc_header(self.document, self.dictionary, self.limits)?
                    .map_or(ModelValue::Null, |header| {
                        ModelValue::String(BoundedText::unchecked(header.pcs))
                    }),
            ),
            "iccRenderingIntent" => optional_u64_model_value(
                output_intent_icc_header(self.document, self.dictionary, self.limits)?
                    .map(|header| header.rendering_intent),
            ),
            "iccTagCount" => optional_u64_model_value(
                output_intent_icc_header(self.document, self.dictionary, self.limits)?
                    .map(|header| header.tag_count),
            ),
            "iccCapped" => Ok(ModelValue::Bool(
                icc_stream_from_dictionary(self.document, self.dictionary).is_some_and(|stream| {
                    stream.discovered_length > self.limits.max_icc_profile_bytes
                }),
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
    limits: &'a ResourceLimits,
    page_key: Option<ObjectKey>,
    resource_context: Option<crate::Dictionary>,
    context_prefix: String,
    page_ordinal: usize,
    ordinal: usize,
    key: ObjectKey,
    offset: u64,
    stream: &'a crate::StreamObject,
    summary_cache: Arc<OnceLock<std::result::Result<ContentStreamSummary, ParseError>>>,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> ContentStreamModel<'a> {
    fn from_page(
        document: &'a ParsedDocument,
        page: &PageModel<'a>,
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

    fn summary(&self) -> Result<ContentStreamSummary> {
        let location_path = format!("{}/contentStream[{}]", self.context_prefix, self.ordinal);
        self.summary_cache
            .get_or_init(|| {
                let decoded = self.stream.decoded_bytes(self.limits)?;
                crate::content::summarize_content_stream(
                    self.key,
                    &location_path,
                    &decoded,
                    self.limits,
                )
            })
            .clone()
            .map_err(PdfvError::Parse)
    }

    fn effective_resources(&self, document: &'a ParsedDocument) -> Result<EffectiveResources<'_>> {
        if let Some(resources) = &self.resource_context {
            return Ok(EffectiveResources {
                dictionaries: vec![resources],
                cyclic: false,
            });
        }
        let Some(page_key) = self.page_key else {
            return Ok(EffectiveResources {
                dictionaries: Vec::new(),
                cyclic: false,
            });
        };
        effective_resources_for_page_key(document, page_key, self.limits)
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
            "nrOperators" | "operatorCount" => Ok(ModelValue::Number(u64_to_f64(
                self.summary()?.operators_seen,
            )?)),
            "markedContentCount" => Ok(ModelValue::Number(usize_to_f64(
                self.summary()?.marked_content.len(),
            )?)),
            "hasText" => Ok(ModelValue::Bool(self.summary()?.has_text())),
            "hasMarkedContent" => Ok(ModelValue::Bool(self.summary()?.has_marked_content())),
            "hasInlineImage" => Ok(ModelValue::Bool(self.summary()?.has_inline_image())),
            "hasUnknownOperators" => Ok(ModelValue::Bool(self.summary()?.has_unknown_operators())),
            "truncated" => Ok(ModelValue::Bool(self.summary()?.truncated)),
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
        graph: &ModelGraph<'a>,
        max_objects: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let summary = self.summary()?;
        let mut objects = Vec::new();
        if graph.materializes("operator") {
            for (ordinal, fact) in summary.facts.iter().cloned().enumerate() {
                push_linked(
                    &mut objects,
                    ModelObjectRef::Operator(OperatorModel::new(
                        graph.document,
                        self.key,
                        self.page_ordinal,
                        self.ordinal,
                        ordinal,
                        fact,
                    )),
                    max_objects,
                )?;
            }
        }
        if graph.materializes("markedContent") {
            for (ordinal, span) in summary.marked_content.iter().cloned().enumerate() {
                push_linked(
                    &mut objects,
                    ModelObjectRef::MarkedContent(MarkedContentModel::new(
                        graph.document,
                        self.key,
                        self.page_ordinal,
                        self.ordinal,
                        ordinal,
                        span,
                    )),
                    max_objects,
                )?;
            }
        }
        if graph.materializes("inlineImage") {
            let mut ordinal = 0_usize;
            for fact in summary
                .facts
                .iter()
                .filter(|fact| matches!(fact, OperatorFact::InlineImage { .. }))
                .cloned()
            {
                push_linked(
                    &mut objects,
                    ModelObjectRef::InlineImage(InlineImageModel::new(
                        graph.document,
                        self.key,
                        self.page_ordinal,
                        self.ordinal,
                        ordinal,
                        fact,
                    )),
                    max_objects,
                )?;
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or(ValidationError::LimitExceeded {
                        limit: "max_objects",
                    })?;
            }
        }
        if graph.materializes("resourceUse") {
            let effective = self.effective_resources(graph.document)?;
            for (ordinal, use_fact) in summary.resource_uses.iter().cloned().enumerate() {
                let resolved = resolve_resource_use(graph.document, &effective, &use_fact);
                push_linked(
                    &mut objects,
                    ModelObjectRef::ResourceUse(ResourceUseModel::new(
                        graph.document,
                        self.key,
                        self.page_ordinal,
                        self.ordinal,
                        ordinal,
                        use_fact,
                        resolved,
                    )),
                    max_objects,
                )?;
            }
        }
        Ok(objects)
    }
}

/// Content-stream operator model wrapper.
#[derive(Clone, Debug)]
pub struct OperatorModel<'a> {
    document: &'a ParsedDocument,
    source: ObjectKey,
    page_ordinal: usize,
    stream_ordinal: usize,
    ordinal: usize,
    fact: OperatorFact,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> OperatorModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        source: ObjectKey,
        page_ordinal: usize,
        stream_ordinal: usize,
        ordinal: usize,
        fact: OperatorFact,
    ) -> Self {
        let supertypes = if fact.is_unknown() {
            vec![
                ObjectTypeName::unchecked("undefinedOperator"),
                ObjectTypeName::unchecked("object"),
            ]
        } else {
            vec![ObjectTypeName::unchecked("object")]
        };
        Self {
            document,
            source,
            page_ordinal,
            stream_ordinal,
            ordinal,
            fact,
            object_type: ObjectTypeName::unchecked("operator"),
            supertypes,
            links: OPERATOR_LINKS
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
        }
    }
}

impl ModelObject for OperatorModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "operator:{}:{}:{}:{}",
                self.page_ordinal, self.stream_ordinal, self.source.number, self.ordinal
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
        Some("operator")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "op" | "name" => Ok(ModelValue::String(BoundedText::unchecked(
                self.fact.op_name(),
            ))),
            "family" => Ok(ModelValue::String(BoundedText::unchecked(
                self.fact.family(),
            ))),
            "operandCount" => Ok(ModelValue::Number(f64::from(self.fact.operand_count()))),
            "location" => Ok(location_model_value(self.fact.location())),
            "isUnknown" => Ok(ModelValue::Bool(self.fact.is_unknown())),
            "textBytes" => Ok(ModelValue::Number(operator_text_bytes(&self.fact)?)),
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

/// Marked-content model wrapper.
#[derive(Clone, Debug)]
pub struct MarkedContentModel<'a> {
    document: &'a ParsedDocument,
    source: ObjectKey,
    page_ordinal: usize,
    stream_ordinal: usize,
    ordinal: usize,
    span: MarkedContentSpan,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> MarkedContentModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        source: ObjectKey,
        page_ordinal: usize,
        stream_ordinal: usize,
        ordinal: usize,
        span: MarkedContentSpan,
    ) -> Self {
        Self {
            document,
            source,
            page_ordinal,
            stream_ordinal,
            ordinal,
            span,
            object_type: ObjectTypeName::unchecked("markedContent"),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: Vec::new(),
        }
    }
}

impl ModelObject for MarkedContentModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "markedContent:{}:{}:{}:{}",
                self.page_ordinal, self.stream_ordinal, self.source.number, self.ordinal
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
        Some("markedContent")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "tag" => Ok(ModelValue::String(BoundedText::unchecked(
                String::from_utf8_lossy(self.span.tag.as_bytes()).into_owned(),
            ))),
            "hasProperties" => Ok(ModelValue::Bool(self.span.properties.is_some())),
            "nestingDepth" => Ok(ModelValue::Number(f64::from(self.span.nesting_depth))),
            "location" => Ok(location_model_value(&self.span.location)),
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

/// Inline-image model wrapper.
#[derive(Clone, Debug)]
pub struct InlineImageModel<'a> {
    document: &'a ParsedDocument,
    source: ObjectKey,
    page_ordinal: usize,
    stream_ordinal: usize,
    ordinal: usize,
    fact: OperatorFact,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> InlineImageModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        source: ObjectKey,
        page_ordinal: usize,
        stream_ordinal: usize,
        ordinal: usize,
        fact: OperatorFact,
    ) -> Self {
        Self {
            document,
            source,
            page_ordinal,
            stream_ordinal,
            ordinal,
            fact,
            object_type: ObjectTypeName::unchecked("inlineImage"),
            supertypes: vec![ObjectTypeName::unchecked("operator")],
            links: Vec::new(),
        }
    }
}

impl ModelObject for InlineImageModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "inlineImage:{}:{}:{}:{}",
                self.page_ordinal, self.stream_ordinal, self.source.number, self.ordinal
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
        Some("inlineImage")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        let OperatorFact::InlineImage {
            width,
            height,
            filters,
            color_space,
            bits_per_component,
            location,
        } = &self.fact
        else {
            return Err(crate::ProfileError::UnknownProperty {
                property: BoundedText::unchecked(name.as_str()),
            }
            .into());
        };
        match name.as_str() {
            "width" => optional_u64_model_value(*width),
            "height" => optional_u64_model_value(*height),
            "filters" => Ok(ModelValue::List(
                filters
                    .iter()
                    .map(|filter| {
                        ModelValue::String(BoundedText::unchecked(
                            String::from_utf8_lossy(filter.as_bytes()).into_owned(),
                        ))
                    })
                    .collect(),
            )),
            "colorSpace" => Ok(color_space.as_ref().map_or(ModelValue::Null, |name| {
                ModelValue::String(BoundedText::unchecked(
                    String::from_utf8_lossy(name.as_bytes()).into_owned(),
                ))
            })),
            "bitsPerComponent" => optional_u64_model_value(*bits_per_component),
            "location" => Ok(location_model_value(location)),
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

/// Content-stream resource-use model wrapper.
#[derive(Clone, Debug)]
pub struct ResourceUseModel<'a> {
    document: &'a ParsedDocument,
    source: ObjectKey,
    page_ordinal: usize,
    stream_ordinal: usize,
    ordinal: usize,
    use_fact: ResourceUse,
    resolved: ResolvedResourceUse,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

impl<'a> ResourceUseModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        source: ObjectKey,
        page_ordinal: usize,
        stream_ordinal: usize,
        ordinal: usize,
        use_fact: ResourceUse,
        resolved: ResolvedResourceUse,
    ) -> Self {
        Self {
            document,
            source,
            page_ordinal,
            stream_ordinal,
            ordinal,
            use_fact,
            resolved,
            object_type: ObjectTypeName::unchecked("resourceUse"),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: Vec::new(),
        }
    }
}

impl ModelObject for ResourceUseModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: format!(
                "resourceUse:{}:{}:{}:{}",
                self.page_ordinal, self.stream_ordinal, self.source.number, self.ordinal
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
        Some("resourceUse")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match name.as_str() {
            "family" => Ok(ModelValue::String(BoundedText::unchecked(
                self.use_fact.family.as_str(),
            ))),
            "name" => Ok(ModelValue::String(BoundedText::unchecked(
                String::from_utf8_lossy(self.use_fact.name.as_bytes()).into_owned(),
            ))),
            "operator" => Ok(ModelValue::String(BoundedText::unchecked(
                self.use_fact.operator.as_str(),
            ))),
            "location" => Ok(location_model_value(&self.use_fact.location)),
            "status" => Ok(ModelValue::String(BoundedText::unchecked(
                self.resolved.status.as_str(),
            ))),
            "resolvedFamily" => Ok(self
                .resolved
                .object_family
                .map_or(ModelValue::Null, |family| {
                    ModelValue::String(BoundedText::unchecked(family))
                })),
            "resolvedObject" => Ok(self
                .resolved
                .object
                .map_or(ModelValue::Null, ModelValue::ObjectKey)),
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

/// Accessibility semantic model wrapper.
#[derive(Clone, Debug)]
pub struct AccessibilityModel<'a> {
    document: &'a ParsedDocument,
    graph: Arc<AccessibilityGraph>,
    kind: AccessibilityModelKind,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
}

#[derive(Clone, Debug)]
enum AccessibilityModelKind {
    Document,
    StructureElement(AccessibilityNodeId),
    TextChunk(usize),
    ImageChunk(usize),
    Annotation(usize),
    Artifact(usize),
    Table(AccessibilityNodeId),
    List(AccessibilityNodeId),
    Heading(AccessibilityNodeId),
    Link(AccessibilityNodeId),
}

impl<'a> AccessibilityModel<'a> {
    fn document(document: &'a ParsedDocument, graph: Arc<AccessibilityGraph>) -> Self {
        Self {
            document,
            graph,
            kind: AccessibilityModelKind::Document,
            object_type: ObjectTypeName::unchecked("accessibilityDocument"),
            supertypes: vec![ObjectTypeName::unchecked("structureTreeRoot")],
            links: ACCESSIBILITY_DOCUMENT_LINKS
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
        }
    }

    fn structure_element(
        document: &'a ParsedDocument,
        graph: Arc<AccessibilityGraph>,
        node: AccessibilityNodeId,
    ) -> Self {
        Self {
            document,
            graph,
            kind: AccessibilityModelKind::StructureElement(node),
            object_type: ObjectTypeName::unchecked("structureElement"),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: STRUCTURE_ELEMENT_LINKS
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
        }
    }

    fn semantic(
        document: &'a ParsedDocument,
        graph: Arc<AccessibilityGraph>,
        kind: AccessibilityModelKind,
        family: &'static str,
    ) -> Self {
        Self {
            document,
            graph,
            kind,
            object_type: ObjectTypeName::unchecked(family),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: Vec::new(),
        }
    }

    fn node(&self, id: AccessibilityNodeId) -> Result<&AccessibilityNode> {
        self.graph.node(id).ok_or(
            ValidationError::LimitExceeded {
                limit: "accessibility_node",
            }
            .into(),
        )
    }

    fn marked(&self, ordinal: usize) -> Result<&AccessibilityMarkedContent> {
        self.graph.marked_content.get(ordinal).ok_or(
            ValidationError::LimitExceeded {
                limit: "accessibility_marked_content",
            }
            .into(),
        )
    }

    fn annotation(&self, ordinal: usize) -> Result<&AccessibilityObjectReference> {
        self.graph.object_references.get(ordinal).ok_or(
            ValidationError::LimitExceeded {
                limit: "accessibility_annotation",
            }
            .into(),
        )
    }

    fn artifact(&self, ordinal: usize) -> Result<&AccessibilityArtifact> {
        self.graph.artifacts.get(ordinal).ok_or(
            ValidationError::LimitExceeded {
                limit: "accessibility_artifact",
            }
            .into(),
        )
    }

    fn location(&self) -> ObjectLocation {
        match &self.kind {
            AccessibilityModelKind::Document => ObjectLocation {
                object: None,
                offset: None,
                path: Some(BoundedText::unchecked("root/accessibility")),
            },
            AccessibilityModelKind::StructureElement(node)
            | AccessibilityModelKind::Table(node)
            | AccessibilityModelKind::List(node)
            | AccessibilityModelKind::Heading(node)
            | AccessibilityModelKind::Link(node) => self
                .graph
                .node(*node)
                .map_or_else(unknown_location, |node| node.location.clone()),
            AccessibilityModelKind::TextChunk(ordinal)
            | AccessibilityModelKind::ImageChunk(ordinal) => self
                .graph
                .marked_content
                .get(*ordinal)
                .map_or_else(unknown_location, |marked| marked.location.clone()),
            AccessibilityModelKind::Annotation(ordinal) => {
                let context = format!("root/accessibility/annotation[{ordinal}]");
                ObjectLocation {
                    object: self
                        .graph
                        .object_references
                        .get(*ordinal)
                        .and_then(|reference| reference.object),
                    offset: None,
                    path: Some(BoundedText::unchecked(context)),
                }
            }
            AccessibilityModelKind::Artifact(ordinal) => self
                .graph
                .artifacts
                .get(*ordinal)
                .map_or_else(unknown_location, |artifact| artifact.location.clone()),
        }
    }

    fn context(&self) -> BoundedText {
        match &self.kind {
            AccessibilityModelKind::Document => BoundedText::unchecked("root/accessibility"),
            AccessibilityModelKind::StructureElement(node) => {
                BoundedText::unchecked(format!("root/accessibility/structureElement[{}]", node.0))
            }
            AccessibilityModelKind::TextChunk(ordinal) => {
                BoundedText::unchecked(format!("root/accessibility/textChunk[{ordinal}]"))
            }
            AccessibilityModelKind::ImageChunk(ordinal) => {
                BoundedText::unchecked(format!("root/accessibility/imageChunk[{ordinal}]"))
            }
            AccessibilityModelKind::Annotation(ordinal) => {
                BoundedText::unchecked(format!("root/accessibility/annotation[{ordinal}]"))
            }
            AccessibilityModelKind::Artifact(ordinal) => {
                BoundedText::unchecked(format!("root/accessibility/artifact[{ordinal}]"))
            }
            AccessibilityModelKind::Table(node) => {
                BoundedText::unchecked(format!("root/accessibility/table[{}]", node.0))
            }
            AccessibilityModelKind::List(node) => {
                BoundedText::unchecked(format!("root/accessibility/list[{}]", node.0))
            }
            AccessibilityModelKind::Heading(node) => {
                BoundedText::unchecked(format!("root/accessibility/heading[{}]", node.0))
            }
            AccessibilityModelKind::Link(node) => {
                BoundedText::unchecked(format!("root/accessibility/link[{}]", node.0))
            }
        }
    }

    fn identity_key(&self) -> String {
        match &self.kind {
            AccessibilityModelKind::Document => String::from("accessibilityDocument"),
            AccessibilityModelKind::StructureElement(node) => {
                format!("structureElement:{}", node.0)
            }
            AccessibilityModelKind::TextChunk(ordinal) => format!("textChunk:{ordinal}"),
            AccessibilityModelKind::ImageChunk(ordinal) => format!("imageChunk:{ordinal}"),
            AccessibilityModelKind::Annotation(ordinal) => {
                format!("accessibilityAnnotation:{ordinal}")
            }
            AccessibilityModelKind::Artifact(ordinal) => format!("artifact:{ordinal}"),
            AccessibilityModelKind::Table(node) => format!("table:{}", node.0),
            AccessibilityModelKind::List(node) => format!("list:{}", node.0),
            AccessibilityModelKind::Heading(node) => format!("heading:{}", node.0),
            AccessibilityModelKind::Link(node) => format!("link:{}", node.0),
        }
    }
}

impl ModelObject for AccessibilityModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: self.identity_key(),
        })
    }

    fn object_type(&self) -> ObjectTypeName {
        self.object_type.clone()
    }

    fn super_types(&self) -> &[ObjectTypeName] {
        &self.supertypes
    }

    fn extra_context(&self) -> Option<&str> {
        Some("accessibility")
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match &self.kind {
            AccessibilityModelKind::Document => accessibility_document_property(&self.graph, name),
            AccessibilityModelKind::StructureElement(node) => {
                structure_element_property(&self.graph, self.node(*node)?, name)
            }
            AccessibilityModelKind::TextChunk(ordinal) => {
                text_chunk_property(&self.graph, self.marked(*ordinal)?, name)
            }
            AccessibilityModelKind::ImageChunk(ordinal) => {
                image_chunk_property(&self.graph, self.marked(*ordinal)?, name)
            }
            AccessibilityModelKind::Annotation(ordinal) => {
                annotation_property(&self.graph, self.annotation(*ordinal)?, name)
            }
            AccessibilityModelKind::Artifact(ordinal) => {
                artifact_property(&self.graph, self.artifact(*ordinal)?, name)
            }
            AccessibilityModelKind::Table(node) => table_property(self.node(*node)?, name),
            AccessibilityModelKind::List(node) => list_property(self.node(*node)?, name),
            AccessibilityModelKind::Heading(node) => heading_property(self.node(*node)?, name),
            AccessibilityModelKind::Link(node) => {
                link_property(&self.graph, self.node(*node)?, name)
            }
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
        match self.kind {
            AccessibilityModelKind::Document => {
                for node in &self.graph.nodes {
                    push_linked(
                        &mut objects,
                        ModelObjectRef::Accessibility(AccessibilityModel::structure_element(
                            graph.document,
                            Arc::clone(&self.graph),
                            node.id,
                        )),
                        max_objects,
                    )?;
                }
                push_accessibility_semantics(
                    graph.document,
                    &self.graph,
                    &mut objects,
                    max_objects,
                )?;
            }
            AccessibilityModelKind::StructureElement(node_id) => {
                let node = self.node(node_id)?;
                for child in &node.children {
                    push_linked(
                        &mut objects,
                        ModelObjectRef::Accessibility(AccessibilityModel::structure_element(
                            graph.document,
                            Arc::clone(&self.graph),
                            *child,
                        )),
                        max_objects,
                    )?;
                }
            }
            _ => {}
        }
        Ok(objects)
    }
}

fn push_accessibility_semantics<'a>(
    document: &'a ParsedDocument,
    graph: &Arc<AccessibilityGraph>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    for (ordinal, marked) in graph.marked_content.iter().enumerate() {
        if marked.has_text {
            push_linked(
                objects,
                ModelObjectRef::Accessibility(AccessibilityModel::semantic(
                    document,
                    Arc::clone(graph),
                    AccessibilityModelKind::TextChunk(ordinal),
                    "textChunk",
                )),
                max_objects,
            )?;
        }
        if marked.has_image {
            push_linked(
                objects,
                ModelObjectRef::Accessibility(AccessibilityModel::semantic(
                    document,
                    Arc::clone(graph),
                    AccessibilityModelKind::ImageChunk(ordinal),
                    "imageChunk",
                )),
                max_objects,
            )?;
        }
    }
    for ordinal in 0..graph.object_references.len() {
        push_linked(
            objects,
            ModelObjectRef::Accessibility(AccessibilityModel::semantic(
                document,
                Arc::clone(graph),
                AccessibilityModelKind::Annotation(ordinal),
                "accessibilityAnnotation",
            )),
            max_objects,
        )?;
    }
    for ordinal in 0..graph.artifacts.len() {
        push_linked(
            objects,
            ModelObjectRef::Accessibility(AccessibilityModel::semantic(
                document,
                Arc::clone(graph),
                AccessibilityModelKind::Artifact(ordinal),
                "artifact",
            )),
            max_objects,
        )?;
    }
    for node in &graph.nodes {
        let Some(family) = semantic_node_family(&node.normalized_role) else {
            continue;
        };
        let kind = match family {
            "table" => AccessibilityModelKind::Table(node.id),
            "list" => AccessibilityModelKind::List(node.id),
            "heading" => AccessibilityModelKind::Heading(node.id),
            "link" => AccessibilityModelKind::Link(node.id),
            _ => continue,
        };
        push_linked(
            objects,
            ModelObjectRef::Accessibility(AccessibilityModel::semantic(
                document,
                Arc::clone(graph),
                kind,
                family,
            )),
            max_objects,
        )?;
    }
    Ok(())
}

fn accessibility_document_property(
    graph: &AccessibilityGraph,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "isTagged" => Ok(ModelValue::Bool(graph.tagged)),
        "language" => Ok(graph.language.as_ref().map_or(ModelValue::Null, |value| {
            ModelValue::String(BoundedText::unchecked(value.clone()))
        })),
        "hasStructTreeRoot" => Ok(ModelValue::Bool(graph.has_structure_tree_root)),
        "roleMapPresent" | "RoleMap" => Ok(ModelValue::Bool(graph.role_map_present)),
        "classMapPresent" | "ClassMap" => Ok(ModelValue::Bool(graph.class_map_present)),
        "idTreeEntries" | "IDTree" => Ok(ModelValue::Number(u64_to_f64(graph.id_tree_entries)?)),
        "parentTreeEntries" | "ParentTree" => {
            Ok(ModelValue::Number(u64_to_f64(graph.parent_tree_entries)?))
        }
        "parentTreeNextKey" | "ParentTreeNextKey" => {
            optional_i64_model_value(graph.parent_tree_next_key)
        }
        "structureElementCount" | "K" => Ok(ModelValue::Number(usize_to_f64(graph.nodes.len())?)),
        "markedContentAssociationCount" => Ok(ModelValue::Number(usize_to_f64(
            graph.marked_content.len(),
        )?)),
        "artifactCount" => Ok(ModelValue::Number(usize_to_f64(graph.artifacts.len())?)),
        "warningCount" => Ok(ModelValue::Number(usize_to_f64(graph.warnings.len())?)),
        "truncated" => Ok(ModelValue::Bool(graph.truncated)),
        "Type" => Ok(ModelValue::String(BoundedText::unchecked("StructTreeRoot"))),
        _ => unknown_property(name),
    }
}

fn structure_element_property(
    graph: &AccessibilityGraph,
    node: &AccessibilityNode,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "role" | "S" | "parentType" | "structParentType" => Ok(ModelValue::String(
            BoundedText::unchecked(node.role.clone()),
        )),
        "normalizedRole" | "parentStandardType" | "structParentStandardType" => Ok(
            ModelValue::String(BoundedText::unchecked(node.normalized_role.clone())),
        ),
        "page" | "Pg" => Ok(node.page.map_or(ModelValue::Null, ModelValue::ObjectKey)),
        "pageIndex" => optional_usize_model_value(node.page_ordinal),
        "childCount" => Ok(ModelValue::Number(usize_to_f64(node.children.len())?)),
        "contentItemCount" | "K" => Ok(ModelValue::Number(usize_to_f64(node.content_items.len())?)),
        "associatedMarkedContentCount" => Ok(ModelValue::Number(usize_to_f64(
            graph
                .marked_content
                .iter()
                .filter(|marked| marked.node == Some(node.id))
                .count(),
        )?)),
        "associatedAnnotationCount" => Ok(ModelValue::Number(usize_to_f64(
            graph
                .object_references
                .iter()
                .filter(|reference| reference.node == node.id)
                .count(),
        )?)),
        "hasAltText" => Ok(ModelValue::Bool(node.has_alt_text)),
        "altTextBytes" => Ok(ModelValue::Number(u64_to_f64(node.alt_text_bytes)?)),
        "hasActualText" => Ok(ModelValue::Bool(node.has_actual_text)),
        "actualTextBytes" => Ok(ModelValue::Number(u64_to_f64(node.actual_text_bytes)?)),
        "hasLanguage" => Ok(ModelValue::Bool(node.has_language)),
        "hasAttributes" => Ok(ModelValue::Bool(node.has_attributes)),
        "hasClass" => Ok(ModelValue::Bool(node.has_class)),
        "hasId" => Ok(ModelValue::Bool(node.has_id)),
        "Alt" | "ActualText" | "Lang" | "A" | "C" | "ID" => {
            Ok(ModelValue::Bool(match name.as_str() {
                "Alt" => node.has_alt_text,
                "ActualText" => node.has_actual_text,
                "Lang" => node.has_language,
                "A" => node.has_attributes,
                "C" => node.has_class,
                "ID" => node.has_id,
                _ => false,
            }))
        }
        "P" | "containsParent" => Ok(ModelValue::Bool(node.contains_parent)),
        "containsRef" => Ok(ModelValue::Bool(node.contains_ref)),
        "parentStandardTypeNamespaceURL"
        | "parentNamespaceURL"
        | "firstChildStandardTypeNamespaceURL"
        | "ListNumbering"
        | "NoteType" => Ok(ModelValue::Null),
        "kidsStandardTypes" => Ok(ModelValue::List(
            node.children
                .iter()
                .filter_map(|child| graph.node(*child))
                .map(|child| {
                    ModelValue::String(BoundedText::unchecked(child.normalized_role.clone()))
                })
                .collect(),
        )),
        "hasContentItems" | "isTaggedContent" => {
            Ok(ModelValue::Bool(!node.content_items.is_empty()))
        }
        "containsLabels" => Ok(ModelValue::Bool(node.children.iter().any(|child| {
            graph
                .node(*child)
                .is_some_and(|child| child.normalized_role == "Lbl")
        }))),
        "orphanRefs"
        | "ghostRefs"
        | "hasIntersection"
        | "wrongColumnSpan"
        | "differentTargetAnnotObjectKey" => Ok(ModelValue::Bool(false)),
        "isArtifact" => Ok(ModelValue::Bool(node.normalized_role == "Artifact")),
        "parentsTags" => Ok(parent_tags(graph, node)),
        "isNotMappedToStandardType" => Ok(ModelValue::Bool(node.non_standard_role)),
        "circularMappingExist" => Ok(ModelValue::Bool(node.circular_role_mapping)),
        "roleMapToSameNamespaceTag" => Ok(ModelValue::Bool(node.role == node.normalized_role)),
        "remappedStandardType" => Ok(ModelValue::Bool(node.role != node.normalized_role)),
        "numberOfColumnWithWrongRowSpan" | "numberOfRowWithWrongColumnSpan" => {
            Ok(ModelValue::Number(0.0))
        }
        "Type" => Ok(ModelValue::String(BoundedText::unchecked("StructElem"))),
        _ => unknown_property(name),
    }
}

fn text_chunk_property(
    graph: &AccessibilityGraph,
    marked: &AccessibilityMarkedContent,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "tag" => Ok(ModelValue::String(BoundedText::unchecked(
            marked.tag.clone(),
        ))),
        "mcid" => optional_i64_model_value(marked.mcid),
        "pageIndex" => Ok(ModelValue::Number(usize_to_f64(marked.page_ordinal)?)),
        "structureRole" => Ok(associated_node_string(graph, marked.node, false)),
        "normalizedRole" => Ok(associated_node_string(graph, marked.node, true)),
        "textBytes" => Ok(ModelValue::Number(0.0)),
        "rawText" => Ok(ModelValue::String(BoundedText::unchecked(""))),
        "isArtifact" => Ok(ModelValue::Bool(marked.tag == "Artifact")),
        _ => unknown_property(name),
    }
}

fn image_chunk_property(
    graph: &AccessibilityGraph,
    marked: &AccessibilityMarkedContent,
    name: &PropertyName,
) -> Result<ModelValue> {
    let node = marked.node.and_then(|node| graph.node(node));
    match name.as_str() {
        "tag" => Ok(ModelValue::String(BoundedText::unchecked(
            marked.tag.clone(),
        ))),
        "mcid" => optional_i64_model_value(marked.mcid),
        "pageIndex" => Ok(ModelValue::Number(usize_to_f64(marked.page_ordinal)?)),
        "structureRole" => Ok(associated_node_string(graph, marked.node, false)),
        "normalizedRole" => Ok(associated_node_string(graph, marked.node, true)),
        "hasAltText" => Ok(ModelValue::Bool(node.is_some_and(|node| node.has_alt_text))),
        "altTextBytes" => Ok(ModelValue::Number(u64_to_f64(
            node.map_or(0, |node| node.alt_text_bytes),
        )?)),
        "hasActualText" => Ok(ModelValue::Bool(
            node.is_some_and(|node| node.has_actual_text),
        )),
        "actualTextBytes" => Ok(ModelValue::Number(u64_to_f64(
            node.map_or(0, |node| node.actual_text_bytes),
        )?)),
        "isArtifact" => Ok(ModelValue::Bool(marked.tag == "Artifact")),
        _ => unknown_property(name),
    }
}

fn annotation_property(
    graph: &AccessibilityGraph,
    reference: &AccessibilityObjectReference,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "pageIndex" => optional_usize_model_value(reference.page_ordinal),
        "structureRole" => Ok(associated_node_string(graph, Some(reference.node), false)),
        "normalizedRole" => Ok(associated_node_string(graph, Some(reference.node), true)),
        "isLink" => Ok(ModelValue::Bool(reference.is_link_annotation)),
        "object" => Ok(reference
            .object
            .map_or(ModelValue::Null, ModelValue::ObjectKey)),
        _ => unknown_property(name),
    }
}

fn artifact_property(
    graph: &AccessibilityGraph,
    artifact: &AccessibilityArtifact,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "tag" => Ok(ModelValue::String(BoundedText::unchecked(
            artifact.tag.clone(),
        ))),
        "pageIndex" => optional_usize_model_value(artifact.page_ordinal),
        "structureRole" => Ok(associated_node_string(graph, artifact.node, true)),
        _ => unknown_property(name),
    }
}

fn table_property(node: &AccessibilityNode, name: &PropertyName) -> Result<ModelValue> {
    match name.as_str() {
        "role" => Ok(ModelValue::String(BoundedText::unchecked(
            node.role.clone(),
        ))),
        "normalizedRole" => Ok(ModelValue::String(BoundedText::unchecked(
            node.normalized_role.clone(),
        ))),
        "pageIndex" => optional_usize_model_value(node.page_ordinal),
        "childCount" => Ok(ModelValue::Number(usize_to_f64(node.children.len())?)),
        "hasAttributes" => Ok(ModelValue::Bool(node.has_attributes)),
        "numberOfColumnWithWrongRowSpan" | "numberOfRowWithWrongColumnSpan" => {
            Ok(ModelValue::Number(0.0))
        }
        "wrongColumnSpan" => Ok(ModelValue::Bool(false)),
        _ => unknown_property(name),
    }
}

fn list_property(node: &AccessibilityNode, name: &PropertyName) -> Result<ModelValue> {
    match name.as_str() {
        "role" => Ok(ModelValue::String(BoundedText::unchecked(
            node.role.clone(),
        ))),
        "normalizedRole" => Ok(ModelValue::String(BoundedText::unchecked(
            node.normalized_role.clone(),
        ))),
        "pageIndex" => optional_usize_model_value(node.page_ordinal),
        "childCount" => Ok(ModelValue::Number(usize_to_f64(node.children.len())?)),
        "ListNumbering" => Ok(ModelValue::Null),
        "containsLabels" => Ok(ModelValue::Bool(false)),
        _ => unknown_property(name),
    }
}

fn heading_property(node: &AccessibilityNode, name: &PropertyName) -> Result<ModelValue> {
    match name.as_str() {
        "role" => Ok(ModelValue::String(BoundedText::unchecked(
            node.role.clone(),
        ))),
        "normalizedRole" => Ok(ModelValue::String(BoundedText::unchecked(
            node.normalized_role.clone(),
        ))),
        "pageIndex" => optional_usize_model_value(node.page_ordinal),
        "level" => Ok(ModelValue::Number(heading_level(&node.normalized_role))),
        _ => unknown_property(name),
    }
}

fn link_property(
    graph: &AccessibilityGraph,
    node: &AccessibilityNode,
    name: &PropertyName,
) -> Result<ModelValue> {
    match name.as_str() {
        "role" => Ok(ModelValue::String(BoundedText::unchecked(
            node.role.clone(),
        ))),
        "normalizedRole" => Ok(ModelValue::String(BoundedText::unchecked(
            node.normalized_role.clone(),
        ))),
        "pageIndex" => optional_usize_model_value(node.page_ordinal),
        "associatedAnnotationCount" => Ok(ModelValue::Number(usize_to_f64(
            graph
                .object_references
                .iter()
                .filter(|reference| reference.node == node.id)
                .count(),
        )?)),
        "differentTargetAnnotObjectKey" => Ok(ModelValue::Bool(false)),
        _ => unknown_property(name),
    }
}

fn associated_node_string(
    graph: &AccessibilityGraph,
    node: Option<AccessibilityNodeId>,
    normalized: bool,
) -> ModelValue {
    node.and_then(|node| graph.node(node))
        .map_or(ModelValue::Null, |node| {
            let value = if normalized {
                node.normalized_role.clone()
            } else {
                node.role.clone()
            };
            ModelValue::String(BoundedText::unchecked(value))
        })
}

fn parent_tags(graph: &AccessibilityGraph, node: &AccessibilityNode) -> ModelValue {
    let mut tags = Vec::new();
    let mut current = node.parent;
    while let Some(parent) = current {
        let Some(parent_node) = graph.node(parent) else {
            break;
        };
        tags.push(ModelValue::String(BoundedText::unchecked(
            parent_node.normalized_role.clone(),
        )));
        current = parent_node.parent;
    }
    ModelValue::List(tags)
}

fn semantic_node_family(role: &str) -> Option<&'static str> {
    match role {
        "Table" | "TR" | "TH" | "TD" | "THead" | "TBody" | "TFoot" => Some("table"),
        "L" | "LI" | "Lbl" | "LBody" => Some("list"),
        "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" => Some("heading"),
        "Link" => Some("link"),
        _ => None,
    }
}

fn heading_level(role: &str) -> f64 {
    match role {
        "H1" => 1.0,
        "H2" => 2.0,
        "H3" => 3.0,
        "H4" => 4.0,
        "H5" => 5.0,
        "H6" => 6.0,
        _ => 0.0,
    }
}

fn optional_i64_model_value(value: Option<i64>) -> Result<ModelValue> {
    value.map_or(Ok(ModelValue::Null), |value| {
        Ok(ModelValue::Number(i64_to_f64(value)?))
    })
}

fn optional_usize_model_value(value: Option<usize>) -> Result<ModelValue> {
    value.map_or(Ok(ModelValue::Null), |value| {
        Ok(ModelValue::Number(usize_to_f64(value)?))
    })
}

fn unknown_location() -> ObjectLocation {
    ObjectLocation {
        object: None,
        offset: None,
        path: Some(BoundedText::unchecked("root/accessibility/unknown")),
    }
}

fn operator_text_bytes(fact: &OperatorFact) -> Result<f64> {
    match fact {
        OperatorFact::TextShow { bytes, .. } => u64_to_f64(bytes.bytes),
        _ => Ok(0.0),
    }
}

fn optional_u64_model_value(value: Option<u64>) -> Result<ModelValue> {
    value.map_or(Ok(ModelValue::Null), |value| {
        Ok(ModelValue::Number(u64_to_f64(value)?))
    })
}

fn location_model_value(location: &ObjectLocation) -> ModelValue {
    ModelValue::String(
        location
            .path
            .clone()
            .unwrap_or_else(|| BoundedText::unchecked("unknown")),
    )
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

fn effective_page_resources<'a>(
    document: &'a ParsedDocument,
    page: &PageModel<'a>,
) -> Result<EffectiveResources<'a>> {
    effective_resources_for_page_key(document, page.key, page.limits)
}

fn effective_resources_for_page_key<'a>(
    document: &'a ParsedDocument,
    page_key: ObjectKey,
    limits: &ResourceLimits,
) -> Result<EffectiveResources<'a>> {
    let mut current = Some(page_key);
    let mut visited = HashSet::new();
    let mut dictionaries = Vec::new();
    let mut cyclic = false;
    while let Some(key) = current {
        if !visited.insert(key) {
            cyclic = true;
            break;
        }
        if u64::try_from(visited.len()).map_err(|_| ValidationError::LimitExceeded {
            limit: "max_resource_contexts",
        })? > limits.max_resource_contexts
        {
            return Err(ValidationError::LimitExceeded {
                limit: "max_resource_contexts",
            }
            .into());
        }
        let Some(dictionary) = page_dictionary(document, key) else {
            break;
        };
        if let Some(resources) = resolve_dictionary_value(document, dictionary.get("Resources")) {
            dictionaries.push(resources);
        }
        current = match dictionary.get("Parent") {
            Some(crate::CosObject::Reference(parent)) => Some(*parent),
            _ => None,
        };
    }
    dictionaries.reverse();
    Ok(EffectiveResources {
        dictionaries,
        cyclic,
    })
}

fn resolve_resource_use(
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    use_fact: &ResourceUse,
) -> ResolvedResourceUse {
    if resources.cyclic {
        return ResolvedResourceUse {
            status: ResourceResolutionStatus::Cyclic,
            family: use_fact.family,
            object: None,
            object_family: None,
        };
    }
    let Some(value) = resources
        .dictionaries
        .iter()
        .rev()
        .find_map(|dictionary| resource_value(dictionary, use_fact.family, &use_fact.name))
    else {
        return ResolvedResourceUse {
            status: ResourceResolutionStatus::Missing,
            family: use_fact.family,
            object: None,
            object_family: None,
        };
    };
    let object = match value {
        crate::CosObject::Reference(key) => Some(*key),
        _ => None,
    };
    let object_family = resolved_resource_family(document, use_fact.family, value);
    ResolvedResourceUse {
        status: if object_family.is_some() {
            ResourceResolutionStatus::Resolved
        } else {
            ResourceResolutionStatus::WrongType
        },
        family: use_fact.family,
        object,
        object_family,
    }
}

fn resource_value<'a>(
    resources: &'a crate::Dictionary,
    family: ResourceFamily,
    name: &PdfName,
) -> Option<&'a crate::CosObject> {
    let dictionary_name = match family {
        ResourceFamily::Font => "Font",
        ResourceFamily::ColorSpace => "ColorSpace",
        ResourceFamily::ExtGState => "ExtGState",
        ResourceFamily::Shading => "Shading",
        ResourceFamily::XObject => "XObject",
    };
    let Some(crate::CosObject::Dictionary(collection)) = resources.get(dictionary_name) else {
        return None;
    };
    collection
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn resolved_resource_family(
    document: &ParsedDocument,
    expected: ResourceFamily,
    value: &crate::CosObject,
) -> Option<&'static str> {
    match expected {
        ResourceFamily::Font => {
            resolve_named_dictionary(document, value).and_then(|(_key, _offset, dictionary)| {
                classify_font_dictionary(dictionary).or(Some("font"))
            })
        }
        ResourceFamily::ColorSpace => Some("colorSpace"),
        ResourceFamily::ExtGState => resolve_named_dictionary(document, value)
            .map(|(_key, _offset, _dictionary)| "extGState"),
        ResourceFamily::Shading => {
            resolve_named_dictionary(document, value).map(|(_key, _offset, _dictionary)| "shading")
        }
        ResourceFamily::XObject => resolve_named_dictionary(document, value)
            .and_then(|(_key, _offset, dictionary)| classify_xobject(dictionary)),
    }
}

fn inherited_page_value<'a>(
    document: &'a ParsedDocument,
    page_key: ObjectKey,
    name: &str,
    limits: &ResourceLimits,
) -> Result<Option<&'a crate::CosObject>> {
    let mut current = Some(page_key);
    let mut visited = HashSet::new();
    while let Some(key) = current {
        if !visited.insert(key) {
            return Ok(None);
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
        let Some(dictionary) = page_dictionary(document, key) else {
            return Ok(None);
        };
        if let Some(value) = dictionary.get(name) {
            return Ok(Some(value));
        }
        current = match dictionary.get("Parent") {
            Some(crate::CosObject::Reference(parent)) => Some(*parent),
            _ => None,
        };
    }
    Ok(None)
}

fn classify_font_dictionary(dictionary: &crate::Dictionary) -> Option<&'static str> {
    match dictionary.get("Subtype") {
        Some(crate::CosObject::Name(name)) if name.matches("Type0") => Some("type0"),
        Some(crate::CosObject::Name(name)) if name.matches("Type1") => Some("type1"),
        Some(crate::CosObject::Name(name)) if name.matches("MMType1") => Some("mmType1"),
        Some(crate::CosObject::Name(name)) if name.matches("TrueType") => Some("trueType"),
        Some(crate::CosObject::Name(name)) if name.matches("Type3") => Some("type3"),
        Some(crate::CosObject::Name(name)) if name.matches("CIDFontType0") => Some("cidFontType0"),
        Some(crate::CosObject::Name(name)) if name.matches("CIDFontType2") => Some("cidFontType2"),
        _ => None,
    }
}

fn font_embedded(document: &ParsedDocument, dictionary: &crate::Dictionary) -> bool {
    font_program_stream(document, dictionary).is_some()
}

fn font_program_bytes(document: &ParsedDocument, dictionary: &crate::Dictionary) -> Result<u64> {
    font_program_stream(document, dictionary).map_or(Ok(0), |stream| {
        Ok(stream.declared_length.unwrap_or(stream.discovered_length))
    })
}

fn font_program_stream<'a>(
    document: &'a ParsedDocument,
    dictionary: &'a crate::Dictionary,
) -> Option<&'a crate::StreamObject> {
    let descriptor =
        resolve_dictionary_value(document, dictionary.get("FontDescriptor")).or_else(|| {
            descendant_font_dictionary(document, dictionary).and_then(|descendant| {
                resolve_dictionary_value(document, descendant.get("FontDescriptor"))
            })
        })?;
    ["FontFile", "FontFile2", "FontFile3"]
        .iter()
        .find_map(|name| stream_from_value(document, descriptor.get(name)))
}

fn descendant_font_dictionary<'a>(
    document: &'a ParsedDocument,
    dictionary: &'a crate::Dictionary,
) -> Option<&'a crate::Dictionary> {
    let Some(crate::CosObject::Array(descendants)) = dictionary.get("DescendantFonts") else {
        return None;
    };
    descendants
        .first()
        .and_then(|value| resolve_named_dictionary(document, value))
        .map(|(_key, _offset, dictionary)| dictionary)
}

fn stream_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a crate::CosObject>,
) -> Option<&'a crate::StreamObject> {
    match value? {
        crate::CosObject::Stream(stream) => Some(stream),
        crate::CosObject::Reference(key) => match &document.objects.get(key)?.object {
            crate::CosObject::Stream(stream) => Some(stream),
            _ => None,
        },
        _ => None,
    }
}

fn icc_stream_from_dictionary<'a>(
    document: &'a ParsedDocument,
    dictionary: &'a crate::Dictionary,
) -> Option<&'a crate::StreamObject> {
    stream_from_value(document, dictionary.get("DestOutputProfile"))
}

fn output_intent_icc_header(
    document: &ParsedDocument,
    dictionary: &crate::Dictionary,
    limits: &ResourceLimits,
) -> Result<Option<IccHeader>> {
    let Some(stream) = icc_stream_from_dictionary(document, dictionary) else {
        return Ok(None);
    };
    if stream.discovered_length > limits.max_icc_profile_bytes {
        return Ok(None);
    }
    let bytes = stream.decoded_bytes(limits)?;
    Ok(parse_icc_header(&bytes))
}

fn parse_icc_header(bytes: &[u8]) -> Option<IccHeader> {
    if bytes.len() < 132 {
        return None;
    }
    let profile_size = u32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?);
    let major = *bytes.get(8)?;
    let minor = *bytes.get(9)? >> 4;
    let bugfix = *bytes.get(9)? & 0x0f;
    let rendering_intent = u32::from_be_bytes(bytes.get(64..68)?.try_into().ok()?);
    let tag_count = u32::from_be_bytes(bytes.get(128..132)?.try_into().ok()?);
    Some(IccHeader {
        profile_size: u64::from(profile_size),
        version: format!("{major}.{minor}.{bugfix}"),
        device_class: ascii_tag(bytes.get(12..16)?),
        color_space: ascii_tag(bytes.get(16..20)?),
        pcs: ascii_tag(bytes.get(20..24)?),
        rendering_intent: u64::from(rendering_intent),
        tag_count: u64::from(tag_count),
    })
}

fn ascii_tag(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

fn color_space_family(dictionary: &crate::Dictionary) -> String {
    if let Some(crate::CosObject::Name(name)) = dictionary.get("Family") {
        return String::from_utf8_lossy(name.as_bytes()).into_owned();
    }
    if dictionary.get("N").is_some() {
        return String::from("ICCBased");
    }
    if dictionary.get("HiVal").is_some() || dictionary.get("Lookup").is_some() {
        return String::from("Indexed");
    }
    if dictionary.get("TintTransform").is_some() && dictionary.get("Colorants").is_some() {
        return String::from("DeviceN");
    }
    if dictionary.get("TintTransform").is_some() {
        return String::from("Separation");
    }
    String::from("unknown")
}

fn color_space_component_count(dictionary: &crate::Dictionary) -> Option<u64> {
    match dictionary.get("N") {
        Some(crate::CosObject::Integer(value)) => u64::try_from(*value).ok(),
        _ => match color_space_family(dictionary).as_str() {
            "DeviceGray" | "CalGray" => Some(1),
            "DeviceRGB" | "CalRGB" | "Lab" => Some(3),
            "DeviceCMYK" => Some(4),
            _ => None,
        },
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
    page: &PageModel<'a>,
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
    page: &PageModel<'a>,
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
        limits: page.limits,
        page_key: Some(page.key),
        resource_context: None,
        context_prefix: format!("root/page[{}]", page.ordinal),
        page_ordinal: page.ordinal,
        ordinal,
        key,
        offset: object.offset,
        stream,
        summary_cache: Arc::new(OnceLock::new()),
        object_type: ObjectTypeName::unchecked("contentStream"),
        supertypes: vec![
            ObjectTypeName::unchecked("stream"),
            ObjectTypeName::unchecked("object"),
        ],
        links: CONTENT_STREAM_LINKS
            .iter()
            .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
            .collect(),
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

fn action_models_from_dictionary_entries<'a>(
    document: &'a ParsedDocument,
    dictionary: &crate::Dictionary,
    keys: &[&str],
    owner_ordinal: usize,
    context_prefix: &str,
    max_objects: usize,
) -> Result<Vec<GenericModel<'a>>> {
    let mut actions = Vec::new();
    for key in keys {
        let context = format!("{context_prefix}[{owner_ordinal}]/{key}");
        if *key == "AA" {
            collect_additional_action_models_from_value(
                document,
                dictionary.get(key),
                owner_ordinal,
                &context,
                max_objects,
                &mut actions,
            )?;
        } else {
            collect_action_models_from_value(
                document,
                dictionary.get(key),
                &context,
                max_objects,
                &mut actions,
            )?;
        }
    }
    Ok(actions)
}

fn collect_action_models_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    context: &str,
    max_objects: usize,
    actions: &mut Vec<GenericModel<'a>>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if let Some(model) = dictionary_backed_model_from_value(
        document,
        Some(value),
        "action",
        actions.len(),
        context.to_owned(),
    ) {
        push_generic_model(actions, model, max_objects)?;
    }
    Ok(())
}

fn collect_additional_action_models_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    owner_ordinal: usize,
    context: &str,
    max_objects: usize,
    actions: &mut Vec<GenericModel<'a>>,
) -> Result<()> {
    let Some(additional_actions) = resolve_dictionary_value(document, value) else {
        return Ok(());
    };
    for (name, action) in additional_actions.iter() {
        let action_context = format!(
            "{context}/action[{}:{}]",
            owner_ordinal,
            String::from_utf8_lossy(name.as_bytes())
        );
        if let Some(model) = dictionary_backed_model_from_value(
            document,
            Some(action),
            "action",
            actions.len(),
            action_context,
        ) {
            push_generic_model(actions, model, max_objects)?;
        }
    }
    Ok(())
}

fn file_spec_model_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    ordinal: usize,
    context: String,
) -> Option<GenericModel<'a>> {
    dictionary_backed_model_from_value(document, value, "fileSpec", ordinal, context)
}

fn destination_model_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    ordinal: usize,
    context: String,
) -> Option<GenericModel<'a>> {
    model_from_value(document, value, "destination", ordinal, context)
}

fn dictionary_backed_model_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    family: &'static str,
    ordinal: usize,
    context: String,
) -> Option<GenericModel<'a>> {
    match value? {
        crate::CosObject::Dictionary(dictionary) => Some(GenericModel::new(
            document, family, None, None, dictionary, ordinal, context,
        )),
        crate::CosObject::Reference(key) => {
            let object = document.objects.get(key)?;
            let dictionary = object.object.as_dictionary()?;
            Some(GenericModel::new(
                document,
                family,
                Some(*key),
                Some(object.offset),
                dictionary,
                ordinal,
                context,
            ))
        }
        _ => None,
    }
}

fn model_from_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    family: &'static str,
    ordinal: usize,
    context: String,
) -> Option<GenericModel<'a>> {
    match value? {
        crate::CosObject::Array(values) if family == "destination" => {
            Some(GenericModel::new_value(
                document,
                family,
                ModelValue::from(crate::CosObject::Array(values.clone())),
                ordinal,
                context,
            ))
        }
        value => {
            dictionary_backed_model_from_value(document, Some(value), family, ordinal, context)
        }
    }
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
    dictionary: crate::Dictionary,
    object_type: ObjectTypeName,
    supertypes: Vec<ObjectTypeName>,
    links: Vec<LinkName>,
    allowed_properties: &'static [&'static str],
    direct_value: Option<ModelValue>,
    context: String,
    ordinal: usize,
}

impl<'a> GenericModel<'a> {
    fn new(
        document: &'a ParsedDocument,
        family: &'static str,
        key: Option<ObjectKey>,
        offset: Option<u64>,
        dictionary: &crate::Dictionary,
        ordinal: usize,
        context: impl Into<String>,
    ) -> Self {
        Self {
            document,
            key,
            offset,
            dictionary: dictionary.clone(),
            object_type: ObjectTypeName::unchecked(family),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: family_links(family)
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
            allowed_properties: family_direct_properties(family),
            direct_value: None,
            context: context.into(),
            ordinal,
        }
    }

    fn new_value(
        document: &'a ParsedDocument,
        family: &'static str,
        value: ModelValue,
        ordinal: usize,
        context: impl Into<String>,
    ) -> Self {
        Self {
            document,
            key: None,
            offset: None,
            dictionary: crate::Dictionary::default(),
            object_type: ObjectTypeName::unchecked(family),
            supertypes: vec![ObjectTypeName::unchecked("object")],
            links: family_links(family)
                .iter()
                .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
                .collect(),
            allowed_properties: family_direct_properties(family),
            direct_value: Some(value),
            context: context.into(),
            ordinal,
        }
    }
}

impl ModelObject for GenericModel<'_> {
    fn id(&self) -> Option<ObjectIdentity> {
        Some(ObjectIdentity {
            key: generic_identity_key(
                self.object_type.as_str(),
                self.key,
                self.ordinal,
                self.context.as_str(),
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
        Some(&self.context)
    }

    fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match (self.object_type.as_str(), name.as_str()) {
            ("destination", "D" | "Dest") => Ok(self.direct_value.clone().map_or_else(
                || dictionary_property(&self.dictionary, name, self.allowed_properties),
                Ok,
            )?),
            ("image", "width") => dictionary_property(
                &self.dictionary,
                &PropertyName::unchecked("Width"),
                IMAGE_DIRECT_PROPERTIES,
            ),
            ("image", "height") => dictionary_property(
                &self.dictionary,
                &PropertyName::unchecked("Height"),
                IMAGE_DIRECT_PROPERTIES,
            ),
            ("colorSpace", "family") => Ok(ModelValue::String(BoundedText::unchecked(
                color_space_family(&self.dictionary),
            ))),
            ("colorSpace", "componentCount") => {
                optional_u64_model_value(color_space_component_count(&self.dictionary))
            }
            ("colorSpace", "hasAlternate") => {
                Ok(ModelValue::Bool(self.dictionary.get("Alternate").is_some()))
            }
            ("colorSpace", "hasTintTransform") => Ok(ModelValue::Bool(
                self.dictionary.get("TintTransform").is_some(),
            )),
            ("colorSpace", "hasICCProfile") => Ok(ModelValue::Bool(
                color_space_family(&self.dictionary) == "ICCBased"
                    || self.object_type.as_str() == "iccProfile",
            )),
            ("extGState", "hasSoftMask") => {
                Ok(ModelValue::Bool(self.dictionary.get("SMask").is_some_and(
                    |value| !matches!(value, crate::CosObject::Name(name) if name.matches("None")),
                )))
            }
            ("extGState", "hasBlendMode") => {
                Ok(ModelValue::Bool(self.dictionary.get("BM").is_some()))
            }
            ("extGState", "alphaSource") => Ok(self
                .dictionary
                .get("AIS")
                .cloned()
                .map_or(ModelValue::Null, ModelValue::from)),
            ("cMap", "hasCIDSystemInfo") => Ok(ModelValue::Bool(
                self.dictionary.get("CIDSystemInfo").is_some(),
            )),
            ("cMap", "hasUseCMap") => {
                Ok(ModelValue::Bool(self.dictionary.get("UseCMap").is_some()))
            }
            ("cMap", "embedded") => Ok(ModelValue::Bool(self.key.is_some())),
            ("contentStream", "operatorCount" | "markedContentCount") => {
                Ok(ModelValue::Number(0.0))
            }
            _ => dictionary_property(&self.dictionary, name, self.allowed_properties),
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

fn generic_identity_key(
    family: &str,
    key: Option<ObjectKey>,
    ordinal: usize,
    context: &str,
) -> String {
    key.map_or_else(
        || format!("{family}:inline:{ordinal}:{context}"),
        |key| format!("{family}:{}:{}", key.number, key.generation),
    )
}

fn generic_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    match model.object_type.as_str() {
        "acroForm" => generic_models_from_dictionary_value(
            graph.document,
            model.dictionary.get("Fields"),
            "formField",
            &model.context,
            max_objects,
        ),
        "formField" => form_field_linked_objects(model, graph, max_objects),
        "action" => action_linked_objects(model, graph, max_objects),
        "outline" => outline_linked_objects(model, graph, max_objects),
        "names" => names_linked_objects(model, graph, max_objects),
        "destination" => destination_linked_objects(model, graph, max_objects),
        "xObject" | "formXObject" => xobject_linked_objects(model, graph, max_objects),
        _ => Ok(Vec::new()),
    }
}

fn xobject_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    if !graph.materializes("contentStream") {
        return Ok(Vec::new());
    }
    let Some(key) = model.key else {
        return Ok(Vec::new());
    };
    let Some(object) = graph.document.objects.get(&key) else {
        return Ok(Vec::new());
    };
    let crate::CosObject::Stream(stream) = &object.object else {
        return Ok(Vec::new());
    };
    let resource_context =
        resolve_dictionary_value(graph.document, model.dictionary.get("Resources")).cloned();
    let model = ContentStreamModel {
        document: graph.document,
        limits: graph.limits,
        page_key: None,
        resource_context,
        context_prefix: model.context.clone(),
        page_ordinal: model.ordinal,
        ordinal: 0,
        key,
        offset: object.offset,
        stream,
        summary_cache: Arc::new(OnceLock::new()),
        object_type: ObjectTypeName::unchecked("contentStream"),
        supertypes: vec![
            ObjectTypeName::unchecked("stream"),
            ObjectTypeName::unchecked("object"),
        ],
        links: CONTENT_STREAM_LINKS
            .iter()
            .map(|(name, _target)| LinkName(Identifier::unchecked(*name)))
            .collect(),
    };
    let mut objects = Vec::new();
    push_linked(
        &mut objects,
        ModelObjectRef::ContentStream(model),
        max_objects,
    )?;
    Ok(objects)
}

fn generic_models_from_dictionary_value<'a>(
    document: &'a ParsedDocument,
    value: Option<&crate::CosObject>,
    family: &'static str,
    context_prefix: &str,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = Vec::new();
    let Some(value) = value else {
        return Ok(objects);
    };
    match value {
        crate::CosObject::Array(values) => {
            for item in values {
                let ordinal = objects.len();
                if let Some(model) = model_from_value(
                    document,
                    Some(item),
                    family,
                    ordinal,
                    format!("{context_prefix}/{family}[{ordinal}]"),
                ) {
                    push_linked(&mut objects, ModelObjectRef::Generic(model), max_objects)?;
                }
            }
        }
        _ => {
            if let Some(model) = model_from_value(
                document,
                Some(value),
                family,
                0,
                format!("{context_prefix}/{family}[0]"),
            ) {
                push_linked(&mut objects, ModelObjectRef::Generic(model), max_objects)?;
            }
        }
    }
    Ok(objects)
}

fn form_field_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = generic_models_from_dictionary_value(
        graph.document,
        model.dictionary.get("Kids"),
        "formField",
        &model.context,
        max_objects,
    )?;
    if let Some(parent) = generic_models_from_dictionary_value(
        graph.document,
        model.dictionary.get("Parent"),
        "formField",
        &model.context,
        max_objects.saturating_sub(objects.len()),
    )?
    .into_iter()
    .next()
    {
        push_linked(&mut objects, parent, max_objects)?;
    }
    let mut actions = action_models_from_dictionary_entries(
        graph.document,
        &model.dictionary,
        &["A", "AA"],
        model.ordinal,
        &model.context,
        max_objects.saturating_sub(objects.len()),
    )?;
    actions.reverse();
    for action in actions {
        push_linked(&mut objects, ModelObjectRef::Generic(action), max_objects)?;
    }
    if let Some(file_spec) = file_spec_model_from_value(
        graph.document,
        model.dictionary.get("F"),
        model.ordinal,
        format!("{}/fileSpec[0]", model.context),
    ) {
        push_linked(
            &mut objects,
            ModelObjectRef::Generic(file_spec),
            max_objects,
        )?;
    }
    Ok(objects)
}

fn action_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = generic_models_from_dictionary_value(
        graph.document,
        model.dictionary.get("Next"),
        "action",
        &model.context,
        max_objects,
    )?;
    if let Some(file_spec) = file_spec_model_from_value(
        graph.document,
        model
            .dictionary
            .get("F")
            .or_else(|| model.dictionary.get("FS")),
        model.ordinal,
        format!("{}/fileSpec[0]", model.context),
    ) {
        push_linked(
            &mut objects,
            ModelObjectRef::Generic(file_spec),
            max_objects,
        )?;
    }
    for value_name in ["D"] {
        if let Some(destination) = destination_model_from_value(
            graph.document,
            model.dictionary.get(value_name),
            model.ordinal,
            format!("{}/destination[0]", model.context),
        ) {
            push_linked(
                &mut objects,
                ModelObjectRef::Generic(destination),
                max_objects,
            )?;
        }
    }
    Ok(objects)
}

fn outline_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = Vec::new();
    for key in ["First", "Last", "Next", "Prev"] {
        let linked = generic_models_from_dictionary_value(
            graph.document,
            model.dictionary.get(key),
            "outline",
            &model.context,
            max_objects.saturating_sub(objects.len()),
        )?;
        for object in linked {
            push_linked(&mut objects, object, max_objects)?;
        }
    }
    let mut actions = action_models_from_dictionary_entries(
        graph.document,
        &model.dictionary,
        &["A"],
        model.ordinal,
        &model.context,
        max_objects.saturating_sub(objects.len()),
    )?;
    actions.reverse();
    for action in actions {
        push_linked(&mut objects, ModelObjectRef::Generic(action), max_objects)?;
    }
    if let Some(destination) = destination_model_from_value(
        graph.document,
        model.dictionary.get("Dest"),
        model.ordinal,
        format!("{}/destination[0]", model.context),
    ) {
        push_linked(
            &mut objects,
            ModelObjectRef::Generic(destination),
            max_objects,
        )?;
    }
    Ok(objects)
}

fn names_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = Vec::new();
    for key in ["Dests", "EmbeddedFiles"] {
        let family = if key == "EmbeddedFiles" {
            "fileSpec"
        } else {
            "destination"
        };
        collect_name_tree_entries(
            graph.document,
            model.dictionary.get(key),
            family,
            &model.context,
            max_objects,
            &mut objects,
        )?;
    }
    Ok(objects)
}

fn collect_name_tree_entries<'a>(
    document: &'a ParsedDocument,
    root: Option<&crate::CosObject>,
    family: &'static str,
    context_prefix: &str,
    max_objects: usize,
    objects: &mut Vec<ModelObjectRef<'a>>,
) -> Result<()> {
    let mut stack = Vec::new();
    push_name_tree_node(
        document,
        root,
        context_prefix.to_owned(),
        &mut stack,
        max_objects,
    )?;
    let mut visited = HashSet::new();
    let mut scanned = 0_usize;
    while let Some((key, dictionary, context)) = stack.pop() {
        if let Some(key) = key
            && !visited.insert(key)
        {
            continue;
        }
        scanned = scanned
            .checked_add(1)
            .ok_or(ValidationError::LimitExceeded {
                limit: "max_objects",
            })?;
        if scanned > max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        for (ordinal, value) in array_values(dictionary.get("Names")).enumerate() {
            if ordinal % 2 == 0 {
                continue;
            }
            let model_ordinal = objects.len();
            if let Some(model) = model_from_value(
                document,
                Some(value),
                family,
                model_ordinal,
                format!("{context}/{family}[{model_ordinal}]"),
            ) {
                push_linked(objects, ModelObjectRef::Generic(model), max_objects)?;
            }
        }
        for (ordinal, kid) in array_values(dictionary.get("Kids")).enumerate() {
            push_name_tree_node(
                document,
                Some(kid),
                format!("{context}/kid[{ordinal}]"),
                &mut stack,
                max_objects.saturating_sub(objects.len()),
            )?;
        }
    }
    Ok(())
}

fn push_name_tree_node(
    document: &ParsedDocument,
    value: Option<&crate::CosObject>,
    context: String,
    stack: &mut Vec<(Option<ObjectKey>, crate::Dictionary, String)>,
    max_objects: usize,
) -> Result<()> {
    if stack.len() >= max_objects {
        return Err(ValidationError::LimitExceeded {
            limit: "max_objects",
        }
        .into());
    }
    match value {
        Some(crate::CosObject::Dictionary(dictionary)) => {
            stack.push((None, dictionary.clone(), context));
        }
        Some(crate::CosObject::Reference(key)) => {
            let Some(object) = document.objects.get(key) else {
                return Ok(());
            };
            let Some(dictionary) = object.object.as_dictionary() else {
                return Ok(());
            };
            stack.push((Some(*key), dictionary.clone(), context));
        }
        _ => {}
    }
    Ok(())
}

fn destination_linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    action_models_from_dictionary_entries(
        graph.document,
        &model.dictionary,
        &["A"],
        model.ordinal,
        &model.context,
        max_objects,
    )
    .map(|actions| actions.into_iter().map(ModelObjectRef::Generic).collect())
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
        "fontDescriptor" => FONT_DESCRIPTOR_PROPERTIES,
        "fontProgram" => FONT_PROGRAM_PROPERTIES,
        "image" => IMAGE_DIRECT_PROPERTIES,
        "xObject" | "formXObject" | "postScriptXObject" => XOBJECT_DIRECT_PROPERTIES,
        "pattern" => PATTERN_DIRECT_PROPERTIES,
        "shading" => SHADING_DIRECT_PROPERTIES,
        "function" => FUNCTION_DIRECT_PROPERTIES,
        "action" => ACTION_DIRECT_PROPERTIES,
        "formField" => FORM_FIELD_DIRECT_PROPERTIES,
        "fileSpec" => FILE_SPEC_DIRECT_PROPERTIES,
        "colorSpace" => COLOR_SPACE_DIRECT_PROPERTIES,
        "iccProfile" => ICC_PROFILE_PROPERTIES,
        "extGState" => EXT_GSTATE_DIRECT_PROPERTIES,
        "structureTreeRoot" => STRUCTURE_DIRECT_PROPERTIES,
        "structureElement" => STRUCTURE_ELEMENT_PROPERTIES,
        "signature" => SIGNATURE_DIRECT_PROPERTIES,
        "security" => SECURITY_DIRECT_PROPERTIES,
        "pageTree" => PAGE_TREE_PROPERTIES,
        _ => DIRECT_PROPERTY_NAMES,
    }
}

fn family_links(family: &str) -> &'static [(&'static str, &'static str)] {
    match family {
        "page" => PAGE_LINKS,
        "annotation" => ANNOTATION_LINKS,
        "action" => ACTION_LINKS,
        "formField" => FORM_FIELD_LINKS,
        "acroForm" => ACRO_FORM_LINKS,
        "outline" => OUTLINE_LINKS,
        "names" => NAMES_LINKS,
        "destination" => DESTINATION_LINKS,
        "xObject" | "formXObject" => XOBJECT_LINKS,
        _ => EMPTY_LINK_NAMES,
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
        Some(crate::CosObject::Name(name)) if name.matches("PS") => Some("postScriptXObject"),
        _ => None,
    }
}

fn classify_dictionary(dictionary: &crate::Dictionary) -> Option<&'static str> {
    if let Some(crate::CosObject::Name(name)) = dictionary.get("Subtype") {
        if name.matches("Image") {
            return Some("image");
        }
        if name.matches("Form") {
            return Some("formXObject");
        }
        if name.matches("PS") {
            return Some("postScriptXObject");
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
            return Some("fileSpec");
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
    if dictionary.get("ShadingType").is_some() {
        return Some("shading");
    }
    if dictionary.get("FunctionType").is_some() {
        return Some("function");
    }
    if dictionary.get("PatternType").is_some() {
        return Some("pattern");
    }
    if dictionary.get("N").is_some() && dictionary.get("Alternate").is_some() {
        return Some("iccProfile");
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
            ModelObjectRef::Operator(model) => model.super_types(),
            ModelObjectRef::MarkedContent(model) => model.super_types(),
            ModelObjectRef::InlineImage(model) => model.super_types(),
            ModelObjectRef::ResourceUse(model) => model.super_types(),
            ModelObjectRef::Accessibility(model) => model.super_types(),
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
        evaluator: &mut DefaultRuleEvaluator<'_>,
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

#[derive(Clone, Copy, Debug)]
enum XmpPrefixProperty {
    Part,
    Conformance,
    Rev,
}

#[derive(Clone, Debug)]
struct XmpClaimView<'a> {
    family: &'a Identifier,
    part: u32,
    conformance: Option<&'a Identifier>,
    part_prefix: &'a Identifier,
    rev: Option<&'a BoundedText>,
    rev_prefix: Option<&'a Identifier>,
    conformance_prefix: Option<&'a Identifier>,
}

fn xmp_identification_claim<'a>(
    document: &'a ParsedDocument,
    expected_family: Option<&str>,
) -> Option<XmpClaimView<'a>> {
    document.parse_facts.iter().find_map(|fact| {
        let crate::ParseFact::Xmp {
            fact:
                crate::XmpFact::FlavourClaim {
                    family,
                    part,
                    conformance,
                    part_prefix,
                    display_flavour: _,
                    rev,
                    rev_prefix,
                    conformance_prefix,
                    ..
                },
            ..
        } = fact
        else {
            return None;
        };
        if family.as_str() == "wtpdf" {
            return None;
        }
        if let Some(expected_family) = expected_family
            && family.as_str() != expected_family
        {
            return None;
        }
        Some(XmpClaimView {
            family,
            part: *part,
            conformance: conformance.as_ref(),
            part_prefix,
            rev: rev.as_ref(),
            rev_prefix: rev_prefix.as_ref(),
            conformance_prefix: conformance_prefix.as_ref(),
        })
    })
}

fn xmp_part(document: &ParsedDocument, family: Option<&str>) -> Option<f64> {
    xmp_identification_claim(document, family).map(|claim| f64::from(claim.part))
}

fn xmp_prefix(
    document: &ParsedDocument,
    family: Option<&str>,
    property: XmpPrefixProperty,
) -> Option<String> {
    xmp_identification_claim(document, family).and_then(|claim| match property {
        XmpPrefixProperty::Part => Some(claim.part_prefix.as_str().to_owned()),
        XmpPrefixProperty::Conformance => claim
            .conformance_prefix
            .map(|prefix| prefix.as_str().to_owned()),
        XmpPrefixProperty::Rev => claim.rev_prefix.map(|prefix| prefix.as_str().to_owned()),
    })
}

fn xmp_conformance(document: &ParsedDocument, family: Option<&str>) -> Option<String> {
    xmp_identification_claim(document, family).and_then(|claim| {
        if claim.family.as_str() == "pdfa" {
            claim
                .conformance
                .map(|conformance| conformance.as_str().to_owned())
        } else {
            None
        }
    })
}

fn xmp_rev(document: &ParsedDocument, family: Option<&str>) -> Option<String> {
    xmp_identification_claim(document, family)
        .and_then(|claim| claim.rev.map(|rev| rev.as_str().to_owned()))
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

fn i64_to_f64(value: i64) -> Result<f64> {
    let bounded = i32::try_from(value).map_err(|_| ValidationError::LimitExceeded {
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
        BinaryOp, BoundedText, ErrorTemplate, FeatureSelection, FlavourSelection, Identifier,
        InputName, ModelObject, ModelObjectRef, ModelValue, ObjectTypeName, Parser, PdfvError,
        ProfileIdentity, ProfileRepository, PropertyName, ResourceLimits, Rule, RuleExpr, RuleId,
        ValidationFlavour, ValidationOptions, ValidationProfile, Validator,
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
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>
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

    #[allow(
        clippy::too_many_lines,
        reason = "single inline PDF fixture keeps object numbers readable for model graph tests"
    )]
    fn m6_model_pdf() -> &'static [u8] {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /Metadata 24 0 R /OutputIntents [26 0 R] /AcroForm 7 0 R /StructTreeRoot 8 0 R /OCProperties 9 0 R /Names 10 0 R /Outlines 11 0 R /Perms 12 0 R /Dests [21 0 R] /Lang (en-US) /MarkInfo << /Marked true >> >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>
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
<< /Type /Annot /Subtype /Widget /FT /Sig /A 17 0 R /AA << /D 27 0 R >> /FS 28 0 R >>
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
<< /Type /Action /S /URI /URI (https://example.invalid) /Next 27 0 R >>
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
24 0 obj
<< /Type /Metadata /Subtype /XML /Length 0 >>
stream
endstream
endobj
25 0 obj
<< /Length 0 >>
stream
endstream
endobj
26 0 obj
<< /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 25 0 R >>
endobj
27 0 obj
<< /Type /Action /S /GoTo /D 21 0 R >>
endobj
28 0 obj
<< /Type /Filespec /F (attachment.txt) >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
    }

    fn content_operator_pdf() -> Vec<u8> {
        let stream =
            b"BT /F1 12 Tf (secret text) Tj ET /Cs1 CS /GS1 gs /Sh1 sh /Im1 Do /Span BMC EMC BI /W 1 /H 1 /BPC 8 /CS /RGB ID x EI WeirdOp";
        let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> /ColorSpace << /Cs1 /DeviceRGB >> /ExtGState << /GS1 << >> >> /Shading << /Sh1 << >> >> /XObject << /Im1 6 0 R >> >> /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
            .to_vec();
        pdf.extend(stream.len().to_string().as_bytes());
        pdf.extend(
            br" >>
stream
",
        );
        pdf.extend(stream);
        pdf.extend(
            br"
endstream
endobj
5 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
6 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
        );
        pdf
    }

    fn phase16_resource_pdf() -> Vec<u8> {
        let stream = b"BT /Inherited 12 Tf ET /Local cs /Missing gs /Bad Do /Shade sh";
        let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /OutputIntents [10 0 R] >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /Inherited 5 0 R >> /Shading << /Shade 11 0 R >> >> >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /ColorSpace << /Local 7 0 R >> /XObject << /Bad 6 0 R >> >> /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
            .to_vec();
        pdf.extend(stream.len().to_string().as_bytes());
        pdf.extend(
            br" >>
stream
",
        );
        pdf.extend(stream);
        pdf.extend(
            br"
endstream
endobj
5 0 obj
<< /Type /Font /Subtype /Type0 /BaseFont /Faux /DescendantFonts [8 0 R] /ToUnicode 12 0 R >>
endobj
6 0 obj
<< /NotAnXObject true >>
endobj
7 0 obj
<< /N 3 /Alternate /DeviceRGB /Range [0 1 0 1 0 1] >>
endobj
8 0 obj
<< /Type /Font /Subtype /CIDFontType2 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 9 0 R >>
endobj
9 0 obj
<< /Type /FontDescriptor /FontName /Faux /FontFile2 13 0 R >>
endobj
10 0 obj
<< /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 14 0 R >>
endobj
11 0 obj
<< /ShadingType 2 /ColorSpace /DeviceRGB /Function 15 0 R >>
endobj
12 0 obj
<< /Type /CMap /CMapName /Identity-H /WMode 0 >>
endobj
13 0 obj
<< /Length 4 /Length1 4 >>
stream
font
endstream
endobj
14 0 obj
<< /N 3 /Length 132 >>
stream
",
        );
        let mut icc = vec![0_u8; 132];
        write_fixture_bytes(&mut icc, 0, &132_u32.to_be_bytes());
        write_fixture_byte(&mut icc, 8, 4);
        write_fixture_byte(&mut icc, 9, 0x30);
        write_fixture_bytes(&mut icc, 12, b"mntr");
        write_fixture_bytes(&mut icc, 16, b"RGB ");
        write_fixture_bytes(&mut icc, 20, b"XYZ ");
        write_fixture_bytes(&mut icc, 64, &1_u32.to_be_bytes());
        pdf.extend(icc);
        pdf.extend(
            br"
endstream
endobj
15 0 obj
<< /FunctionType 2 /Domain [0 1] /Range [0 1] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
        );
        pdf
    }

    fn phase18_accessibility_pdf() -> Vec<u8> {
        let stream = b"/P <</MCID 0>> BDC BT (secret text) Tj ET EMC /Figure <</MCID 1>> BDC /Im1 Do EMC /Artifact BMC EMC";
        let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 8 0 R /Lang (en-US) /MarkInfo << /Marked true >> >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /StructParents 0 /Resources << /XObject << /Im1 5 0 R >> >> /Annots [6 0 R] /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
            .to_vec();
        pdf.extend(stream.len().to_string().as_bytes());
        pdf.extend(
            br" >>
stream
",
        );
        pdf.extend(stream);
        pdf.extend(
            br"
endstream
endobj
5 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
6 0 obj
<< /Type /Annot /Subtype /Link >>
endobj
8 0 obj
<< /Type /StructTreeRoot /K [9 0 R 10 0 R 11 0 R 12 0 R] /RoleMap << /CustomH /H1 >> /ClassMap << /Important << >> >> /IDTree << /Names [(heading) 9 0 R] >> /ParentTree << /Nums [0 [9 0 R 10 0 R]] >> /ParentTreeNextKey 1 >>
endobj
9 0 obj
<< /Type /StructElem /S /CustomH /Pg 3 0 R /K 0 /ID (heading) >>
endobj
10 0 obj
<< /Type /StructElem /S /Figure /Pg 3 0 R /K << /Type /MCR /Pg 3 0 R /MCID 1 >> /Alt (chart alternative text) >>
endobj
11 0 obj
<< /Type /StructElem /S /Link /Pg 3 0 R /K << /Type /OBJR /Obj 6 0 R /Pg 3 0 R >> >>
endobj
12 0 obj
<< /Type /StructElem /S /L /Pg 3 0 R /K [13 0 R] >>
endobj
13 0 obj
<< /Type /StructElem /S /LI /Pg 3 0 R /K [] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
        );
        pdf
    }

    fn write_fixture_bytes(target: &mut [u8], start: usize, bytes: &[u8]) {
        let end = start.saturating_add(bytes.len());
        if let Some(slot) = target.get_mut(start..end) {
            slot.copy_from_slice(bytes);
        }
    }

    fn write_fixture_byte(target: &mut [u8], index: usize, byte: u8) {
        if let Some(slot) = target.get_mut(index) {
            *slot = byte;
        }
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

        let limits = crate::ResourceLimits::default();
        let pages = PageModel::from_catalog(&document, &catalog, &limits, 16)?;
        let page = pages.first().ok_or(crate::ParseError::MissingObject {
            message: crate::BoundedText::unchecked("missing page"),
        })?;
        let fonts = FontModel::from_page(&document, page, 16)?;
        let annotations = AnnotationModel::from_page(&document, page, 16)?;
        let output_intents = OutputIntentModel::from_catalog(&document, &catalog, &limits, 16)?;
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
        assert_eq!(
            page.property(&PropertyName::new("MediaBox")?)?,
            ModelValue::List(vec![
                ModelValue::Number(0.0),
                ModelValue::Number(0.0),
                ModelValue::Number(200.0),
                ModelValue::Number(200.0),
            ])
        );
        Ok(())
    }

    #[test]
    fn test_should_materialize_content_stream_operator_families_lazily() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(content_operator_pdf()))?;
        let limits = ResourceLimits::default();
        let graph = super::ModelGraph::with_all_families(&document, &limits);
        let mut stack = vec![ModelObjectRef::Document(super::DocumentModel::new(
            &document,
        ))];
        let mut operator_count = 0_u64;
        let mut resource_use_count = 0_u64;
        let mut marked_content_count = 0_u64;
        let mut inline_image_count = 0_u64;
        let mut content_streams = 0_u64;
        let mut unknown_operator_count = 0_u64;

        while let Some(object) = stack.pop() {
            match object.object_type().as_str() {
                "contentStream" => {
                    content_streams = content_streams.saturating_add(1);
                    assert_eq!(
                        object.property(&PropertyName::new("hasText")?)?,
                        ModelValue::Bool(true)
                    );
                    assert_eq!(
                        object.property(&PropertyName::new("hasInlineImage")?)?,
                        ModelValue::Bool(true)
                    );
                    assert_eq!(
                        object.property(&PropertyName::new("hasUnknownOperators")?)?,
                        ModelValue::Bool(true)
                    );
                }
                "operator" => {
                    operator_count = operator_count.saturating_add(1);
                    if object.property(&PropertyName::new("isUnknown")?)? == ModelValue::Bool(true)
                    {
                        unknown_operator_count = unknown_operator_count.saturating_add(1);
                        assert_eq!(
                            object.property(&PropertyName::new("name")?)?,
                            ModelValue::String(BoundedText::unchecked("WeirdOp"))
                        );
                    }
                }
                "resourceUse" => resource_use_count = resource_use_count.saturating_add(1),
                "markedContent" => marked_content_count = marked_content_count.saturating_add(1),
                "inlineImage" => inline_image_count = inline_image_count.saturating_add(1),
                _ => {}
            }
            for linked in object.linked_objects(&graph, 128)? {
                stack.push(linked);
            }
        }

        assert_eq!(content_streams, 1);
        assert!(operator_count >= 10);
        assert_eq!(unknown_operator_count, 1);
        assert_eq!(resource_use_count, 5);
        assert_eq!(marked_content_count, 1);
        assert_eq!(inline_image_count, 1);
        Ok(())
    }

    #[test]
    fn test_should_extract_redacted_content_stream_features() -> crate::Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::All)
            .build();
        let validator = Validator::new(options)?;
        let report =
            validator.validate_reader(Cursor::new(content_operator_pdf()), InputName::memory())?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;

        assert!(
            features
                .objects
                .iter()
                .any(|object| object.family.as_str() == "operator")
        );
        assert!(
            features
                .objects
                .iter()
                .any(|object| object.family.as_str() == "resourceUse")
        );
        let raw_text = PropertyName::new("rawText")?;
        assert!(
            features
                .objects
                .iter()
                .filter(|object| object.family.as_str() == "operator")
                .all(|object| !object.properties.contains_key(&raw_text))
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_content_resource_uses_with_effective_resources() -> crate::Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::All)
            .build();
        let validator = Validator::new(options)?;
        let report =
            validator.validate_reader(Cursor::new(phase16_resource_pdf()), InputName::memory())?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;
        let status = PropertyName::new("status")?;
        let name = PropertyName::new("name")?;
        let resolved_family = PropertyName::new("resolvedFamily")?;
        let uses = features
            .objects
            .iter()
            .filter(|object| object.family.as_str() == "resourceUse")
            .collect::<Vec<_>>();

        assert!(uses.iter().any(|object| {
            matches!(
                object.properties.get(&name),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "Inherited"
            ) && matches!(
                object.properties.get(&status),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "resolved"
            )
        }));
        assert!(uses.iter().any(|object| {
            matches!(
                object.properties.get(&name),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "Missing"
            ) && matches!(
                object.properties.get(&status),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "missing"
            )
        }));
        assert!(uses.iter().any(|object| {
            matches!(
                object.properties.get(&name),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "Bad"
            ) && matches!(
                object.properties.get(&status),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "wrongType"
            )
        }));
        assert!(uses.iter().any(|object| {
            matches!(
                object.properties.get(&resolved_family),
                Some(crate::FeatureValue::String(value)) if value.as_str() == "colorSpace"
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_extract_font_color_output_intent_and_function_summaries() -> crate::Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::All)
            .build();
        let validator = Validator::new(options)?;
        let report =
            validator.validate_reader(Cursor::new(phase16_resource_pdf()), InputName::memory())?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;
        let family = PropertyName::new("family")?;
        let has_cid_system_info = PropertyName::new("hasCIDSystemInfo")?;
        let icc_color_space = PropertyName::new("iccColorSpace")?;

        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "font"
                && matches!(
                    object.properties.get(&has_cid_system_info),
                    Some(crate::FeatureValue::Bool(true))
                )
        }));
        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "colorSpace"
                && matches!(
                    object.properties.get(&family),
                    Some(crate::FeatureValue::String(value)) if value.as_str() == "ICCBased"
                )
        }));
        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "outputIntent"
                && matches!(
                    object.properties.get(&icc_color_space),
                    Some(crate::FeatureValue::String(value)) if value.as_str() == "RGB"
                )
        }));
        assert!(
            features
                .objects
                .iter()
                .any(|object| object.family.as_str() == "shading")
        );
        assert!(
            features
                .objects
                .iter()
                .any(|object| object.family.as_str() == "function")
        );
        Ok(())
    }

    #[test]
    fn test_should_extract_accessibility_semantic_families_redacted() -> crate::Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::All)
            .build();
        let validator = Validator::new(options)?;
        let report = validator.validate_reader(
            Cursor::new(phase18_accessibility_pdf()),
            InputName::memory(),
        )?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;
        let families = features
            .objects
            .iter()
            .map(|object| object.family.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for family in [
            "accessibilityDocument",
            "structureElement",
            "textChunk",
            "imageChunk",
            "accessibilityAnnotation",
            "artifact",
            "heading",
            "list",
            "link",
        ] {
            assert!(families.contains(family), "missing {family}: {families:?}");
        }

        let normalized_role = PropertyName::new("normalizedRole")?;
        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "structureElement"
                && matches!(
                    object.properties.get(&normalized_role),
                    Some(crate::FeatureValue::String(value)) if value.as_str() == "H1"
                )
        }));
        let has_alt_text = PropertyName::new("hasAltText")?;
        let alt_text = PropertyName::new("Alt")?;
        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "imageChunk"
                && matches!(
                    object.properties.get(&has_alt_text),
                    Some(crate::FeatureValue::Bool(true))
                )
        }));
        assert!(features.objects.iter().all(|object| {
            !matches!(
                object.properties.get(&alt_text),
                Some(crate::FeatureValue::String(value)) if value.as_str().contains("chart")
            )
        }));
        let raw_text = PropertyName::new("rawText")?;
        assert!(features.objects.iter().all(|object| {
            !matches!(
                object.properties.get(&raw_text),
                Some(crate::FeatureValue::String(value)) if value.as_str().contains("secret")
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_cap_accessibility_graph_without_panicking() -> crate::Result<()> {
        let limits = ResourceLimits {
            max_structure_nodes: 1,
            ..ResourceLimits::default()
        };
        let options = ValidationOptions::builder()
            .resource_limits(limits)
            .feature_selection(FeatureSelection::Families {
                families: vec![ObjectTypeName::new("accessibilityDocument")?],
            })
            .build();
        let validator = Validator::new(options)?;
        let report = validator.validate_reader(
            Cursor::new(phase18_accessibility_pdf()),
            InputName::memory(),
        )?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;
        let truncated = PropertyName::new("truncated")?;

        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "accessibilityDocument"
                && matches!(
                    object.properties.get(&truncated),
                    Some(crate::FeatureValue::Bool(true))
                )
        }));
        Ok(())
    }

    #[test]
    fn test_should_materialize_form_xobject_content_stream_context() -> crate::Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::All)
            .build();
        let validator = Validator::new(options)?;
        let report = validator.validate_reader(Cursor::new(m6_model_pdf()), InputName::memory())?;
        let features =
            report
                .feature_report
                .ok_or(crate::ValidationError::SubsystemUnavailable {
                    subsystem: "featureExtraction",
                })?;

        assert!(features.objects.iter().any(|object| {
            object.family.as_str() == "contentStream"
                && object
                    .context
                    .as_str()
                    .contains("/xObject[Fm1]/contentStream[0]")
        }));
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
    fn test_should_redact_content_strings_from_feature_report() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m6_model_pdf()))?;
        let session =
            super::ValidationSession::new(document, crate::ResourceLimits::default(), 100, false);
        let action_family = crate::ObjectTypeName::new("action")?;
        let report = session.extract_features(&super::FeatureSelection::Families {
            families: vec![action_family.clone()],
        })?;
        let uri_property = PropertyName::new("URI")?;
        let has_redacted_uri = report
            .objects
            .iter()
            .filter(|object| object.family == action_family)
            .any(|action| {
                matches!(
                    action.properties.get(&uri_property),
                    Some(crate::FeatureValue::RedactedString { bytes }) if *bytes > 0
                )
            });
        assert!(has_redacted_uri);
        Ok(())
    }

    #[test]
    fn test_should_truncate_feature_report_on_object_cap() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m6_model_pdf()))?;
        let limits = crate::ResourceLimits {
            max_objects: 1,
            ..crate::ResourceLimits::default()
        };
        let session = super::ValidationSession::new(document, limits, 100, false);
        let report = session.extract_features(&super::FeatureSelection::All)?;

        assert!(report.truncated);
        assert_eq!(report.visited_objects, 1);
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
        let catalog_family = registry
            .families
            .get(&crate::ObjectTypeName::new("catalog")?)
            .ok_or(crate::ProfileError::UnsupportedSelection)?;
        let acro_form_link = crate::LinkName::new("acroForm")?;
        assert!(
            catalog_family
                .link_schema()
                .iter()
                .any(|spec| spec.name == acro_form_link)
        );
        Ok(())
    }

    #[test]
    fn test_should_read_registered_catalog_root_properties() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m6_model_pdf()))?;
        let catalog_key = document.catalog.ok_or(crate::ParseError::MissingObject {
            message: crate::BoundedText::unchecked("missing catalog"),
        })?;
        let catalog =
            CatalogModel::new(&document, catalog_key).ok_or(crate::ParseError::MissingObject {
                message: crate::BoundedText::unchecked("missing catalog model"),
            })?;

        assert_eq!(
            catalog.property(&PropertyName::new("AcroForm")?)?,
            ModelValue::ObjectKey(crate::ObjectKey::new(
                std::num::NonZeroU32::new(7).ok_or(crate::ParseError::MissingObject {
                    message: BoundedText::unchecked("invalid object number"),
                })?,
                0,
            ))
        );
        assert_eq!(
            catalog.property(&PropertyName::new("language")?)?,
            ModelValue::String(BoundedText::unchecked("en-US"))
        );
        assert_eq!(
            catalog.property(&PropertyName::new("Marked")?)?,
            ModelValue::Bool(true)
        );
        assert_eq!(
            catalog.property(&PropertyName::new("permissions")?)?,
            ModelValue::Bool(true)
        );
        assert_eq!(
            catalog.property(&PropertyName::new("OutputIntents")?)?,
            ModelValue::List(vec![ModelValue::ObjectKey(crate::ObjectKey::new(
                std::num::NonZeroU32::new(26).ok_or(crate::ParseError::MissingObject {
                    message: BoundedText::unchecked("invalid object number"),
                })?,
                0,
            ))])
        );
        assert_eq!(
            catalog.property(&PropertyName::new("Dests")?)?,
            ModelValue::List(vec![ModelValue::ObjectKey(crate::ObjectKey::new(
                std::num::NonZeroU32::new(21).ok_or(crate::ParseError::MissingObject {
                    message: BoundedText::unchecked("invalid object number"),
                })?,
                0,
            ))])
        );
        Ok(())
    }

    #[test]
    fn test_should_materialize_every_present_catalog_link() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(m6_model_pdf()))?;
        let catalog_key = document.catalog.ok_or(crate::ParseError::MissingObject {
            message: crate::BoundedText::unchecked("missing catalog"),
        })?;
        let catalog =
            CatalogModel::new(&document, catalog_key).ok_or(crate::ParseError::MissingObject {
                message: crate::BoundedText::unchecked("missing catalog model"),
            })?;
        let limits = crate::ResourceLimits::default();
        let graph = super::ModelGraph::with_all_families(&document, &limits);
        let linked = catalog.linked_objects(&graph, 64)?;
        let families = linked
            .iter()
            .map(ModelObjectRef::object_type)
            .collect::<std::collections::BTreeSet<_>>();

        for (_link, target) in super::CATALOG_LINKS {
            assert!(
                families.contains(&crate::ObjectTypeName::new(*target)?),
                "missing catalog link target {target}: {families:?}"
            );
        }
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
            "fileSpec",
            "signature",
            "security",
        ] {
            assert!(families.contains(family), "missing {family}: {families:?}");
        }
        Ok(())
    }

    #[test]
    fn test_should_report_non_placeholder_model_schema_parity() -> crate::Result<()> {
        let report = crate::model_schema_parity_report()?;

        assert!(report.registered_families > 20);
        assert!(report.registered_properties > 100);
        assert!(report.registered_links > 20);
        assert!(!report.profiles.is_empty());
        assert!(
            report
                .profiles
                .iter()
                .any(|profile| profile.bound_rules > 0)
        );
        assert!(report.profiles.iter().any(|profile| {
            profile
                .unsupported_by_reason
                .contains_key("missingProperty")
                || profile.unsupported_by_reason.contains_key("missingLink")
                || profile
                    .unsupported_by_reason
                    .contains_key("missingObjectType")
        }));
        assert!(report.profiles.iter().any(|profile| {
            profile
                .unsupported_by_reason
                .contains_key("missingSemanticFamily")
        }));
        Ok(())
    }

    #[test]
    fn test_should_bound_cycles_in_page_names_outline_form_and_action_graphs() -> crate::Result<()>
    {
        let document = Parser::default().parse(Cursor::new(cyclic_m6_graph_pdf()))?;
        let limits = crate::ResourceLimits {
            max_objects: 64,
            ..crate::ResourceLimits::default()
        };
        let session = super::ValidationSession::new(document, limits, 100, false);
        let report = session.extract_features(&super::FeatureSelection::All)?;
        let families = report
            .objects
            .iter()
            .map(|object| object.family.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        assert!(!report.truncated);
        assert!(report.visited_objects <= 64);
        for family in [
            "names",
            "outline",
            "destination",
            "formField",
            "action",
            "fileSpec",
        ] {
            assert!(families.contains(family), "missing {family}: {families:?}");
        }
        let destination_property = PropertyName::new("D")?;
        assert!(report.objects.iter().any(|object| {
            object.family.as_str() == "destination"
                && matches!(
                    object.properties.get(&destination_property),
                    Some(crate::FeatureValue::List(values)) if !values.is_empty()
                )
        }));
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

    fn cyclic_m6_graph_pdf() -> &'static [u8] {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /Names 7 0 R /Outlines 8 0 R /AcroForm 11 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 1 /Resources << >> /MediaBox [0 0 100 100] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Annots [4 0 R] /AA 16 0 R >>
endobj
4 0 obj
<< /Type /Annot /Subtype /Widget /FT /Btn /A 5 0 R /AA 16 0 R >>
endobj
5 0 obj
<< /Type /Action /S /URI /URI (https://example.invalid) /Next 5 0 R >>
endobj
6 0 obj
<< /D [3 0 R /Fit] /A 5 0 R >>
endobj
7 0 obj
<< /Dests 13 0 R /EmbeddedFiles << /Names [(f) 15 0 R] >> >>
endobj
8 0 obj
<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 1 >>
endobj
9 0 obj
<< /Title (loop) /Next 9 0 R /Dest 6 0 R >>
endobj
11 0 obj
<< /Fields [12 0 R] >>
endobj
12 0 obj
<< /FT /Tx /Kids [12 0 R] /A 5 0 R /AA 16 0 R >>
endobj
13 0 obj
<< /Kids [13 0 R 14 0 R] >>
endobj
14 0 obj
<< /Names [(home) [3 0 R /Fit]] >>
endobj
15 0 obj
<< /Type /Filespec /F (attachment.txt) >>
endobj
16 0 obj
<< /D 5 0 R >>
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
