// use crate::{
//     mock::*,
//     Config,
//     Error,
//     Event,
//     NodeRankings,
//     // NodeType,
//     Pallet as RankingPallet,
// };
// use frame_support::{
//     assert_noop,
//     assert_ok,
//     traits::Get,
// };
// use pallet_registration::NodeType;
// use frame_system::RawOrigin;
// use sp_runtime::traits::Zero;

// #[test]
// fn test_update_rankings_works() {
//     new_test_ext().execute_with(|| {
//         // Prepare test data
//         let node_ids = vec![
//             b"node1".to_vec(),
//             b"node2".to_vec(),
//             b"node3".to_vec(),
//         ];
//         let weights = vec![100, 50, 75];
//         let node_types = vec![
//             NodeType::Relay,
//             NodeType::Miner,
//             NodeType::Relay,
//         ];
//         let node_ss58_addresses = vec![
//             b"address1".to_vec(),
//             b"address2".to_vec(),
//             b"address3".to_vec(),
//         ];

//         // Simulate node registration
//         for (i, node_id) in node_ids.iter().enumerate() {
//             pallet_registration::Pallet::<Test>::register_node(
//                 RawOrigin::Root.into(),
//                 node_id.clone(),
//                 node_ss58_addresses[i].clone(),
//                 node_types[i].clone(),
//                 1, // owner
//             ).expect("Should register node");
//         }

//         // Update rankings
//         assert_ok!(
//             RankingPallet::<Test>::do_update_rankings(
//                 weights.clone(),
//                 node_ss58_addresses.clone(),
//                 node_ids.clone(),
//                 node_types.clone()
//             )
//         );

//         // Verify rankings
//         let ranked_list = RankingPallet::<Test>::ranked_list();
//         assert_eq!(ranked_list.len(), 3);

//         // Check ranking order (descending by weight)
//         assert_eq!(ranked_list[0].node_id, b"node1");
//         assert_eq!(ranked_list[0].rank, 1);
//         assert_eq!(ranked_list[1].node_id, b"node3");
//         assert_eq!(ranked_list[1].rank, 2);
//         assert_eq!(ranked_list[2].node_id, b"node2");
//         assert_eq!(ranked_list[2].rank, 3);
//     });
// }

// #[test]
// fn test_update_rankings_with_invalid_input() {
//     new_test_ext().execute_with(|| {
//         // Mismatched vector lengths should fail
//         let node_ids = vec![b"node1".to_vec(), b"node2".to_vec()];
//         let weights = vec![100];
//         let node_types = vec![NodeType::Relay];
//         let node_ss58_addresses = vec![b"address1".to_vec()];

//         assert_noop!(
//             RankingPallet::<Test>::do_update_rankings(
//                 weights,
//                 node_ss58_addresses,
//                 node_ids,
//                 node_types
//             ),
//             Error::<Test>::InvalidInput
//         );
//     });
// }

// #[test]
// fn test_update_rank_distribution_limit() {
//     new_test_ext().execute_with(|| {
//         // Initial limit
//         let initial_limit = RankingPallet::<Test>::rank_distribution_limit();
//         assert_eq!(initial_limit, 10); // From mock runtime config

//         // Update limit via sudo
//         assert_ok!(
//             RankingPallet::<Test>::update_rank_distribution_limit(
//                 RawOrigin::Root.into(),
//                 20
//             )
//         );

//         // Verify updated limit
//         let updated_limit = RankingPallet::<Test>::rank_distribution_limit();
//         assert_eq!(updated_limit, 20);
//     });
// }

// #[test]
// fn test_reward_distribution_on_initialize() {
//     new_test_ext().execute_with(|| {
//         // Prepare test data with registered nodes
//         let node_ids = vec![
//             b"relay1".to_vec(),
//             b"miner1".to_vec(),
//             b"relay2".to_vec(),
//         ];
//         let weights = vec![100, 50, 75];
//         let node_types = vec![
//             NodeType::Relay,
//             NodeType::Miner,
//             NodeType::Relay,
//         ];
//         let node_ss58_addresses = vec![
//             b"relay1_address".to_vec(),
//             b"miner1_address".to_vec(),
//             b"relay2_address".to_vec(),
//         ];

//         // Simulate node registration
//         for (i, node_id) in node_ids.iter().enumerate() {
//             pallet_registration::Pallet::<Test>::register_node(
//                 RawOrigin::Root.into(),
//                 node_id.clone(),
//                 node_ss58_addresses[i].clone(),
//                 node_types[i].clone(),
//                 i as u64 + 1, // owner
//             ).expect("Should register node");
//         }

//         // Update rankings
//         assert_ok!(
//             RankingPallet::<Test>::do_update_rankings(
//                 weights.clone(),
//                 node_ss58_addresses.clone(),
//                 node_ids.clone(),
//                 node_types.clone()
//             )
//         );

//         // Add initial balance to pallet account for rewards
//         let pallet_account = RankingPallet::<Test>::account_id();
//         let initial_reward_balance = 10_000;

//         // Manually set pallet account balance
//         <pallet_balances::Pallet::<Test> as frame_support::traits::Currency<_>>::make_free_balance_be(
//             &pallet_account,
//             initial_reward_balance
//         );

//         // Trigger on_initialize to distribute rewards
//         let weight_used = RankingPallet::<Test>::on_initialize(1);

//         // Verify non-zero weight used
//         assert!(!weight_used.is_zero());

//         // Additional checks could include verifying reward events,
//         // checking account balances, etc.
//     });
// }

// #[test]
// fn test_unauthorized_rank_distribution_limit_update() {
//     new_test_ext().execute_with(|| {
//         // Non-root origin should fail to update distribution limit
//         assert_noop!(
//             RankingPallet::<Test>::update_rank_distribution_limit(
//                 RawOrigin::Signed(1).into(),
//                 20
//             ),
//             frame_system::Error::<Test>::CallFiltered
//         );
//     });
// }
