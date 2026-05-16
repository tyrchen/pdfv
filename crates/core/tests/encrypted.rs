#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "encrypted fixture tests build deterministic byte fixtures synchronously"
)]

use std::{error::Error, io::Cursor, num::NonZeroU32};

use aes::{
    Aes128,
    cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7},
};
use md5::{Digest, Md5};
use pdfv_core::{
    CosObject, InputName, ObjectKey, ParseFact, ParseOptions, Parser, PasswordSecret,
    ResourceLimits, ValidationOptions, ValidationStatus, Validator,
};
use rc4::{KeyInit, Rc4, StreamCipher};

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];
const DOCUMENT_ID: &[u8] = b"pdfv-phase9-doc1";
const USER_PASSWORD: &[u8] = b"user";
const OWNER_PASSWORD: &[u8] = b"owner";

type Aes128CbcEnc = cbc::Encryptor<Aes128>;

#[test]
fn test_should_validate_rc4_encrypted_fixture_with_user_password() -> Result<(), Box<dyn Error>> {
    let fixture = encrypted_rc4_fixture()?;
    let password = PasswordSecret::new("user")?;
    let options = ValidationOptions::builder()
        .password(Some(password))
        .build();

    let report =
        Validator::new(options)?.validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    assert!(report.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            ParseFact::Encryption {
                revision: Some(2),
                decrypted: true,
                ..
            }
        )
    }));
    Ok(())
}

#[test]
fn test_should_validate_aesv2_encrypted_fixture_with_owner_password() -> Result<(), Box<dyn Error>>
{
    let fixture = encrypted_aesv2_fixture()?;
    let password = PasswordSecret::new("owner")?;
    let options = ValidationOptions::builder()
        .password(Some(password))
        .build();

    let report =
        Validator::new(options)?.validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    assert!(report.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            ParseFact::Encryption {
                revision: Some(4),
                algorithm: Some(algorithm),
                decrypted: true,
                ..
            } if algorithm.as_str() == "aesv2"
        )
    }));
    Ok(())
}

#[test]
fn test_should_return_encrypted_for_missing_or_wrong_password() -> Result<(), Box<dyn Error>> {
    let fixture = encrypted_rc4_fixture()?;
    let missing = Validator::new(ValidationOptions::default())?
        .validate_reader(Cursor::new(fixture.clone()), InputName::memory())?;
    let wrong = Validator::new(
        ValidationOptions::builder()
            .password(Some(PasswordSecret::new("wrong")?))
            .build(),
    )?
    .validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(missing.status, ValidationStatus::Encrypted);
    assert_eq!(wrong.status, ValidationStatus::Encrypted);
    assert!(wrong.warnings.iter().any(|warning| {
        matches!(
            warning,
            pdfv_core::ValidationWarning::General { message } if message.as_str() == "incorrect password"
        )
    }));
    Ok(())
}

#[test]
fn test_should_decrypt_strings_and_streams_under_limits() -> Result<(), Box<dyn Error>> {
    let fixture = encrypted_aesv2_fixture()?;
    let password = PasswordSecret::new("owner")?;
    let mut limits = ResourceLimits::default();
    limits.max_decrypted_string_bytes = 64;
    limits.max_decrypted_stream_bytes = 64;
    let mut parse_options = ParseOptions::default();
    parse_options.password = Some(&password);
    let document =
        Parser::new(limits.clone()).parse_with_options(Cursor::new(fixture), parse_options)?;
    let object_one = document
        .objects
        .get(&object_key(1))
        .ok_or_else(|| std::io::Error::other("missing object 1"))?;
    let dictionary = object_one
        .object
        .as_dictionary()
        .ok_or_else(|| std::io::Error::other("missing dictionary"))?;
    let Some(CosObject::String(title)) = dictionary.get("Title") else {
        return Err(std::io::Error::other("missing title string").into());
    };
    let object_two = document
        .objects
        .get(&object_key(2))
        .ok_or_else(|| std::io::Error::other("missing object 2"))?;
    let CosObject::Stream(stream) = &object_two.object else {
        return Err(std::io::Error::other("missing stream").into());
    };

    assert_eq!(title.as_bytes(), b"secret-title");
    assert_eq!(stream.decoded_bytes(&limits)?, b"stream-secret");
    Ok(())
}

#[test]
fn test_should_return_encrypted_for_unsupported_revision() -> Result<(), Box<dyn Error>> {
    let fixture = unsupported_revision_fixture();
    let password = PasswordSecret::new("user")?;
    let report = Validator::new(
        ValidationOptions::builder()
            .password(Some(password))
            .build(),
    )?
    .validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Encrypted);
    assert!(report.warnings.iter().any(|warning| {
        matches!(
            warning,
            pdfv_core::ValidationWarning::General { message } if message.as_str() == "unsupported encryption revision 6"
        )
    }));
    Ok(())
}

#[test]
fn test_should_return_encrypted_for_public_key_security_handler() -> Result<(), Box<dyn Error>> {
    let fixture = public_key_handler_fixture();
    let password = PasswordSecret::new("user")?;
    let report = Validator::new(
        ValidationOptions::builder()
            .password(Some(password))
            .build(),
    )?
    .validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Encrypted);
    assert!(report.warnings.iter().any(|warning| {
        matches!(
            warning,
            pdfv_core::ValidationWarning::General { message } if message.as_str().contains("unsupported security handler")
        )
    }));
    Ok(())
}

#[test]
fn test_should_parse_fail_malformed_encryption_dictionary_without_password()
-> Result<(), Box<dyn Error>> {
    let report = Validator::new(ValidationOptions::default())?.validate_reader(
        Cursor::new(malformed_encryption_fixture()),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::ParseFailed);
    Ok(())
}

#[test]
fn test_should_parse_fail_malformed_encryption_dictionary() -> Result<(), Box<dyn Error>> {
    let fixture = malformed_encryption_fixture();
    let password = PasswordSecret::new("user")?;
    let report = Validator::new(
        ValidationOptions::builder()
            .password(Some(password))
            .build(),
    )?
    .validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::ParseFailed);
    Ok(())
}

#[test]
fn test_should_redact_password_debug_and_serialization() -> Result<(), Box<dyn Error>> {
    let password = PasswordSecret::new("secret-value")?;
    let options = ValidationOptions::builder()
        .password(Some(password.clone()))
        .build();
    let debug = format!("{password:?} {options:?}");
    let json = serde_json::to_string(&options)?;

    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("secret-value"));
    assert!(!json.contains("password"));
    assert!(!json.contains("secret-value"));
    Ok(())
}

fn encrypted_rc4_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    let owner_key = owner_key(Revision::R2, 5, OWNER_PASSWORD);
    let owner_entry = rc4_crypt(&owner_key, &padded_password(USER_PASSWORD))?;
    let file_key = file_key(Revision::R2, 5, USER_PASSWORD, &owner_entry, true);
    let user_entry = rc4_crypt(&file_key, &PASSWORD_PADDING)?;
    let title = encrypt_object(
        Revision::R2,
        &file_key,
        object_key(1),
        CipherMethod::Rc4,
        b"secret-title",
    )?;
    let stream = encrypt_object(
        Revision::R2,
        &file_key,
        object_key(2),
        CipherMethod::Rc4,
        b"stream-secret",
    )?;
    let encrypt_dictionary = format!(
        "<< /Filter /Standard /V 1 /R 2 /Length 40 /O <{}> /U <{}> /P -4 >>",
        hex(&owner_entry),
        hex(&user_entry),
    );
    Ok(pdf_bytes(&title, &stream, &encrypt_dictionary))
}

fn encrypted_aesv2_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    let owner_key = owner_key(Revision::R4, 16, OWNER_PASSWORD);
    let mut owner_entry = padded_password(USER_PASSWORD).to_vec();
    for round in 0_u8..=19 {
        owner_entry = rc4_crypt(&xor_key(&owner_key, round), &owner_entry)?;
    }
    let file_key = file_key(Revision::R4, 16, USER_PASSWORD, &owner_entry, true);
    let mut user_entry = user_value_r4(&file_key)?;
    user_entry.resize(32, 0);
    let title = encrypt_object(
        Revision::R4,
        &file_key,
        object_key(1),
        CipherMethod::AesV2,
        b"secret-title",
    )?;
    let stream = encrypt_object(
        Revision::R4,
        &file_key,
        object_key(2),
        CipherMethod::AesV2,
        b"stream-secret",
    )?;
    let encrypt_dictionary = format!(
        "<< /Filter /Standard /V 4 /R 4 /Length 128 /O <{}> /U <{}> /P -4 /EncryptMetadata true \
         /CF << /StdCF << /CFM /AESV2 /Length 16 /AuthEvent /DocOpen >> >> /StmF /StdCF /StrF \
         /StdCF >>",
        hex(&owner_entry),
        hex(&user_entry),
    );
    Ok(pdf_bytes(&title, &stream, &encrypt_dictionary))
}

#[test]
fn test_should_validate_rc4_revision_three_fixture() -> Result<(), Box<dyn Error>> {
    let fixture = encrypted_rc4_revision_three_fixture()?;
    let password = PasswordSecret::new("user")?;
    let options = ValidationOptions::builder()
        .password(Some(password))
        .build();

    let report =
        Validator::new(options)?.validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    assert!(report.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            ParseFact::Encryption {
                revision: Some(3),
                decrypted: true,
                ..
            }
        )
    }));
    Ok(())
}

#[test]
fn test_should_validate_rc4_crypt_filter_revision_four_fixture() -> Result<(), Box<dyn Error>> {
    let fixture = encrypted_rc4_revision_four_fixture()?;
    let password = PasswordSecret::new("user")?;
    let options = ValidationOptions::builder()
        .password(Some(password))
        .build();

    let report =
        Validator::new(options)?.validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    assert!(report.parse_facts.iter().any(|fact| {
        matches!(
            fact,
            ParseFact::Encryption {
                revision: Some(4),
                algorithm: Some(algorithm),
                decrypted: true,
                ..
            } if algorithm.as_str() == "rc4"
        )
    }));
    Ok(())
}

#[test]
fn test_should_leave_metadata_stream_unencrypted_when_encrypt_metadata_false()
-> Result<(), Box<dyn Error>> {
    let fixture = encrypted_metadata_false_fixture()?;
    let password = PasswordSecret::new("user")?;
    let options = ValidationOptions::builder()
        .password(Some(password))
        .build();

    let report =
        Validator::new(options)?.validate_reader(Cursor::new(fixture), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    Ok(())
}

fn unsupported_revision_fixture() -> Vec<u8> {
    let encrypt_dictionary = "<< /Filter /Standard /V 5 /R 6 /Length 256 /O <00> /U <00> /P -4 >>";
    pdf_bytes(b"plain", b"plain", encrypt_dictionary)
}

fn public_key_handler_fixture() -> Vec<u8> {
    pdf_bytes(
        b"plain",
        b"plain",
        "<< /Filter /Adobe.PubSec /V 2 /R 3 /Length 128 /O <00> /U <00> /P -4 >>",
    )
}

fn malformed_encryption_fixture() -> Vec<u8> {
    pdf_bytes(b"plain", b"plain", "<< /Filter /Standard >>")
}

fn encrypted_rc4_revision_three_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    encrypted_rc4_fixture_for(Revision::R3, 16, 2, 3, None, true)
}

fn encrypted_rc4_revision_four_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    encrypted_rc4_fixture_for(
        Revision::R4,
        16,
        4,
        4,
        Some(
            "/CF << /StdCF << /CFM /V2 /Length 16 /AuthEvent /DocOpen >> >> /StmF /StdCF /StrF \
             /StdCF",
        ),
        true,
    )
}

fn encrypted_metadata_false_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    encrypted_rc4_fixture_for(
        Revision::R4,
        16,
        4,
        4,
        Some(
            "/EncryptMetadata false /CF << /StdCF << /CFM /V2 /Length 16 /AuthEvent /DocOpen >> \
             >> /StmF /StdCF /StrF /StdCF",
        ),
        false,
    )
}

fn encrypted_rc4_fixture_for(
    revision: Revision,
    key_len: usize,
    version: u8,
    revision_number: u8,
    extra: Option<&str>,
    encrypt_metadata: bool,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let owner_key = owner_key(revision, key_len, OWNER_PASSWORD);
    let mut owner_entry = padded_password(USER_PASSWORD).to_vec();
    match revision {
        Revision::R2 => owner_entry = rc4_crypt(&owner_key, &owner_entry)?,
        Revision::R3 | Revision::R4 => {
            for round in 0_u8..=19 {
                owner_entry = rc4_crypt(&xor_key(&owner_key, round), &owner_entry)?;
            }
        }
    }
    let file_key = file_key(
        revision,
        key_len,
        USER_PASSWORD,
        &owner_entry,
        encrypt_metadata,
    );
    let mut user_entry = match revision {
        Revision::R2 => rc4_crypt(&file_key, &PASSWORD_PADDING)?,
        Revision::R3 | Revision::R4 => user_value_r4(&file_key)?,
    };
    if matches!(revision, Revision::R3 | Revision::R4) {
        user_entry.resize(32, 0);
    }
    let title = encrypt_object(
        revision,
        &file_key,
        object_key(1),
        CipherMethod::Rc4,
        b"secret-title",
    )?;
    let stream = encrypt_object(
        revision,
        &file_key,
        object_key(2),
        CipherMethod::Rc4,
        b"stream-secret",
    )?;
    let extra = extra.unwrap_or("");
    let encrypt_dictionary = format!(
        "<< /Filter /Standard /V {version} /R {revision_number} /Length {} /O <{}> /U <{}> /P -4 \
         {extra} >>",
        key_len.saturating_mul(8),
        hex(&owner_entry),
        hex(&user_entry),
    );
    Ok(pdf_bytes(&title, &stream, &encrypt_dictionary))
}

fn pdf_bytes(title: &[u8], stream: &[u8], encrypt_dictionary: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0_usize];
    push_object(
        &mut bytes,
        &mut offsets,
        1,
        format!("<< /Type /Catalog /Title <{}> >>", hex(title)).as_bytes(),
    );
    let stream_header = format!("<< /Length {} >>\nstream\n", stream.len());
    push_object_stream(
        &mut bytes,
        &mut offsets,
        2,
        stream_header.as_bytes(),
        stream,
    );
    push_object(&mut bytes, &mut offsets, 3, encrypt_dictionary.as_bytes());
    let xref_offset = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Encrypt 3 0 R /Size {} /ID [<{}> <{}>] \
             >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len(),
            hex(DOCUMENT_ID),
            hex(DOCUMENT_ID),
        )
        .as_bytes(),
    );
    bytes
}

fn push_object(bytes: &mut Vec<u8>, offsets: &mut Vec<usize>, number: u32, body: &[u8]) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
}

fn push_object_stream(
    bytes: &mut Vec<u8>,
    offsets: &mut Vec<usize>,
    number: u32,
    header: &[u8],
    stream: &[u8],
) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(stream);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
}

#[derive(Clone, Copy)]
enum Revision {
    R2,
    R3,
    R4,
}

#[derive(Clone, Copy)]
enum CipherMethod {
    Rc4,
    AesV2,
}

fn owner_key(revision: Revision, key_len: usize, password: &[u8]) -> Vec<u8> {
    let mut digest = Md5::digest(padded_password(password)).to_vec();
    if matches!(revision, Revision::R3 | Revision::R4) {
        for _ in 0..50 {
            digest = Md5::digest(digest.get(..key_len).unwrap_or(&digest)).to_vec();
        }
    }
    digest.truncate(key_len);
    digest
}

fn file_key(
    revision: Revision,
    key_len: usize,
    password: &[u8],
    owner_entry: &[u8],
    encrypt_metadata: bool,
) -> Vec<u8> {
    let mut hasher = Md5::new();
    hasher.update(padded_password(password));
    hasher.update(owner_entry);
    hasher.update((-4_i32).to_le_bytes());
    hasher.update(DOCUMENT_ID);
    if matches!(revision, Revision::R4) && !encrypt_metadata {
        hasher.update([0xff, 0xff, 0xff, 0xff]);
    }
    let mut digest = hasher.finalize().to_vec();
    if matches!(revision, Revision::R3 | Revision::R4) {
        for _ in 0..50 {
            digest = Md5::digest(digest.get(..key_len).unwrap_or(&digest)).to_vec();
        }
    }
    digest.truncate(key_len);
    digest
}

fn user_value_r4(file_key: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut hasher = Md5::new();
    hasher.update(PASSWORD_PADDING);
    hasher.update(DOCUMENT_ID);
    let mut current = hasher.finalize().to_vec();
    for round in 0_u8..=19 {
        current = rc4_crypt(&xor_key(file_key, round), &current)?;
    }
    Ok(current)
}

fn encrypt_object(
    revision: Revision,
    file_key: &[u8],
    key: ObjectKey,
    method: CipherMethod,
    bytes: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let object_key = object_encryption_key(revision, file_key, key, method);
    match method {
        CipherMethod::Rc4 => rc4_crypt(&object_key, bytes),
        CipherMethod::AesV2 => aes_encrypt(&object_key, bytes),
    }
}

fn object_encryption_key(
    _revision: Revision,
    file_key: &[u8],
    key: ObjectKey,
    method: CipherMethod,
) -> Vec<u8> {
    let object_number = key.number.get().to_le_bytes();
    let generation = key.generation.to_le_bytes();
    let mut hasher = Md5::new();
    hasher.update(file_key);
    hasher.update(object_number.get(..3).unwrap_or(&object_number));
    hasher.update(generation.get(..2).unwrap_or(&generation));
    if matches!(method, CipherMethod::AesV2) {
        hasher.update(b"sAlT");
    }
    let mut digest = hasher.finalize().to_vec();
    digest.truncate(file_key.len().saturating_add(5).min(16));
    digest
}

fn rc4_crypt(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = bytes.to_vec();
    let mut cipher =
        Rc4::new_from_slice(key).map_err(|_| std::io::Error::other("invalid rc4 key"))?;
    cipher.apply_keystream(&mut output);
    Ok(output)
}

fn aes_encrypt(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let iv = [0x42_u8; 16];
    let mut buffer = vec![0_u8; bytes.len().saturating_add(16)];
    let ciphertext = Aes128CbcEnc::new_from_slices(key, &iv)
        .map_err(|_| std::io::Error::other("invalid aes key"))?
        .encrypt_padded_b2b::<Pkcs7>(bytes, &mut buffer)
        .map_err(|_| std::io::Error::other("invalid aes padding"))?;
    let mut output = iv.to_vec();
    output.extend_from_slice(ciphertext);
    Ok(output)
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

fn object_key(number: u32) -> ObjectKey {
    ObjectKey::new(NonZeroU32::new(number).unwrap_or(NonZeroU32::MIN), 0)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(
            DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'),
        ));
        output.push(char::from(
            DIGITS
                .get(usize::from(byte & 0x0f))
                .copied()
                .unwrap_or(b'0'),
        ));
    }
    output
}
