//! The inner hardware-wrapped storage blob is distinct from the outer
//! KeyMint key that decrypts its on-disk envelope. Android's keystore2 may
//! renew this inner blob before converting it; vold retains the original.

use anyhow::{Context, Result};
use skyward_binder::status;

const KEY_REQUIRES_UPGRADE: i32 = -62;

/// Convert once, optionally renewing only on KeyMint's explicit -62. The
/// renewed blob lives only for the second conversion. This makes no claim
/// about a vendor's secure-world side effects of upgradeKey.
pub(crate) fn convert(
    original: &[u8],
    allow_renewal: bool,
    mut convert: impl FnMut(&[u8]) -> Result<Vec<u8>>,
    renew: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
) -> Result<(Vec<u8>, bool)> {
    match convert(original) {
        Ok(key) => {
            anyhow::ensure!(!key.is_empty(), "the key service returned an empty ephemeral storage key");
            Ok((key, false))
        }
        Err(e) if status::refused_with(&e) == Some(KEY_REQUIRES_UPGRADE) => {
            if !allow_renewal {
                return Err(e.context("the existing storage key requires an upgrade; the decryption policy forbids it"));
            }
            let renewed = renew(original).context("renew the inner wrapped storage key for this boot")?;
            anyhow::ensure!(!renewed.is_empty(), "the key service returned an empty renewed storage blob");
            let key = convert(&renewed).context("convert the renewed inner wrapped storage key")?;
            anyhow::ensure!(!key.is_empty(), "the key service returned an empty ephemeral storage key");
            Ok((key, true))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn refused(code: i32) -> anyhow::Error {
        status::Fault::Service { code, said: String::new() }.into()
    }

    #[test]
    fn a_usable_storage_blob_needs_no_renewal_even_when_permitted() {
        let result = convert(b"original", true, |_| Ok(b"ephemeral".to_vec()), |_| panic!("unexpected renewal")).unwrap();
        assert_eq!(result, (b"ephemeral".to_vec(), false));
    }

    #[test]
    fn preserving_storage_blobs_stops_before_any_renewal() {
        let e = convert(b"original", false, |_| Err(refused(-62)), |_| panic!("renewal is forbidden")).unwrap_err();
        assert_eq!(status::refused_with(&e), Some(-62));
        assert!(format!("{e:#}").contains("policy forbids it"));
    }

    #[test]
    fn only_the_renewed_inner_blob_is_converted_again_and_original_is_unchanged() {
        let original = b"original".to_vec();
        let calls = RefCell::new(Vec::new());
        let result = convert(
            &original,
            true,
            |key| {
                calls.borrow_mut().push(("convert", key.to_vec()));
                if key == b"original" { Err(refused(-62)) } else { Ok(b"ephemeral".to_vec()) }
            },
            |key| {
                calls.borrow_mut().push(("renew", key.to_vec()));
                Ok(b"renewed".to_vec())
            },
        ).unwrap();
        assert_eq!(result, (b"ephemeral".to_vec(), true));
        assert_eq!(original, b"original");
        assert_eq!(calls.into_inner(), [("convert", b"original".to_vec()), ("renew", b"original".to_vec()), ("convert", b"renewed".to_vec())]);
    }

    #[test]
    fn other_service_errors_and_binder_failures_never_trigger_renewal() {
        for code in [-26, -30, -38, -50, -100] {
            let e = convert(b"original", true, |_| Err(refused(code)), |_| panic!("unexpected renewal")).unwrap_err();
            assert_eq!(status::refused_with(&e), Some(code));
        }
        let e = convert(b"original", true, |_| Err(status::Fault::Call { exception: -1, said: String::new() }.into()), |_| panic!("unexpected renewal")).unwrap_err();
        assert_eq!(status::refused_with(&e), None);
    }

    #[test]
    fn a_failed_renewal_stops_without_another_conversion() {
        let calls = RefCell::new(Vec::new());
        let e = convert(b"original", true,
            |_| { calls.borrow_mut().push("convert"); Err(refused(-62)) },
            |_| { calls.borrow_mut().push("renew"); Err(refused(-38)) },
        ).unwrap_err();
        assert_eq!(status::refused_with(&e), Some(-38));
        assert_eq!(calls.into_inner(), ["convert", "renew"]);
    }

    #[test]
    fn a_second_upgrade_request_is_returned_without_looping() {
        let calls = RefCell::new(Vec::new());
        let e = convert(b"original", true,
            |_| { calls.borrow_mut().push("convert"); Err(refused(-62)) },
            |_| { calls.borrow_mut().push("renew"); Ok(b"renewed".to_vec()) },
        ).unwrap_err();
        assert_eq!(status::refused_with(&e), Some(-62));
        assert_eq!(calls.into_inner(), ["convert", "renew", "convert"]);
    }

    #[test]
    fn an_empty_renewed_blob_is_never_sent_back_to_keymint() {
        let calls = RefCell::new(Vec::new());
        let e = convert(b"original", true,
            |_| { calls.borrow_mut().push("convert"); Err(refused(-62)) },
            |_| { calls.borrow_mut().push("renew"); Ok(Vec::new()) },
        ).unwrap_err();
        assert!(format!("{e:#}").contains("empty renewed storage blob"));
        assert_eq!(calls.into_inner(), ["convert", "renew"]);
    }

    #[test]
    fn empty_ephemeral_results_never_report_success() {
        let e = convert(b"original", true, |_| Ok(Vec::new()), |_| panic!("unexpected renewal")).unwrap_err();
        assert!(format!("{e:#}").contains("empty ephemeral storage key"));
        let e = convert(b"original", true,
            |key| if key == b"original" { Err(refused(-62)) } else { Ok(Vec::new()) },
            |_| Ok(b"renewed".to_vec()),
        ).unwrap_err();
        assert!(format!("{e:#}").contains("empty ephemeral storage key"));
    }
}
