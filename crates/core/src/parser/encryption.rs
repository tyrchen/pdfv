//! PDF Standard security handler support for password-based decryption.

use std::sync::Arc;

use aes::{
    Aes128, Aes256,
    cipher::{
        BlockCipherDecrypt, BlockModeDecrypt, BlockModeEncrypt, KeyInit as AesKeyInit, KeyIvInit,
        block_padding::{NoPadding, Pkcs7},
    },
};
use md5::{Digest, Md5};
use rc4::{Rc4, StreamCipher};
use sha2::{Sha256, Sha384, Sha512};
use subtle::ConstantTimeEq;

use super::{
    CosObject, Dictionary, IndirectObject, ObjectKey, ObjectStore, ParseError, PdfName, PdfString,
    StreamObject, Trailer, integer_from_dictionary,
};
use crate::{BoundedText, Identifier, ParseFact, PasswordSecret, ResourceLimits};

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];
const AES_SALT: &[u8] = b"sAlT";
const AES_256_KEY_BYTES: usize = 32;
const AES_BLOCK_BYTES: usize = 16;
const REVISION_5_6_PASSWORD_BYTES: usize = 127;
const REVISION_5_6_HASH_BYTES: usize = 32;
const REVISION_5_6_ENTRY_BYTES: usize = 48;
const REVISION_5_6_SALT_BYTES: usize = 8;
const REVISION_6_HASH_MAX_ROUNDS: u16 = 288;
const PERMS_BYTES: usize = 16;

type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecurityRevision {
    R2,
    R3,
    R4,
    R5,
    R6,
}

impl SecurityRevision {
    fn from_i64(value: i64) -> Result<Self, DecryptionError> {
        match value {
            2 => Ok(Self::R2),
            3 => Ok(Self::R3),
            4 => Ok(Self::R4),
            5 => Ok(Self::R5),
            6 => Ok(Self::R6),
            _ if value > 0 => Err(DecryptionError::Unsupported {
                message: format!("unsupported encryption revision {value}"),
                version: None,
                revision: u8::try_from(value).ok(),
                algorithm: None,
            }),
            _ => Err(DecryptionError::Malformed("invalid security revision")),
        }
    }

    fn value(self) -> u8 {
        match self {
            Self::R2 => 2,
            Self::R3 => 3,
            Self::R4 => 4,
            Self::R5 => 5,
            Self::R6 => 6,
        }
    }

    fn repeats_hash(self) -> bool {
        matches!(self, Self::R3 | Self::R4)
    }

    fn uses_aes_256(self) -> bool {
        matches!(self, Self::R5 | Self::R6)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CryptMethod {
    Identity,
    Rc4,
    AesV2,
    AesV3,
}

impl CryptMethod {
    fn algorithm(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Rc4 => "rc4",
            Self::AesV2 => "aesv2",
            Self::AesV3 => "aesv3",
        }
    }
}

#[derive(Clone, Debug)]
struct EncryptionDictionary {
    object: Option<ObjectKey>,
    version: u8,
    revision: SecurityRevision,
    key_length_bytes: usize,
    owner_key: Vec<u8>,
    user_key: Vec<u8>,
    owner_encryption_key: Vec<u8>,
    user_encryption_key: Vec<u8>,
    perms: Vec<u8>,
    permissions: i32,
    document_id: Vec<u8>,
    encrypt_metadata: bool,
    stream_method: CryptMethod,
    string_method: CryptMethod,
}

/// Safe encryption parse/authentication metadata.
#[derive(Clone, Debug)]
pub(crate) struct EncryptionSummary {
    handler: Option<Identifier>,
    version: Option<u8>,
    revision: Option<u8>,
    algorithm: Option<Identifier>,
}

impl EncryptionSummary {
    /// Builds a report-safe parse fact.
    pub(crate) fn into_fact(self, decrypted: bool) -> ParseFact {
        ParseFact::Encryption {
            encrypted: true,
            handler: self.handler,
            version: self.version,
            revision: self.revision,
            algorithm: self.algorithm,
            decrypted,
        }
    }
}

#[derive(Debug)]
pub(crate) enum DecryptionError {
    IncorrectPassword(EncryptionSummary),
    Unsupported {
        message: String,
        version: Option<u8>,
        revision: Option<u8>,
        algorithm: Option<Identifier>,
    },
    Malformed(&'static str),
    LimitExceeded(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EncryptionClassification {
    Supported,
    Unsupported,
}

impl DecryptionError {
    pub(crate) fn is_encrypted_status(&self) -> bool {
        matches!(self, Self::IncorrectPassword(_) | Self::Unsupported { .. })
    }

    pub(crate) fn safe_message(&self) -> String {
        match self {
            Self::IncorrectPassword(_) => String::from("incorrect password"),
            Self::Unsupported { message, .. } => message.clone(),
            Self::Malformed(message) => String::from(*message),
            Self::LimitExceeded(limit) => format!("resource limit exceeded: {limit}"),
        }
    }

    pub(crate) fn into_fact(self, fallback: ParseFact) -> ParseFact {
        match self {
            Self::IncorrectPassword(summary) => summary.into_fact(false),
            Self::Unsupported {
                version,
                revision,
                algorithm,
                ..
            } => ParseFact::Encryption {
                encrypted: true,
                handler: Some(Identifier::unchecked("Standard")),
                version,
                revision,
                algorithm,
                decrypted: false,
            },
            Self::Malformed(_) | Self::LimitExceeded(_) => fallback,
        }
    }

    pub(crate) fn into_parse_error(self) -> ParseError {
        match self {
            Self::LimitExceeded(limit) => ParseError::LimitExceeded { limit },
            Self::Malformed(message) => ParseError::Malformed {
                message: BoundedText::unchecked(message),
            },
            Self::IncorrectPassword(_) | Self::Unsupported { .. } => ParseError::Malformed {
                message: BoundedText::unchecked(self.safe_message()),
            },
        }
    }
}

/// Decrypts supported encrypted documents in place.
pub(crate) fn decrypt_document(
    objects: &mut ObjectStore,
    trailers: &mut [Trailer],
    limits: &ResourceLimits,
    password: &PasswordSecret,
) -> Result<EncryptionSummary, DecryptionError> {
    let encryption = parse_encryption_dictionary(objects, trailers, limits)?;
    let Some(file_key) = authenticate(&encryption, password.expose_secret_bytes())? else {
        return Err(DecryptionError::IncorrectPassword(encryption.summary()));
    };
    decrypt_objects(objects, limits, &encryption, &file_key)?;
    Ok(encryption.summary())
}

pub(crate) fn encryption_summary(objects: &ObjectStore, trailers: &[Trailer]) -> EncryptionSummary {
    parse_encryption_dictionary(objects, trailers, &ResourceLimits::default()).map_or_else(
        |_| EncryptionSummary::detected(),
        |dictionary| dictionary.summary(),
    )
}

pub(crate) fn classify_encryption(
    objects: &ObjectStore,
    trailers: &[Trailer],
    limits: &ResourceLimits,
) -> Result<EncryptionClassification, DecryptionError> {
    parse_encryption_dictionary(objects, trailers, limits).map_or_else(
        |error| {
            if error.is_encrypted_status() {
                Ok(EncryptionClassification::Unsupported)
            } else {
                Err(error)
            }
        },
        |_| Ok(EncryptionClassification::Supported),
    )
}

impl EncryptionSummary {
    fn detected() -> Self {
        Self {
            handler: Some(Identifier::unchecked("Standard")),
            version: None,
            revision: None,
            algorithm: None,
        }
    }
}

impl EncryptionDictionary {
    fn summary(&self) -> EncryptionSummary {
        EncryptionSummary {
            handler: Some(Identifier::unchecked("Standard")),
            version: Some(self.version),
            revision: Some(self.revision.value()),
            algorithm: Some(Identifier::unchecked(self.string_method.algorithm())),
        }
    }
}

fn parse_encryption_dictionary(
    objects: &ObjectStore,
    trailers: &[Trailer],
    limits: &ResourceLimits,
) -> Result<EncryptionDictionary, DecryptionError> {
    let (object_key, dictionary) = encryption_dictionary(objects, trailers)
        .ok_or(DecryptionError::Malformed("missing encryption dictionary"))?;
    if u64::try_from(dictionary.len())
        .map_err(|_| DecryptionError::Malformed("dictionary too large"))?
        > limits.max_encryption_dict_entries
    {
        return Err(DecryptionError::LimitExceeded(
            "max_encryption_dict_entries",
        ));
    }
    let filter = required_name(dictionary, "Filter")?;
    if !filter.matches("Standard") {
        return Err(DecryptionError::Unsupported {
            message: format!(
                "unsupported security handler {}",
                String::from_utf8_lossy(filter.as_bytes())
            ),
            version: None,
            revision: None,
            algorithm: None,
        });
    }
    let revision = SecurityRevision::from_i64(required_integer(dictionary, "R")?)?;
    let version = required_integer(dictionary, "V")?;
    let version_u8 = u8::try_from(version).map_err(|_| DecryptionError::Unsupported {
        message: format!("unsupported encryption version {version}"),
        version: None,
        revision: Some(revision.value()),
        algorithm: None,
    })?;
    if !matches!(version_u8, 1 | 2 | 4 | 5) {
        return Err(DecryptionError::Unsupported {
            message: format!("unsupported encryption version {version}"),
            version: Some(version_u8),
            revision: Some(revision.value()),
            algorithm: None,
        });
    }
    let key_length_bits = integer_from_dictionary(dictionary, "Length").unwrap_or(40);
    let key_length_bytes = key_length_bytes(revision, key_length_bits)?;
    let owner_key = required_string(dictionary, "O")?.as_bytes().to_vec();
    let user_key = required_string(dictionary, "U")?.as_bytes().to_vec();
    validate_password_entries(revision, &owner_key, &user_key)?;
    let owner_encryption_key = optional_string_bytes(dictionary, "OE")?;
    let user_encryption_key = optional_string_bytes(dictionary, "UE")?;
    let perms = optional_string_bytes(dictionary, "Perms")?;
    validate_aes_256_entries(
        revision,
        version_u8,
        key_length_bytes,
        &owner_encryption_key,
        &user_encryption_key,
        &perms,
    )?;
    let permissions = i32::try_from(required_integer(dictionary, "P")?)
        .map_err(|_| DecryptionError::Malformed("invalid permissions bits"))?;
    let document_id = document_id(trailers)?;
    let encrypt_metadata = match dictionary.get("EncryptMetadata") {
        Some(CosObject::Boolean(value)) => *value,
        _ => true,
    };
    validate_aes_256_crypt_filter_shape(revision, dictionary)?;
    let (stream_method, string_method) = crypt_methods(dictionary, version_u8)?;
    validate_aes_256_methods(revision, stream_method, string_method)?;
    Ok(EncryptionDictionary {
        object: object_key,
        version: version_u8,
        revision,
        key_length_bytes,
        owner_key,
        user_key,
        owner_encryption_key,
        user_encryption_key,
        perms,
        permissions,
        document_id,
        encrypt_metadata,
        stream_method,
        string_method,
    })
}

fn encryption_dictionary<'a>(
    objects: &'a ObjectStore,
    trailers: &'a [Trailer],
) -> Option<(Option<ObjectKey>, &'a Dictionary)> {
    let encrypt = trailers
        .iter()
        .rev()
        .find_map(|trailer| trailer.dictionary.get("Encrypt"))?;
    match encrypt {
        CosObject::Dictionary(dictionary) => Some((None, dictionary)),
        CosObject::Reference(key) => objects
            .get(key)
            .and_then(|object| object.object.as_dictionary())
            .map(|dictionary| (Some(*key), dictionary)),
        _ => None,
    }
}

fn crypt_methods(
    dictionary: &Dictionary,
    version: u8,
) -> Result<(CryptMethod, CryptMethod), DecryptionError> {
    if !matches!(version, 4 | 5) {
        return Ok((CryptMethod::Rc4, CryptMethod::Rc4));
    }
    let stream_filter =
        optional_name(dictionary, "StmF").unwrap_or_else(|| PdfName::from_static("Identity"));
    let string_filter =
        optional_name(dictionary, "StrF").unwrap_or_else(|| PdfName::from_static("Identity"));
    let stream_method = crypt_method(dictionary, &stream_filter)?;
    let string_method = crypt_method(dictionary, &string_filter)?;
    Ok((stream_method, string_method))
}

fn crypt_method(
    dictionary: &Dictionary,
    filter_name: &PdfName,
) -> Result<CryptMethod, DecryptionError> {
    if filter_name.matches("Identity") {
        return Ok(CryptMethod::Identity);
    }
    let Some(CosObject::Dictionary(filters)) = dictionary.get("CF") else {
        return Err(DecryptionError::Malformed(
            "missing crypt filter dictionary",
        ));
    };
    let Some(CosObject::Dictionary(filter)) =
        filters.get(&String::from_utf8_lossy(filter_name.as_bytes()))
    else {
        return Err(DecryptionError::Malformed("missing named crypt filter"));
    };
    let cfm = optional_name(filter, "CFM").unwrap_or_else(|| PdfName::from_static("None"));
    if cfm.matches("None") {
        Ok(CryptMethod::Identity)
    } else if cfm.matches("V2") {
        Ok(CryptMethod::Rc4)
    } else if cfm.matches("AESV2") {
        Ok(CryptMethod::AesV2)
    } else if cfm.matches("AESV3") {
        Ok(CryptMethod::AesV3)
    } else {
        Err(DecryptionError::Unsupported {
            message: format!(
                "unsupported crypt filter method {}",
                String::from_utf8_lossy(cfm.as_bytes())
            ),
            version: Some(4),
            revision: Some(4),
            algorithm: Identifier::new(String::from_utf8_lossy(cfm.as_bytes()).into_owned()).ok(),
        })
    }
}

fn key_length_bytes(revision: SecurityRevision, bits: i64) -> Result<usize, DecryptionError> {
    if revision.uses_aes_256() {
        if bits != 256 {
            return Err(DecryptionError::Malformed(
                "revision 5 or 6 key length must be 256 bits",
            ));
        }
        return Ok(AES_256_KEY_BYTES);
    }
    if revision == SecurityRevision::R2 {
        if bits != 40 {
            return Err(DecryptionError::Malformed(
                "revision 2 key length must be 40 bits",
            ));
        }
        return Ok(5);
    }
    if !(40..=128).contains(&bits) || bits % 8 != 0 {
        return Err(DecryptionError::Malformed("invalid key length"));
    }
    usize::try_from(bits / 8).map_err(|_| DecryptionError::Malformed("invalid key length"))
}

fn authenticate(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    if dictionary.revision.uses_aes_256() {
        return authenticate_aes_256(dictionary, password);
    }
    if let Some(file_key) = authenticate_owner_password(dictionary, password)? {
        return Ok(Some(file_key));
    }
    authenticate_user_password(dictionary, password)
}

fn authenticate_owner_password(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    let owner_key = owner_password_key(dictionary, password)?;
    let padded_user_password = decrypt_owner_key(dictionary, &owner_key)?;
    authenticate_user_password(dictionary, &padded_user_password)
}

fn authenticate_user_password(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    let file_key = derive_file_key(dictionary, password)?;
    let candidate = user_password_value(dictionary, &file_key)?;
    let authenticated: bool = match dictionary.revision {
        SecurityRevision::R2 => bool::from(candidate.ct_eq(dictionary.user_key.as_slice())),
        SecurityRevision::R3 | SecurityRevision::R4 => {
            let Some(expected) = dictionary.user_key.get(..16) else {
                return Err(DecryptionError::Malformed("user key too short"));
            };
            let Some(actual) = candidate.get(..16) else {
                return Err(DecryptionError::Malformed("user key candidate too short"));
            };
            bool::from(actual.ct_eq(expected))
        }
        SecurityRevision::R5 | SecurityRevision::R6 => {
            return Err(DecryptionError::Malformed(
                "invalid revision for legacy user password authentication",
            ));
        }
    };
    Ok(authenticated.then_some(file_key))
}

fn owner_password_key(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let padded = padded_password(password);
    let mut digest = Md5::digest(padded).to_vec();
    if dictionary.revision.repeats_hash() {
        for _ in 0..50 {
            digest = Md5::digest(hash_prefix(&digest, dictionary.key_length_bytes)?).to_vec();
        }
    }
    digest.truncate(dictionary.key_length_bytes);
    Ok(digest)
}

fn decrypt_owner_key(
    dictionary: &EncryptionDictionary,
    owner_key: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let mut current = dictionary.owner_key.clone();
    match dictionary.revision {
        SecurityRevision::R2 => rc4_crypt(owner_key, &current),
        SecurityRevision::R3 | SecurityRevision::R4 => {
            for round in (0_u8..=19).rev() {
                let round_key = xor_key(owner_key, round);
                current = rc4_crypt(&round_key, &current)?;
            }
            Ok(current)
        }
        SecurityRevision::R5 | SecurityRevision::R6 => Err(DecryptionError::Malformed(
            "invalid revision for legacy owner password authentication",
        )),
    }
}

fn derive_file_key(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let padded = padded_password(password);
    let mut hasher = Md5::new();
    hasher.update(padded);
    hasher.update(&dictionary.owner_key);
    hasher.update(dictionary.permissions.to_le_bytes());
    hasher.update(&dictionary.document_id);
    if dictionary.revision == SecurityRevision::R4 && !dictionary.encrypt_metadata {
        hasher.update([0xff, 0xff, 0xff, 0xff]);
    }
    let mut digest = hasher.finalize().to_vec();
    if dictionary.revision.repeats_hash() {
        for _ in 0..50 {
            digest = Md5::digest(hash_prefix(&digest, dictionary.key_length_bytes)?).to_vec();
        }
    }
    digest.truncate(dictionary.key_length_bytes);
    Ok(digest)
}

fn user_password_value(
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    match dictionary.revision {
        SecurityRevision::R2 => rc4_crypt(file_key, &PASSWORD_PADDING),
        SecurityRevision::R3 | SecurityRevision::R4 => {
            let mut hasher = Md5::new();
            hasher.update(PASSWORD_PADDING);
            hasher.update(&dictionary.document_id);
            let mut current = hasher.finalize().to_vec();
            for round in 0_u8..=19 {
                let round_key = xor_key(file_key, round);
                current = rc4_crypt(&round_key, &current)?;
            }
            current.extend_from_slice(&[0_u8; 16]);
            Ok(current)
        }
        SecurityRevision::R5 | SecurityRevision::R6 => Err(DecryptionError::Malformed(
            "invalid revision for legacy user password value",
        )),
    }
}

fn authenticate_aes_256(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    if let Some(file_key) = authenticate_aes_256_owner_password(dictionary, password)? {
        return Ok(Some(file_key));
    }
    authenticate_aes_256_user_password(dictionary, password)
}

fn authenticate_aes_256_owner_password(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    let password = revision_5_6_password(password);
    let owner_validation_salt = slice_range(
        &dictionary.owner_key,
        REVISION_5_6_HASH_BYTES,
        REVISION_5_6_SALT_BYTES,
        "owner validation salt missing",
    )?;
    let expected = hash_prefix(&dictionary.owner_key, REVISION_5_6_HASH_BYTES)?;
    let candidate = revision_5_6_hash(
        dictionary.revision,
        password,
        owner_validation_salt,
        Some(&dictionary.user_key),
    )?;
    if !bool::from(candidate.as_slice().ct_eq(expected)) {
        return Ok(None);
    }
    let owner_key_salt = slice_range(
        &dictionary.owner_key,
        REVISION_5_6_HASH_BYTES + REVISION_5_6_SALT_BYTES,
        REVISION_5_6_SALT_BYTES,
        "owner key salt missing",
    )?;
    let key = revision_5_6_hash(
        dictionary.revision,
        password,
        owner_key_salt,
        Some(&dictionary.user_key),
    )?;
    let file_key = decrypt_aes_256_cbc_no_padding(&key, &dictionary.owner_encryption_key)?;
    validate_perms(dictionary, &file_key)?;
    Ok(Some(file_key))
}

fn authenticate_aes_256_user_password(
    dictionary: &EncryptionDictionary,
    password: &[u8],
) -> Result<Option<Vec<u8>>, DecryptionError> {
    let password = revision_5_6_password(password);
    let user_validation_salt = slice_range(
        &dictionary.user_key,
        REVISION_5_6_HASH_BYTES,
        REVISION_5_6_SALT_BYTES,
        "user validation salt missing",
    )?;
    let expected = hash_prefix(&dictionary.user_key, REVISION_5_6_HASH_BYTES)?;
    let candidate = revision_5_6_hash(dictionary.revision, password, user_validation_salt, None)?;
    if !bool::from(candidate.as_slice().ct_eq(expected)) {
        return Ok(None);
    }
    let user_key_salt = slice_range(
        &dictionary.user_key,
        REVISION_5_6_HASH_BYTES + REVISION_5_6_SALT_BYTES,
        REVISION_5_6_SALT_BYTES,
        "user key salt missing",
    )?;
    let key = revision_5_6_hash(dictionary.revision, password, user_key_salt, None)?;
    let file_key = decrypt_aes_256_cbc_no_padding(&key, &dictionary.user_encryption_key)?;
    validate_perms(dictionary, &file_key)?;
    Ok(Some(file_key))
}

fn revision_5_6_password(password: &[u8]) -> &[u8] {
    password
        .get(..password.len().min(REVISION_5_6_PASSWORD_BYTES))
        .unwrap_or(password)
}

fn revision_5_6_hash(
    revision: SecurityRevision,
    password: &[u8],
    salt: &[u8],
    owner_context: Option<&[u8]>,
) -> Result<Vec<u8>, DecryptionError> {
    let mut hasher = Sha256::new();
    hasher.update(password);
    hasher.update(salt);
    if let Some(context) = owner_context {
        hasher.update(context);
    }
    let mut digest = hasher.finalize().to_vec();
    if revision == SecurityRevision::R6 {
        digest = revision_6_hash_loop(password, owner_context, digest)?;
    }
    digest.truncate(AES_256_KEY_BYTES);
    Ok(digest)
}

fn revision_6_hash_loop(
    password: &[u8],
    owner_context: Option<&[u8]>,
    mut digest: Vec<u8>,
) -> Result<Vec<u8>, DecryptionError> {
    let mut round = 0_u16;
    loop {
        if round >= REVISION_6_HASH_MAX_ROUNDS {
            return Err(DecryptionError::Malformed(
                "revision 6 hash exceeded iteration bound",
            ));
        }
        let context_len = owner_context.map_or(0, <[u8]>::len);
        let mut k1 = Vec::with_capacity(password.len() + digest.len() + context_len);
        k1.extend_from_slice(password);
        k1.extend_from_slice(&digest);
        if let Some(context) = owner_context {
            k1.extend_from_slice(context);
        }
        let mut repeated = Vec::with_capacity(k1.len().saturating_mul(64));
        for _ in 0..64 {
            repeated.extend_from_slice(&k1);
        }
        let key = hash_prefix(&digest, 16)?;
        let iv = slice_range(&digest, 16, 16, "revision 6 hash IV missing")?;
        let encrypted = encrypt_aes_128_cbc_no_padding(key, iv, &repeated)?;
        let selector = encrypted
            .get(..16)
            .ok_or(DecryptionError::Malformed("revision 6 selector missing"))?
            .iter()
            .fold(0_u16, |sum, byte| sum + u16::from(*byte))
            % 3;
        digest = match selector {
            0 => Sha256::digest(&encrypted).to_vec(),
            1 => Sha384::digest(&encrypted).to_vec(),
            _ => Sha512::digest(&encrypted).to_vec(),
        };
        let Some(last) = encrypted.last().copied() else {
            return Err(DecryptionError::Malformed("revision 6 hash block missing"));
        };
        if round >= 63 && u16::from(last) <= round.saturating_sub(32) {
            break;
        }
        round = round.saturating_add(1);
    }
    Ok(digest)
}

fn validate_perms(
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<(), DecryptionError> {
    let plaintext = decrypt_aes_256_block(file_key, &dictionary.perms)?;
    let expected_permissions = dictionary.permissions.to_le_bytes();
    let permissions = plaintext
        .get(..4)
        .ok_or(DecryptionError::Malformed("permissions prefix missing"))?;
    if !bool::from(permissions.ct_eq(&expected_permissions)) {
        return Err(tampered_perms(dictionary));
    }
    let Some(stable_suffix) = plaintext.get(4..12) else {
        return Err(DecryptionError::Malformed("permissions suffix missing"));
    };
    let metadata_marker = if dictionary.encrypt_metadata {
        b'T'
    } else {
        b'F'
    };
    let expected_suffix = [0xff, 0xff, 0xff, 0xff, metadata_marker, b'a', b'd', b'b'];
    if !bool::from(stable_suffix.ct_eq(expected_suffix.as_slice())) {
        return Err(tampered_perms(dictionary));
    }
    Ok(())
}

fn tampered_perms(dictionary: &EncryptionDictionary) -> DecryptionError {
    DecryptionError::Unsupported {
        message: String::from("invalid encryption permissions"),
        version: Some(dictionary.version),
        revision: Some(dictionary.revision.value()),
        algorithm: Some(Identifier::unchecked("aesv3")),
    }
}

fn decrypt_objects(
    objects: &mut ObjectStore,
    limits: &ResourceLimits,
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<(), DecryptionError> {
    for object in objects.values_mut() {
        if Some(object.key) == dictionary.object {
            continue;
        }
        decrypt_indirect_object(object, limits, dictionary, file_key)?;
    }
    Ok(())
}

fn decrypt_indirect_object(
    object: &mut IndirectObject,
    limits: &ResourceLimits,
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<(), DecryptionError> {
    decrypt_object(&mut object.object, object.key, limits, dictionary, file_key)
}

fn decrypt_object(
    object: &mut CosObject,
    key: ObjectKey,
    limits: &ResourceLimits,
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<(), DecryptionError> {
    match object {
        CosObject::String(string) => {
            if dictionary.string_method != CryptMethod::Identity {
                let object_key = object_key(file_key, key, dictionary.string_method);
                let decrypted =
                    decrypt_bytes(dictionary.string_method, &object_key, string.as_bytes())?;
                if decrypted.len() > limits.max_decrypted_string_bytes {
                    return Err(DecryptionError::LimitExceeded("max_decrypted_string_bytes"));
                }
                *string = PdfString::new(decrypted, limits)
                    .map_err(|error| parse_to_decryption(&error))?;
            }
        }
        CosObject::Array(values) => {
            for value in values {
                decrypt_object(value, key, limits, dictionary, file_key)?;
            }
        }
        CosObject::Dictionary(values) => {
            for value in values.values_mut() {
                decrypt_object(value, key, limits, dictionary, file_key)?;
            }
        }
        CosObject::Stream(stream) => {
            decrypt_stream(stream, key, limits, dictionary, file_key)?;
            for value in stream.dictionary.values_mut() {
                decrypt_object(value, key, limits, dictionary, file_key)?;
            }
        }
        CosObject::Null
        | CosObject::Boolean(_)
        | CosObject::Integer(_)
        | CosObject::Real(_)
        | CosObject::Name(_)
        | CosObject::Reference(_) => {}
    }
    Ok(())
}

fn decrypt_stream(
    stream: &mut StreamObject,
    key: ObjectKey,
    limits: &ResourceLimits,
    dictionary: &EncryptionDictionary,
    file_key: &[u8],
) -> Result<(), DecryptionError> {
    if !dictionary.encrypt_metadata
        && matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("Metadata"))
    {
        return Ok(());
    }
    if dictionary.stream_method == CryptMethod::Identity {
        return Ok(());
    }
    let raw = stream
        .raw_bytes()
        .map_err(|error| parse_to_decryption(&error))?;
    let raw_len = u64::try_from(raw.len())
        .map_err(|_| DecryptionError::LimitExceeded("max_decrypted_stream_bytes"))?;
    if raw_len > limits.max_stream_declared_bytes {
        return Err(DecryptionError::LimitExceeded("max_stream_declared_bytes"));
    }
    let object_key = object_key(file_key, key, dictionary.stream_method);
    let decrypted = decrypt_bytes(dictionary.stream_method, &object_key, raw)?;
    let decrypted_len = u64::try_from(decrypted.len())
        .map_err(|_| DecryptionError::LimitExceeded("max_decrypted_stream_bytes"))?;
    if decrypted_len > limits.max_decrypted_stream_bytes {
        return Err(DecryptionError::LimitExceeded("max_decrypted_stream_bytes"));
    }
    stream.raw_source = Arc::from(decrypted);
    stream.raw_range.start = 0;
    stream.raw_range.end = decrypted_len;
    stream.discovered_length = decrypted_len;
    stream.declared_length = Some(decrypted_len);
    Ok(())
}

fn object_key(file_key: &[u8], key: ObjectKey, method: CryptMethod) -> Vec<u8> {
    if method == CryptMethod::AesV3 {
        return file_key.to_vec();
    }
    let object_number = key.number.get().to_le_bytes();
    let generation = key.generation.to_le_bytes();
    let mut hasher = Md5::new();
    hasher.update(file_key);
    hasher.update(object_number.get(..3).unwrap_or(&object_number));
    hasher.update(generation.get(..2).unwrap_or(&generation));
    if method == CryptMethod::AesV2 {
        hasher.update(AES_SALT);
    }
    let mut digest = hasher.finalize().to_vec();
    digest.truncate(file_key.len().saturating_add(5).min(16));
    digest
}

fn decrypt_bytes(
    method: CryptMethod,
    key: &[u8],
    bytes: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    match method {
        CryptMethod::Identity => Ok(bytes.to_vec()),
        CryptMethod::Rc4 => rc4_crypt(key, bytes),
        CryptMethod::AesV2 => decrypt_aes_v2(key, bytes),
        CryptMethod::AesV3 => decrypt_aes_v3(key, bytes),
    }
}

fn rc4_crypt(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, DecryptionError> {
    let mut output = bytes.to_vec();
    let mut cipher =
        Rc4::new_from_slice(key).map_err(|_| DecryptionError::Malformed("invalid rc4 key"))?;
    cipher.apply_keystream(&mut output);
    Ok(output)
}

fn decrypt_aes_v2(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, DecryptionError> {
    let iv = bytes
        .get(..16)
        .ok_or(DecryptionError::Malformed("AESV2 stream missing IV"))?;
    let ciphertext = bytes
        .get(16..)
        .ok_or(DecryptionError::Malformed("AESV2 ciphertext missing"))?;
    let mut buffer = ciphertext.to_vec();
    let decrypted = Aes128CbcDec::new_from_slices(key, iv)
        .map_err(|_| DecryptionError::Malformed("invalid AESV2 key or IV"))?
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|_| DecryptionError::Malformed("invalid AESV2 padding"))?;
    Ok(decrypted.to_vec())
}

fn decrypt_aes_v3(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, DecryptionError> {
    let iv = bytes
        .get(..AES_BLOCK_BYTES)
        .ok_or(DecryptionError::Malformed("AESV3 stream missing IV"))?;
    let ciphertext = bytes
        .get(AES_BLOCK_BYTES..)
        .ok_or(DecryptionError::Malformed("AESV3 ciphertext missing"))?;
    let mut buffer = ciphertext.to_vec();
    let decrypted = Aes256CbcDec::new_from_slices(key, iv)
        .map_err(|_| DecryptionError::Malformed("invalid AESV3 key or IV"))?
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|_| DecryptionError::Malformed("invalid AESV3 padding"))?;
    Ok(decrypted.to_vec())
}

fn decrypt_aes_256_cbc_no_padding(
    key: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let mut buffer = ciphertext.to_vec();
    let decrypted = Aes256CbcDec::new_from_slices(key, &[0_u8; AES_BLOCK_BYTES])
        .map_err(|_| DecryptionError::Malformed("invalid AES-256 key or IV"))?
        .decrypt_padded::<NoPadding>(&mut buffer)
        .map_err(|_| DecryptionError::Malformed("invalid AES-256 ciphertext"))?;
    Ok(decrypted.to_vec())
}

fn encrypt_aes_128_cbc_no_padding(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let mut buffer = plaintext.to_vec();
    cbc::Encryptor::<Aes128>::new_from_slices(key, iv)
        .map_err(|_| DecryptionError::Malformed("invalid AES-128 key or IV"))?
        .encrypt_padded::<NoPadding>(&mut buffer, plaintext.len())
        .map_err(|_| DecryptionError::Malformed("invalid AES-128 plaintext"))?;
    Ok(buffer)
}

fn decrypt_aes_256_block(
    key: &[u8],
    ciphertext: &[u8],
) -> Result<[u8; PERMS_BYTES], DecryptionError> {
    let block = fixed_bytes::<PERMS_BYTES>(ciphertext, "invalid permissions length")?;
    let cipher = Aes256::new_from_slice(key)
        .map_err(|_| DecryptionError::Malformed("invalid AES-256 permissions key"))?;
    let mut output = aes::Block::from(block);
    cipher.decrypt_block(&mut output);
    Ok(output.into())
}

fn validate_password_entries(
    revision: SecurityRevision,
    owner_key: &[u8],
    user_key: &[u8],
) -> Result<(), DecryptionError> {
    if !revision.uses_aes_256() {
        return Ok(());
    }
    if owner_key.len() != REVISION_5_6_ENTRY_BYTES {
        return Err(DecryptionError::Malformed("invalid owner key length"));
    }
    if user_key.len() != REVISION_5_6_ENTRY_BYTES {
        return Err(DecryptionError::Malformed("invalid user key length"));
    }
    Ok(())
}

fn validate_aes_256_entries(
    revision: SecurityRevision,
    version: u8,
    key_length_bytes: usize,
    owner_encryption_key: &[u8],
    user_encryption_key: &[u8],
    perms: &[u8],
) -> Result<(), DecryptionError> {
    if !revision.uses_aes_256() {
        return Ok(());
    }
    if version != 5 {
        return Err(DecryptionError::Unsupported {
            message: format!("unsupported encryption version {version}"),
            version: Some(version),
            revision: Some(revision.value()),
            algorithm: Some(Identifier::unchecked("aesv3")),
        });
    }
    if key_length_bytes != AES_256_KEY_BYTES {
        return Err(DecryptionError::Malformed("invalid AES-256 key length"));
    }
    if owner_encryption_key.len() != AES_256_KEY_BYTES {
        return Err(DecryptionError::Malformed(
            "invalid owner encryption key length",
        ));
    }
    if user_encryption_key.len() != AES_256_KEY_BYTES {
        return Err(DecryptionError::Malformed(
            "invalid user encryption key length",
        ));
    }
    if perms.len() != PERMS_BYTES {
        return Err(DecryptionError::Malformed("invalid permissions length"));
    }
    Ok(())
}

fn validate_aes_256_methods(
    revision: SecurityRevision,
    stream_method: CryptMethod,
    string_method: CryptMethod,
) -> Result<(), DecryptionError> {
    if !revision.uses_aes_256() {
        return Ok(());
    }
    if matches!(stream_method, CryptMethod::Identity | CryptMethod::AesV3)
        && matches!(string_method, CryptMethod::Identity | CryptMethod::AesV3)
    {
        return Ok(());
    }
    Err(DecryptionError::Unsupported {
        message: String::from("unsupported AES-256 crypt filter method"),
        version: Some(5),
        revision: Some(revision.value()),
        algorithm: Some(Identifier::unchecked("aesv3")),
    })
}

fn validate_aes_256_crypt_filter_shape(
    revision: SecurityRevision,
    dictionary: &Dictionary,
) -> Result<(), DecryptionError> {
    if !revision.uses_aes_256() {
        return Ok(());
    }
    let stream_filter =
        optional_name(dictionary, "StmF").unwrap_or_else(|| PdfName::from_static("Identity"));
    let string_filter =
        optional_name(dictionary, "StrF").unwrap_or_else(|| PdfName::from_static("Identity"));
    for filter_name in [&stream_filter, &string_filter] {
        if filter_name.matches("Identity") {
            continue;
        }
        if !filter_name.matches("StdCF") {
            return Err(DecryptionError::Unsupported {
                message: format!(
                    "unsupported AES-256 crypt filter {}",
                    String::from_utf8_lossy(filter_name.as_bytes())
                ),
                version: Some(5),
                revision: Some(revision.value()),
                algorithm: Some(Identifier::unchecked("aesv3")),
            });
        }
        let filter = named_crypt_filter(dictionary, filter_name)?;
        let cfm = optional_name(filter, "CFM").unwrap_or_else(|| PdfName::from_static("None"));
        if !cfm.matches("AESV3") {
            return Err(DecryptionError::Unsupported {
                message: String::from("unsupported AES-256 crypt filter method"),
                version: Some(5),
                revision: Some(revision.value()),
                algorithm: Identifier::new(String::from_utf8_lossy(cfm.as_bytes()).into_owned())
                    .ok(),
            });
        }
        if integer_from_dictionary(filter, "Length") != Some(32) {
            return Err(DecryptionError::Malformed(
                "invalid AES-256 crypt filter length",
            ));
        }
        match optional_name(filter, "AuthEvent") {
            Some(auth_event) if auth_event.matches("DocOpen") => {}
            _ => {
                return Err(DecryptionError::Malformed(
                    "invalid AES-256 crypt filter auth event",
                ));
            }
        }
    }
    Ok(())
}

fn named_crypt_filter<'a>(
    dictionary: &'a Dictionary,
    filter_name: &PdfName,
) -> Result<&'a Dictionary, DecryptionError> {
    let Some(CosObject::Dictionary(filters)) = dictionary.get("CF") else {
        return Err(DecryptionError::Malformed(
            "missing crypt filter dictionary",
        ));
    };
    let Some(CosObject::Dictionary(filter)) =
        filters.get(&String::from_utf8_lossy(filter_name.as_bytes()))
    else {
        return Err(DecryptionError::Malformed("missing named crypt filter"));
    };
    Ok(filter)
}

fn padded_password(password: &[u8]) -> [u8; 32] {
    let mut padded = PASSWORD_PADDING;
    let copy_len = password.len().min(32);
    if let (Some(target), Some(source)) = (padded.get_mut(..copy_len), password.get(..copy_len)) {
        target.copy_from_slice(source);
    }
    padded
}

fn xor_key(key: &[u8], round: u8) -> Vec<u8> {
    key.iter().map(|byte| byte ^ round).collect()
}

fn document_id(trailers: &[Trailer]) -> Result<Vec<u8>, DecryptionError> {
    let Some(CosObject::Array(values)) = trailers
        .iter()
        .rev()
        .find_map(|trailer| trailer.dictionary.get("ID"))
    else {
        return Err(DecryptionError::Malformed("missing document ID"));
    };
    let Some(CosObject::String(id)) = values.first() else {
        return Err(DecryptionError::Malformed("missing first document ID"));
    };
    Ok(id.as_bytes().to_vec())
}

fn required_integer(dictionary: &Dictionary, key: &'static str) -> Result<i64, DecryptionError> {
    integer_from_dictionary(dictionary, key)
        .ok_or(DecryptionError::Malformed("missing integer encryption key"))
}

fn required_name<'a>(
    dictionary: &'a Dictionary,
    key: &'static str,
) -> Result<&'a PdfName, DecryptionError> {
    match dictionary.get(key) {
        Some(CosObject::Name(name)) => Ok(name),
        _ => Err(DecryptionError::Malformed("missing name encryption key")),
    }
}

fn optional_name(dictionary: &Dictionary, key: &'static str) -> Option<PdfName> {
    match dictionary.get(key) {
        Some(CosObject::Name(name)) => Some(name.clone()),
        _ => None,
    }
}

fn required_string<'a>(
    dictionary: &'a Dictionary,
    key: &'static str,
) -> Result<&'a PdfString, DecryptionError> {
    match dictionary.get(key) {
        Some(CosObject::String(string)) => Ok(string),
        _ => Err(DecryptionError::Malformed("missing string encryption key")),
    }
}

fn optional_string_bytes(
    dictionary: &Dictionary,
    key: &'static str,
) -> Result<Vec<u8>, DecryptionError> {
    match dictionary.get(key) {
        Some(CosObject::String(string)) => Ok(string.as_bytes().to_vec()),
        None => Ok(Vec::new()),
        _ => Err(DecryptionError::Malformed("invalid string encryption key")),
    }
}

fn hash_prefix(bytes: &[u8], len: usize) -> Result<&[u8], DecryptionError> {
    bytes
        .get(..len)
        .ok_or(DecryptionError::Malformed("hash prefix out of bounds"))
}

fn slice_range<'a>(
    bytes: &'a [u8],
    offset: usize,
    len: usize,
    message: &'static str,
) -> Result<&'a [u8], DecryptionError> {
    let end = offset
        .checked_add(len)
        .ok_or(DecryptionError::Malformed(message))?;
    bytes
        .get(offset..end)
        .ok_or(DecryptionError::Malformed(message))
}

fn fixed_bytes<const N: usize>(
    bytes: &[u8],
    message: &'static str,
) -> Result<[u8; N], DecryptionError> {
    bytes
        .try_into()
        .map_err(|_| DecryptionError::Malformed(message))
}

fn parse_to_decryption(error: &ParseError) -> DecryptionError {
    match error {
        ParseError::LimitExceeded { limit } => DecryptionError::LimitExceeded(limit),
        _ => DecryptionError::Malformed("decryption produced invalid object"),
    }
}
