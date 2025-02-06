use crate::{mock::*, Error, Event, pallet, UID, Role, crypto::MetagraphAuthId};
use frame_support::{assert_ok, assert_noop};
use sp_core::{sr25519, crypto::{Pair as TraitPair, Ss58Codec}};
use sp_runtime::transaction_validity::TransactionSource;
use frame_system::offchain::{SignedPayload, SigningTypes};

#[test]
fn test_decode_hex() {
    new_test_ext().execute_with(|| {
        // Test valid hex string
        let hex = "0x0102";
        let result = MetagraphModule::decode_hex(hex).unwrap();
        assert_eq!(result, vec![1, 2]);

        // Test hex string without 0x prefix
        let hex = "0102";
        let result = MetagraphModule::decode_hex(hex).unwrap();
        assert_eq!(result, vec![1, 2]);

        // Test invalid hex string (odd length)
        let hex = "0x123";
        assert!(MetagraphModule::decode_hex(hex).is_err());

        // Test invalid hex string (invalid characters)
        let hex = "0x12GG";
        assert!(MetagraphModule::decode_hex(hex).is_err());
    });
}

#[test]
fn test_parse_uid_from_hex() {
    new_test_ext().execute_with(|| {
        // Create a test hex string
        // Format: prefix + id (2 bytes) + public key (32 bytes)
        let prefix = "658faa385070e074c85bf6b568cf0555aab1b4e78e1ea8305462ee53b3686dc81300";
        let id = "0013"; // ID = 19
        let pubkey = "5CFpNJLJMaYwpydsXP8YNEjiSPmpUVgBDfzUAhNzYxdDp191";
        let public = sr25519::Public::from_ss58check(pubkey).unwrap();
        let pubkey_hex = hex::encode(public.as_ref());
        
        let hex = format!("0x{}{}{}", prefix, id, pubkey_hex);
        
        // Parse the UID
        let uid = MetagraphModule::decode_storage_key_to_address(&hex).unwrap();
        
        // Verify the parsed values
        assert_eq!(uid.id, 19);
        assert_eq!(uid.address.to_ss58check(), pubkey);
        assert_eq!(uid.role, Role::None); // Default role
    });
}

#[test]
fn test_submit_hot_keys() {
    new_test_ext().execute_with(|| {
        let (pair, _) = sr25519::Pair::generate();
        let public = pair.public();
        
        // Create test UIDs
        let uids = vec![
            UID {
                address: public.clone(),
                id: 19,
                role: Role::Validator,
            },
        ];
        
        // Create signed payload
        let payload = pallet::SignedPayloadType::<Test> {
            hot_keys: uids.clone(),
            public: public.clone(),
        };
        
        // Sign the payload
        let signature = pair.sign(&payload.encode());
        
        // Submit the hot keys
        assert_ok!(MetagraphModule::submit_hot_keys(
            RuntimeOrigin::none(),
            payload.clone(),
            signature.clone(),
        ));
        
        // Verify storage was updated
        assert_eq!(MetagraphModule::get_uids(), uids);
        
        // Get the emitted event
        let events = System::events();
        let event = events.iter().find(|record| matches!(
            record.event,
            RuntimeEvent::MetagraphModule(Event::SignedPayloadProcessed { .. })
        )).expect("Should have emitted SignedPayloadProcessed event");
        
        // Verify event contents
        if let RuntimeEvent::MetagraphModule(Event::SignedPayloadProcessed {
            payload: event_payload,
            signature: event_signature,
            signer,
            hot_keys_count,
        }) = &event.event {
            assert_eq!(event_payload, &payload.encode());
            assert_eq!(event_signature, &signature.encode());
            assert_eq!(signer, &public.encode());
            assert_eq!(hot_keys_count, &Some(1));
        }
        
        // Test with invalid signature
        let (wrong_pair, _) = sr25519::Pair::generate();
        let wrong_signature = wrong_pair.sign(&payload.encode());
        
        // Submit should fail with invalid signature
        assert_noop!(
            MetagraphModule::submit_hot_keys(
                RuntimeOrigin::none(),
                payload,
                wrong_signature,
            ),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn test_unsigned_validation() {
    new_test_ext().execute_with(|| {
        let (pair, _) = sr25519::Pair::generate();
        let public = pair.public();
        
        // Create test UIDs
        let uids = vec![
            UID {
                address: public.clone(),
                id: 19,
                role: Role::Validator,
            },
        ];
        
        // Create signed payload
        let payload = pallet::SignedPayloadType::<Test> {
            hot_keys: uids,
            public: public.clone(),
        };
        
        // Sign the payload
        let signature = pair.sign(&payload.encode());
        
        let call = crate::Call::<Test>::submit_hot_keys { 
            payload: payload.clone(),
            signature: signature.clone(),
        };
        
        assert!(MetagraphModule::validate_unsigned(TransactionSource::Local, &call).is_ok());
        
        // Test with invalid signature
        let (wrong_pair, _) = sr25519::Pair::generate();
        let wrong_signature = wrong_pair.sign(&payload.encode());
        
        let invalid_call = crate::Call::<Test>::submit_hot_keys { 
            payload,
            signature: wrong_signature,
        };
        
        assert!(MetagraphModule::validate_unsigned(TransactionSource::Local, &invalid_call).is_err());
    });
}

#[test]
fn test_ss58_encoding() {
    new_test_ext().execute_with(|| {
        // Test SS58 encoding/decoding
        let test_address = "5CFpNJLJMaYwpydsXP8YNEjiSPmpUVgBDfzUAhNzYxdDp191";
        let public = sr25519::Public::from_ss58check(test_address).unwrap();
        assert_eq!(public.to_ss58check(), test_address);
        
        // Create a UID with the decoded address
        let uid = UID {
            address: public.clone(),
            id: 19,
            role: Role::None,
        };
        
        // Verify the address in the UID matches
        assert_eq!(uid.address.to_ss58check(), test_address);
    });
}
