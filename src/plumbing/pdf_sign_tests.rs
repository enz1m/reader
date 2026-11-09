use super::*;

const FIXTURE: &str = "tests/fixtures/multiline-prose.pdf";

fn test_signer(password: &str) -> Vec<u8> {
    use openssl::asn1::Asn1Time;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::{X509, X509NameBuilder};

    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "Ada Lovelace").unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(30).unwrap())
        .unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    let cert = cert.build();
    openssl::pkcs12::Pkcs12::builder()
        .name("Ada")
        .pkey(&key)
        .cert(&cert)
        .build2(password)
        .unwrap()
        .to_der()
        .unwrap()
}

fn covered(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let at = find(bytes, b"/ByteRange").expect("a byte range");
    let open = at + bytes[at..].iter().position(|b| *b == b'[').unwrap() + 1;
    let close = open + bytes[open..].iter().position(|b| *b == b']').unwrap();
    let r: Vec<usize> = std::str::from_utf8(&bytes[open..close])
        .unwrap()
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let mut data = bytes[r[0]..r[0] + r[1]].to_vec();
    data.extend_from_slice(&bytes[r[2]..r[2] + r[3]]);
    let hex = std::str::from_utf8(&bytes[r[1] + 1..r[2] - 1]).unwrap();
    let der: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    (data, der)
}

fn verifies(data: &[u8], der: &[u8]) -> bool {
    use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
    // ASN.1 lengths distinguish signature bytes from the PDF padding.
    let Ok(pkcs7) = Pkcs7::from_der(der) else {
        return false;
    };
    let store = openssl::x509::store::X509StoreBuilder::new()
        .unwrap()
        .build();
    let certs = openssl::stack::Stack::new().unwrap();
    pkcs7
        .verify(
            &certs,
            &store,
            Some(data),
            None,
            Pkcs7Flags::NOVERIFY | Pkcs7Flags::BINARY,
        )
        .is_ok()
}

#[test]
fn a_certificate_signature_verifies_and_catches_tampering() {
    let p12 = test_signer("hunter2");
    assert!(matches!(
        Signer::from_pkcs12(&p12, "wrong"),
        Err(SignError::Certificate)
    ));
    let signer = Signer::from_pkcs12(&p12, "hunter2").unwrap();
    assert_eq!(signer.name(), "Ada Lovelace");

    let doc = Document::load(FIXTURE).unwrap();
    let signed = sign_with_certificate(
        doc,
        &signer,
        Placement {
            page: 0,
            x: 72.0,
            y: 120.0,
            width: 200.0,
            height: 60.0,
        },
    )
    .unwrap();

    let (data, der) = covered(&signed);
    assert_eq!(der.len(), SIGNATURE_SPACE);
    assert_eq!(
        data.len() + SIGNATURE_SPACE * 2 + 2,
        signed.len(),
        "the range covers the whole file but the signature"
    );
    assert!(verifies(&data, &der), "the signature does not verify");

    let mut tampered = signed.clone();
    let i = tampered.len() - 40;
    tampered[i] ^= 0x01;
    let (data, der) = covered(&tampered);
    assert!(!verifies(&data, &der), "a changed byte still verified");

    // An independent verifier, when the system has one.
    if let Ok(out) = std::process::Command::new("pdfsig").arg("-").output()
        && out.status.code().is_some()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signed.pdf");
        std::fs::write(&path, &signed).unwrap();
        let out = std::process::Command::new("pdfsig")
            .arg(&path)
            .output()
            .unwrap();
        let report = String::from_utf8_lossy(&out.stdout);
        assert!(
            report.contains("Signature is Valid"),
            "pdfsig did not accept the signature:\n{report}"
        );
    }
}
