//! Signatures checked: SignedData's signers against the content, and each signer's certificate
//! against the trust anchors (`chain`).

use super::asn1;
use super::ber::Implicit;
use super::cert::Cert;
use super::chain;
use super::open::{Checked, Keys, Signer, inner};
use super::sign::{DigestAlg, signature_digest, verifies};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::OctetString;
use der::{Decode, Encode};
use mail_domain::{BadSignature, Coverage, SmimeVerification};

/// SignedData checked. `detached` is the content for a detached signature; otherwise the content
/// is the one the SignedData encapsulates.
pub(super) fn signed_data(
    info: &ContentInfo,
    detached: Option<&[u8]>,
    keys: &Keys<'_>,
    coverage: Coverage,
    from: Option<&str>,
) -> Checked {
    let bad = |why: BadSignature| Checked {
        content: detached.map(<[u8]>::to_vec).unwrap_or_default(),
        verification: SmimeVerification::Bad { why, coverage },
        signer: None,
    };
    let Some(data) = inner::<SignedData>(info, Implicit::Keep) else {
        return bad(BadSignature::Malformed {
            why: "the signature could not be read".to_owned(),
        });
    };
    let encapsulated = data
        .encap_content_info
        .econtent
        .as_ref()
        .map(|any| any.value().to_vec());
    let content = match (detached, &encapsulated) {
        (Some(bytes), _) => bytes.to_vec(),
        (None, Some(bytes)) => bytes.clone(),
        (None, None) => {
            return bad(BadSignature::Malformed {
                why: "the signature covers no content".to_owned(),
            });
        }
    };
    let carried: Vec<Cert> = data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(cert) => Cert::from_parsed(cert.clone()),
            CertificateChoices::Other(_) => None,
        })
        .collect();
    let verdicts: Vec<(SmimeVerification, Option<Signer>)> = data
        .signer_infos
        .0
        .iter()
        .map(|si| {
            one_signer(
                si,
                &data.encap_content_info.econtent_type,
                &content,
                &carried,
                keys,
                coverage,
                from,
            )
        })
        .collect();
    // The best verdict among the signers: a good one, then one whose certificate falls short,
    // then a bad one, then one nobody knows.
    let rank = |v: &SmimeVerification| match v {
        SmimeVerification::Good { .. } => 0,
        SmimeVerification::Doubtful { .. } => 1,
        SmimeVerification::Bad { .. } => 2,
        SmimeVerification::UnknownSigner { .. } => 3,
        SmimeVerification::NoSignature => 4,
    };
    let (verification, signer) = verdicts
        .into_iter()
        .min_by_key(|(v, _)| rank(v))
        .unwrap_or((SmimeVerification::NoSignature, None));
    Checked {
        content,
        verification,
        signer,
    }
}

/// The last instant a SHA-1 signature is believed (RFC 8551 §2.2: SHA-1 is historic; this client
/// takes 2020 as when collisions became practical).
fn sha1_cutoff() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn one_signer(
    si: &SignerInfo,
    content_type: &const_oid::ObjectIdentifier,
    content: &[u8],
    carried: &[Cert],
    keys: &Keys<'_>,
    coverage: Coverage,
    from: Option<&str>,
) -> (SmimeVerification, Option<Signer>) {
    let bad = |why: BadSignature| (SmimeVerification::Bad { why, coverage }, None);
    let pool: Vec<Cert> = carried.iter().chain(keys.certs).cloned().collect();
    let Some(cert) = pool.iter().find(|c| identifies(&si.sid, c)).cloned() else {
        return (SmimeVerification::UnknownSigner { coverage }, None);
    };
    let signer = Signer {
        chain: carried
            .iter()
            .filter(|c| c.der() != cert.der())
            .cloned()
            .collect(),
        cert: cert.clone(),
    };
    let digest = match DigestAlg::of(&si.digest_alg.oid) {
        Ok(digest) => digest,
        Err(name) if si.digest_alg.oid == asn1::MD5 => {
            return bad(BadSignature::Weak { algorithm: name });
        }
        Err(name) => return bad(BadSignature::Unsupported { algorithm: name }),
    };
    let sig_digest = match signature_digest(&si.signature_algorithm, digest) {
        Ok(d) => d,
        Err(why) => return bad(why),
    };
    let signed_at = si.signed_attrs.as_ref().and_then(signing_time);
    if (digest == DigestAlg::Sha1 || sig_digest == DigestAlg::Sha1)
        && signed_at.is_none_or(|at| at >= sha1_cutoff())
    {
        return bad(BadSignature::Weak {
            algorithm: "SHA-1".to_owned(),
        });
    }
    let content_hash = digest.hash(content);
    let hash = match &si.signed_attrs {
        Some(attrs) => {
            let stated = attribute(attrs, &asn1::MESSAGE_DIGEST)
                .and_then(|any| any.decode_as::<OctetString>().ok());
            match stated {
                Some(stated) if stated.as_bytes() == content_hash.as_slice() => {}
                Some(_) => return bad(BadSignature::Altered),
                None => {
                    return bad(BadSignature::Malformed {
                        why: "the signature states no message digest".to_owned(),
                    });
                }
            }
            let stated_type = attribute(attrs, &asn1::CONTENT_TYPE)
                .and_then(|any| any.decode_as::<const_oid::ObjectIdentifier>().ok());
            if stated_type.as_ref() != Some(content_type) {
                return bad(BadSignature::Malformed {
                    why: "the signature's content type is not the content's".to_owned(),
                });
            }
            // The signature is over the attributes' DER encoding with the SET tag (RFC 5652
            // §5.4), not the IMPLICIT [0] they travel under.
            let Ok(der) = attrs.to_der() else {
                return bad(BadSignature::Malformed {
                    why: "the signed attributes could not be encoded".to_owned(),
                });
            };
            sig_digest.hash(&der)
        }
        None if sig_digest == digest => content_hash,
        None => sig_digest.hash(content),
    };
    match verifies(
        &cert.inner.tbs_certificate.subject_public_key_info,
        &si.signature_algorithm,
        sig_digest,
        &hash,
        si.signature.as_bytes(),
    ) {
        Ok(true) => {}
        Ok(false) => return bad(BadSignature::Forged),
        Err(why) => return bad(why),
    }
    // Checked for when it says it was signed — unless that is in the future, which nobody can
    // have done — and for now when it does not say.
    let at = signed_at
        .filter(|at| *at <= keys.now + TimeDelta::days(1))
        .unwrap_or(keys.now);
    let problems = chain::problems(&cert, &pool, keys.anchors, at, from);
    let signer_fp = cert.fingerprint();
    let verification = if problems.is_empty() {
        SmimeVerification::Good {
            signer: signer_fp,
            coverage,
        }
    } else {
        SmimeVerification::Doubtful {
            signer: signer_fp,
            problems,
            coverage,
        }
    };
    (verification, Some(signer))
}

/// Whether `sid` names `cert`.
fn identifies(sid: &SignerIdentifier, cert: &Cert) -> bool {
    let tbs = &cert.inner.tbs_certificate;
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(ias) => {
            tbs.issuer == ias.issuer && tbs.serial_number == ias.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => {
            cert.subject_key_id().as_deref() == Some(ski.0.as_bytes())
        }
    }
}

fn attribute<'a>(
    attrs: &'a x509_cert::attr::Attributes,
    oid: &const_oid::ObjectIdentifier,
) -> Option<&'a der::Any> {
    attrs
        .iter()
        .find(|a| a.oid == *oid)
        .and_then(|a| a.values.iter().next())
}

fn signing_time(attrs: &x509_cert::attr::Attributes) -> Option<DateTime<Utc>> {
    let time =
        x509_cert::time::Time::from_der(&attribute(attrs, &asn1::SIGNING_TIME)?.to_der().ok()?)
            .ok()?;
    let since = time.to_unix_duration();
    DateTime::from_timestamp(i64::try_from(since.as_secs()).ok()?, 0)
}
