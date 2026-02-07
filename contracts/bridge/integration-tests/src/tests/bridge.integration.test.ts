import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { Binary, TxEvent, TxFinalized } from "polkadot-api";
import { Observable } from "rxjs";
import { randomBytes, createHash } from "crypto";
import {
	TestSetup,
	cleanupTestEnvironment,
	type TestContext,
	type ContractSdk,
} from "../setup";
import { createInkSdk } from "@polkadot-api/sdk-ink";
import { devnet, contracts } from "@polkadot-api/descriptors";
import {
	addContractAsProxy,
	createHotkey,
	expectDepositRequestCreatedEvent,
	expectWithdrawalCompletedEvent,
	findContractEvent,
	fundAccount,
	getRandomWallet,
	registerSubnet,
	registerValidator,
	elevateRegistrationLimits,
	seedBridgeStake,
	taoToRao,
	waitForBlocks,
	type Wallet,
} from "../utils";
import {
	getStakeBalance,
	moveStake,
} from "../utils/stake-helpers";

const MIN_DEPOSIT = 1_000_000_000n;

async function trackTx(observable: Observable<TxEvent>): Promise<TxFinalized> {
	return new Promise<TxFinalized>((resolve, reject) => {
		const subscription = observable.subscribe({
			next: (event) => {
				if (event.type === "finalized") {
					subscription.unsubscribe();
					resolve(event);
				}
			},
			error: (error) => {
				subscription.unsubscribe();
				reject(error);
			},
		});
	});
}

async function signAndFinalize(
	call: ReturnType<ReturnType<ContractSdk["getContract"]>["send"]>,
	signer: Parameters<typeof call.signSubmitAndWatch>[0]
): Promise<TxFinalized> {
	const observable = call.signSubmitAndWatch(signer);
	return await trackTx(observable);
}

function randomHash(label: string): Binary {
	const hash = createHash("blake2b512").update(label).update(randomBytes(16)).digest("hex");
	return Binary.fromHex(hash.slice(0, 64));
}

function toBinary(value: string | Uint8Array | Binary | { toHex?: () => string }): Binary {
	if (value instanceof Binary) {
		return value;
	}
	if (typeof value === "string") {
		const hex = value.startsWith("0x") ? value.slice(2) : value;
		return Binary.fromHex(hex);
	}
	if (value && typeof value === "object" && "toHex" in value && typeof value.toHex === "function") {
		const hex = value.toHex().replace(/^0x/, "");
		return Binary.fromHex(hex);
	}
	if (value instanceof Uint8Array) {
		return Binary.fromBytes(value);
	}
	return Binary.fromBytes(new Uint8Array(value as ArrayLike<number>));
}

describe("Bridge Contract Integration", () => {
	let context: TestContext;
	let contract: ReturnType<ContractSdk["getContract"]>;
	let netuid: number;

	let aliceHotkey: Wallet;
	let bobHotkey: Wallet;
	let charlieHotkey: Wallet;
	let daveHotkey: Wallet;
	let eveHotkey: Wallet;

	let bobDepositRequestId: Binary | undefined;
	let bobDepositAmount: bigint = 0n;

	let charlieDepositRequestId: Binary | undefined;
	let charlieDepositAmount: bigint = 0n;

	beforeAll(async () => {
		const setup = TestSetup.getInstance();
		const api = await setup.getApi();
		const { accounts, guardians, contractHotkey } = setup.createTestAccounts();

		// Create hotkeys
		aliceHotkey = createHotkey(accounts.alice.derivePath);
		bobHotkey = createHotkey(accounts.bob.derivePath);
		charlieHotkey = createHotkey(accounts.charlie.derivePath);
		daveHotkey = createHotkey(accounts.dave.derivePath);
		eveHotkey = createHotkey(accounts.eve.derivePath);

		// Fund hotkeys for fees
		await fundAccount(api, aliceHotkey.address, taoToRao(50), accounts.alice.signer);
		await fundAccount(api, bobHotkey.address, taoToRao(10), accounts.alice.signer);
		await fundAccount(api, charlieHotkey.address, taoToRao(10), accounts.alice.signer);
		await fundAccount(api, daveHotkey.address, taoToRao(10), accounts.alice.signer);
		await fundAccount(api, eveHotkey.address, taoToRao(10), accounts.alice.signer);

		// Register subnet FIRST to get netuid
		netuid = await registerSubnet(api, aliceHotkey.address, accounts.alice.signer);
		await elevateRegistrationLimits(api, netuid, 32, 32, accounts.alice.signer);
		await waitForBlocks(api, 1);

		// Deploy contract with chain_id = netuid
		const contractAddress = await setup.deployContract(api, accounts.alice, contractHotkey, netuid);

		// Fund contract account and contract hotkey for transaction fees
		await fundAccount(api, contractAddress, taoToRao(25), accounts.alice.signer);
		await fundAccount(api, contractHotkey.address, taoToRao(25), accounts.alice.signer);

		// Create contract SDK and context
		const contractSdk = createInkSdk(api, contracts.bridge);
		context = {
			api,
			contractSdk,
			accounts,
			guardians,
			contractHotkey,
			contractAddress,
		};
		contract = contractSdk.getContract(contractAddress);

		// Register validators
		await registerValidator(context.api, netuid, aliceHotkey.address, context.accounts.alice.signer, taoToRao(400));
		await registerValidator(context.api, netuid, bobHotkey.address, context.accounts.bob.signer, taoToRao(200));
		await registerValidator(context.api, netuid, charlieHotkey.address, context.accounts.charlie.signer, taoToRao(200));
		await registerValidator(context.api, netuid, daveHotkey.address, context.accounts.dave.signer, taoToRao(200));
		await registerValidator(context.api, netuid, eveHotkey.address, context.accounts.eve.signer, taoToRao(200));
		await registerValidator(context.api, netuid, context.contractHotkey.address, context.contractHotkey.signer, taoToRao(100));

		// Provide initial stake liquidity to the contract for withdrawal tests
		await moveStake(
			context.api,
			aliceHotkey.address,
			context.contractHotkey.address,
			netuid,
			taoToRao(50),
			context.accounts.alice.signer
		);
		await seedBridgeStake(
			context.api,
			netuid,
			context.accounts.alice.signer,
			context.contractAddress!,
			aliceHotkey.address,
			taoToRao(25),
		);

		// Allow contract to act as proxy for depositors
		await addContractAsProxy(context.api, context.contractAddress!, context.accounts.bob.signer);
		await addContractAsProxy(context.api, context.contractAddress!, context.accounts.charlie.signer);
		await addContractAsProxy(context.api, context.contractAddress!, context.accounts.dave.signer);
		await addContractAsProxy(context.api, context.contractAddress!, context.accounts.eve.signer);

		await waitForBlocks(context.api, 2);
	}, 240000);

	afterAll(async () => {
		await cleanupTestEnvironment();
	});

	describe("Deployment & Configuration", () => {
		it("exposes initial contract parameters", async () => {
			expect(context.contractAddress).toBeDefined();

			const ownerResult = await contract.query("owner", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(ownerResult.success).toBe(true);
			if (ownerResult.success) {
				expect(ownerResult.value.response).toBe(context.accounts.alice.address);
			}

			const chainIdResult = await contract.query("chain_id", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(chainIdResult.success).toBe(true);
			if (chainIdResult.success) {
				expect(Number(chainIdResult.value.response)).toBe(netuid);
			}

			const pausedResult = await contract.query("is_paused", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(pausedResult.success).toBe(true);
			if (pausedResult.success) {
				expect(pausedResult.value.response).toBe(false);
			}

			const guardiansResult = await contract.query("guardians", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(guardiansResult.success).toBe(true);
			if (guardiansResult.success) {
				expect(guardiansResult.value.response.length).toBe(0);
			}

			const nonceResult = await contract.query("next_deposit_nonce", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(nonceResult.success).toBe(true);
			if (nonceResult.success) {
				expect(nonceResult.value.response).toBe(0n);
			}

			const minDepositResult = await contract.query("min_deposit_amount", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(minDepositResult.success).toBe(true);
			if (minDepositResult.success) {
				expect(minDepositResult.value.response).toBe(MIN_DEPOSIT);
			}
		});
	});

	describe("Guardian Management", () => {
		it("allows the owner to configure guardians and threshold", async () => {
			const guardianAddresses = context.guardians.map((guardian) => guardian.address);

			const tx = contract.send("set_guardians_and_threshold", {
				origin: context.accounts.alice.address,
				data: {
					guardians: guardianAddresses,
					approve_threshold: 2,
				},
			});
			const finalized = await signAndFinalize(tx, context.accounts.alice.signer);
			const event = findContractEvent(contract, finalized, "GuardiansUpdated");
			expect(event).toBeDefined();

			const storedGuardians = await contract.query("guardians", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(storedGuardians.success).toBe(true);
			if (storedGuardians.success) {
				expect(storedGuardians.value.response).toEqual(guardianAddresses);
			}

			const thresholdResult = await contract.query("approve_threshold", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(thresholdResult.success).toBe(true);
			if (thresholdResult.success) {
				expect(thresholdResult.value.response).toBe(2);
			}
		});

		it("rejects guardian configuration from non-owner accounts", async () => {
			const attempt = await contract.query("set_guardians_and_threshold", {
				origin: context.accounts.bob.address,
				data: {
					guardians: context.guardians.map((guardian) => guardian.address),
					approve_threshold: 2,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Deposit Flow", () => {
		it("deposits stake and creates a deposit request", async () => {
			bobDepositAmount = MIN_DEPOSIT * 2n;

			const bobStakeBefore = await getStakeBalance(
				context.api,
				bobHotkey.address,
				netuid,
				context.accounts.bob.address
			);

			const tx = contract.send("deposit", {
				origin: context.accounts.bob.address,
				data: {
					amount: bobDepositAmount,
					hotkey: bobHotkey.address,
					netuid,
				},
			});
			const finalized = await signAndFinalize(tx, context.accounts.bob.signer);
			const depositEvent = expectDepositRequestCreatedEvent(contract, finalized);

			bobDepositRequestId = toBinary(depositEvent.deposit_request_id);
			expect(depositEvent.amount).toBe(bobDepositAmount);

			// Verify deposit request ID can be looked up by nonce
			const depositIdByNonce = await contract.query("get_deposit_request_id_by_nonce", {
				origin: context.accounts.alice.address,
				data: {
					nonce: depositEvent.deposit_nonce,
				},
			});

			expect(depositIdByNonce.success).toBe(true);
			if (depositIdByNonce.success) {
				expect(toBinary(depositIdByNonce.value.response?.asHex() || '0x').toString()).toBe(
					toBinary(bobDepositRequestId).toString(),
				);
			}

			// Verify deposit request record exists
			const depositRequest = await contract.query("get_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: bobDepositRequestId,
				},
			});

			expect(depositRequest.success).toBe(true);
			if (depositRequest.success) {
				const request = depositRequest.value.response;
				expect(request).toBeDefined();
				expect(request.amount).toBe(bobDepositAmount);
				expect(request.sender).toBe(context.accounts.bob.address);
				expect(request.recipient).toBe(context.accounts.bob.address);
				expect(request.status.type).toBe("Requested");
			}

			// Verify stake was transferred
			const bobStakeAfter = await getStakeBalance(
				context.api,
				bobHotkey.address,
				netuid,
				context.accounts.bob.address
			);
			expect(bobStakeAfter).toBeLessThan(bobStakeBefore);

			const contractStake = await getStakeBalance(
				context.api,
				context.contractHotkey.address,
				netuid,
				context.contractAddress!
			);
			expect(contractStake).toBeGreaterThan(0n);
		});

		it("rejects deposits below minimum amount", async () => {
			const result = await contract.query("deposit", {
				origin: context.accounts.charlie.address,
				data: {
					amount: 1n,
					hotkey: charlieHotkey.address,
					netuid,
				},
			});

			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.value.type).toBe("FlagReverted");
			}
		});

		it("rejects deposits when stake is insufficient", async () => {
			const randomWallet = getRandomWallet();
			await fundAccount(context.api, randomWallet.address, taoToRao(5), context.accounts.alice.signer);

			const result = await contract.query("deposit", {
				origin: randomWallet.address,
				data: {
					amount: MIN_DEPOSIT,
					hotkey: randomWallet.address,
					netuid,
				},
			});

			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.value.type).toBe("FlagReverted");
			}
		});

		it("rejects deposits with invalid netuid", async () => {
			const invalidNetuid = netuid + 1;

			const result = await contract.query("deposit", {
				origin: context.accounts.charlie.address,
				data: {
					amount: MIN_DEPOSIT,
					hotkey: charlieHotkey.address,
					netuid: invalidNetuid,
				},
			});

			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.value.type).toBe("FlagReverted");
			}
		});

		it("returns none for an unknown deposit nonce", async () => {
			const unknownNonce = 9_999_999n;
			const result = await contract.query("get_deposit_request_id_by_nonce", {
				origin: context.accounts.alice.address,
				data: {
					nonce: unknownNonce,
				},
			});

			expect(result.success).toBe(true);
			if (result.success) {
				expect(result.value.response).toBeUndefined();
			}
		});
	});

	describe("Withdrawal Flow", () => {
		it("handles guardian attestations and finalizes withdrawal", async () => {
			const withdrawalId = randomHash("bob-withdrawal");
			const withdrawalAmount = bobDepositAmount;

			const contractStakeBefore = await getStakeBalance(
				context.api,
				context.contractHotkey.address,
				netuid,
				context.contractAddress!
			);

			// First guardian attests (creates the withdrawal record)
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: withdrawalAmount,
				},
			});
			const attest1Finalized = await signAndFinalize(attest1Tx, context.guardians[0].signer);
			const attestEvent = findContractEvent(contract, attest1Finalized, "WithdrawalAttested");
			expect(attestEvent).toBeDefined();

			// Verify the withdrawal record was created as Pending
			const pendingQuery = await contract.query("get_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			expect(pendingQuery.success).toBe(true);
			if (pendingQuery.success) {
				expect(pendingQuery.value.response).toBeDefined();
				expect(pendingQuery.value.response.status.type).toBe("Pending");
				expect(pendingQuery.value.response.votes.length).toBe(1);
			}

			// Second guardian attests (reaches threshold=2, triggers finalization)
			const attest2Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: withdrawalAmount,
				},
			});
			const attest2Finalized = await signAndFinalize(attest2Tx, context.guardians[1].signer);
			expectWithdrawalCompletedEvent(contract, attest2Finalized);

			// Verify contract stake decreased (Alpha released to Bob)
			const contractStakeAfter = await getStakeBalance(
				context.api,
				context.contractHotkey.address,
				netuid,
				context.contractAddress!
			);
			expect(contractStakeAfter).toBeLessThan(contractStakeBefore);

			// Verify withdrawal status is Completed
			const completedQuery = await contract.query("get_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			expect(completedQuery.success).toBe(true);
			if (completedQuery.success) {
				expect(completedQuery.value.response.status.type).toBe("Completed");
			}
		});

		it("rejects attestation with mismatched recipient", async () => {
			const withdrawalId = randomHash("mismatch-recipient");

			// First guardian attests with correct recipient
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			// Second guardian attests with wrong recipient
			const attempt = await contract.query("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.charlie.address,
					amount: MIN_DEPOSIT,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects attestation with mismatched amount", async () => {
			const withdrawalId = randomHash("mismatch-amount");

			// First guardian attests
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			// Second guardian attests with wrong amount
			const attempt = await contract.query("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT * 2n,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Admin Recovery", () => {
		it("admin fails a deposit request and manually releases Alpha", async () => {
			charlieDepositAmount = MIN_DEPOSIT * 3n;

			// Charlie deposits
			const depositTx = contract.send("deposit", {
				origin: context.accounts.charlie.address,
				data: {
					amount: charlieDepositAmount,
					hotkey: charlieHotkey.address,
					netuid,
				},
			});
			const depositFinalized = await signAndFinalize(depositTx, context.accounts.charlie.signer);
			const depositEvent = expectDepositRequestCreatedEvent(contract, depositFinalized);
			charlieDepositRequestId = toBinary(depositEvent.deposit_request_id);

			// Verify deposit request is in Requested status
			const requestQuery = await contract.query("get_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: charlieDepositRequestId,
				},
			});
			expect(requestQuery.success).toBe(true);
			if (requestQuery.success) {
				expect(requestQuery.value.response.status.type).toBe("Requested");
				expect(requestQuery.value.response.amount).toBe(charlieDepositAmount);
			}

			// Admin fails the deposit request
			const failTx = contract.send("admin_fail_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: charlieDepositRequestId,
				},
			});
			const failFinalized = await signAndFinalize(failTx, context.accounts.alice.signer);
			const failEvent = findContractEvent(contract, failFinalized, "DepositRequestFailed");
			expect(failEvent).toBeDefined();

			// Verify deposit request is now Failed
			const failedQuery = await contract.query("get_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: charlieDepositRequestId,
				},
			});
			expect(failedQuery.success).toBe(true);
			if (failedQuery.success) {
				expect(failedQuery.value.response.status.type).toBe("Failed");
			}

			// Admin manually releases Alpha back to Charlie
			const releaseTx = contract.send("admin_manual_release", {
				origin: context.accounts.alice.address,
				data: {
					recipient: context.accounts.charlie.address,
					amount: charlieDepositAmount,
					deposit_request_id: charlieDepositRequestId,
				},
			});
			const releaseFinalized = await signAndFinalize(releaseTx, context.accounts.alice.signer);
			const releaseEvent = findContractEvent(contract, releaseFinalized, "AdminManualRelease");
			expect(releaseEvent).toBeDefined();
		});

		it("rejects admin_fail_deposit_request from non-owner", async () => {
			if (!bobDepositRequestId) {
				throw new Error("Bob deposit not available");
			}

			const attempt = await contract.query("admin_fail_deposit_request", {
				origin: context.accounts.bob.address,
				data: {
					request_id: bobDepositRequestId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("admin cancels a stuck withdrawal", async () => {
			const withdrawalId = randomHash("cancel-withdrawal");

			// First guardian creates a pending withdrawal
			const attestTx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attestTx, context.guardians[0].signer);

			// Admin cancels the withdrawal
			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			const cancelFinalized = await signAndFinalize(cancelTx, context.accounts.alice.signer);
			const cancelEvent = findContractEvent(contract, cancelFinalized, "WithdrawalCancelled");
			expect(cancelEvent).toBeDefined();

			// Verify withdrawal is Cancelled
			const cancelledQuery = await contract.query("get_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			expect(cancelledQuery.success).toBe(true);
			if (cancelledQuery.success) {
				expect(cancelledQuery.value.response.status.type).toBe("Cancelled");
			}
		});
	});

	describe("Administrative Controls", () => {
		it("toggles pause state and blocks deposit operations", async () => {
			const pauseTx = contract.send("pause", {
				origin: context.accounts.alice.address,
				data: {},
			});
			await signAndFinalize(pauseTx, context.accounts.alice.signer);

			const pausedState = await contract.query("is_paused", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(pausedState.success).toBe(true);
			if (pausedState.success) {
				expect(pausedState.value.response).toBe(true);
			}

			// Deposit should be rejected when paused
			const depositAttempt = await contract.query("deposit", {
				origin: context.accounts.dave.address,
				data: {
					amount: MIN_DEPOSIT,
					hotkey: daveHotkey.address,
					netuid,
				},
			});
			expect(depositAttempt.success).toBe(false);
			if (!depositAttempt.success) {
				expect(depositAttempt.value.type).toBe("FlagReverted");
			}

			// attest_withdrawal should also be rejected when paused
			const attestAttempt = await contract.query("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: randomHash("paused-attest"),
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			expect(attestAttempt.success).toBe(false);
			if (!attestAttempt.success) {
				expect(attestAttempt.value.type).toBe("FlagReverted");
			}

			const unpauseTx = contract.send("unpause", {
				origin: context.accounts.alice.address,
				data: {},
			});
			await signAndFinalize(unpauseTx, context.accounts.alice.signer);
		});

		it("updates owner and contract hotkey", async () => {
			const updateOwnerTx = contract.send("update_owner", {
				origin: context.accounts.alice.address,
				data: {
					new_owner: context.accounts.bob.address,
				},
			});
			await signAndFinalize(updateOwnerTx, context.accounts.alice.signer);

			const ownerAfter = await contract.query("owner", {
				origin: context.accounts.bob.address,
				data: {},
			});
			expect(ownerAfter.success).toBe(true);
			if (ownerAfter.success) {
				expect(ownerAfter.value.response).toBe(context.accounts.bob.address);
			}

			const newHotkey = createHotkey("//BridgeUpdatedHotkey");
			await fundAccount(context.api, newHotkey.address, taoToRao(5), context.accounts.alice.signer);

			const updateHotkeyTx = contract.send("set_contract_hotkey", {
				origin: context.accounts.bob.address,
				data: {
					hotkey: newHotkey.address,
				},
			});
			await signAndFinalize(updateHotkeyTx, context.accounts.bob.signer);

			const hotkeyQuery = await contract.query("contract_hotkey", {
				origin: context.accounts.bob.address,
				data: {},
			});
			expect(hotkeyQuery.success).toBe(true);
			if (hotkeyQuery.success) {
				expect(hotkeyQuery.value.response).toBe(newHotkey.address);
			}

			// Revert owner and hotkey for remaining tests
			const revertHotkeyTx = contract.send("set_contract_hotkey", {
				origin: context.accounts.bob.address,
				data: {
					hotkey: context.contractHotkey.address,
				},
			});
			await signAndFinalize(revertHotkeyTx, context.accounts.bob.signer);

			const revertOwnerTx = contract.send("update_owner", {
				origin: context.accounts.bob.address,
				data: {
					new_owner: context.accounts.alice.address,
				},
			});
			await signAndFinalize(revertOwnerTx, context.accounts.bob.signer);
		});

		it("updates cleanup TTL and min deposit amount", async () => {
			// Update cleanup TTL
			const setTtlTx = contract.send("set_cleanup_ttl", {
				origin: context.accounts.alice.address,
				data: {
					ttl_blocks: 50000,
				},
			});
			const ttlFinalized = await signAndFinalize(setTtlTx, context.accounts.alice.signer);
			const ttlEvent = findContractEvent(contract, ttlFinalized, "CleanupTTLUpdated");
			expect(ttlEvent).toBeDefined();

			const ttlQuery = await contract.query("cleanup_ttl", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(ttlQuery.success).toBe(true);
			if (ttlQuery.success) {
				expect(ttlQuery.value.response).toBe(50000);
			}

			// Restore default TTL
			const restoreTtlTx = contract.send("set_cleanup_ttl", {
				origin: context.accounts.alice.address,
				data: {
					ttl_blocks: 100800,
				},
			});
			await signAndFinalize(restoreTtlTx, context.accounts.alice.signer);

			// Update min deposit amount
			const setMinTx = contract.send("set_min_deposit_amount", {
				origin: context.accounts.alice.address,
				data: {
					amount: MIN_DEPOSIT * 2n,
				},
			});
			const minFinalized = await signAndFinalize(setMinTx, context.accounts.alice.signer);
			const minEvent = findContractEvent(contract, minFinalized, "MinDepositAmountUpdated");
			expect(minEvent).toBeDefined();

			const minQuery = await contract.query("min_deposit_amount", {
				origin: context.accounts.alice.address,
				data: {},
			});
			expect(minQuery.success).toBe(true);
			if (minQuery.success) {
				expect(minQuery.value.response).toBe(MIN_DEPOSIT * 2n);
			}

			// Restore default min deposit
			const restoreMinTx = contract.send("set_min_deposit_amount", {
				origin: context.accounts.alice.address,
				data: {
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(restoreMinTx, context.accounts.alice.signer);
		});
	});

	describe("Access Control Regression", () => {
		it("prevents non-guardians from attesting withdrawals", async () => {
			const nonGuardian = context.accounts.eve;

			const attestAttempt = await contract.query("attest_withdrawal", {
				origin: nonGuardian.address,
				data: {
					request_id: randomHash("unauthorized-attest"),
					recipient: nonGuardian.address,
					amount: MIN_DEPOSIT,
				},
			});
			expect(attestAttempt.success).toBe(false);
			if (!attestAttempt.success) {
				expect(attestAttempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects duplicate guardian attestations on the same withdrawal", async () => {
			const withdrawalId = randomHash("duplicate-attest");

			// Temporarily raise threshold so withdrawal stays pending
			const adjustTx = contract.send("set_guardians_and_threshold", {
				origin: context.accounts.alice.address,
				data: {
					guardians: context.guardians.map((g) => g.address),
					approve_threshold: 3,
				},
			});
			await signAndFinalize(adjustTx, context.accounts.alice.signer);

			// First attestation by guardian 0
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			// Duplicate attestation by guardian 0 should fail
			const duplicateAttempt = await contract.query("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			expect(duplicateAttempt.success).toBe(false);
			if (!duplicateAttempt.success) {
				expect(duplicateAttempt.value.type).toBe("FlagReverted");
			}

			// Clean up: cancel the pending withdrawal and restore threshold
			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			await signAndFinalize(cancelTx, context.accounts.alice.signer);

			const restoreTx = contract.send("set_guardians_and_threshold", {
				origin: context.accounts.alice.address,
				data: {
					guardians: context.guardians.map((g) => g.address),
					approve_threshold: 2,
				},
			});
			await signAndFinalize(restoreTx, context.accounts.alice.signer);
		});

		it("rejects attestation on an already completed withdrawal", async () => {
			const withdrawalId = randomHash("already-completed");

			// First guardian attests
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.eve.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			// Second guardian attests (reaches threshold, completes withdrawal)
			const attest2Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.eve.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest2Tx, context.guardians[1].signer);

			// Third guardian tries to attest on already-completed withdrawal
			const attempt = await contract.query("attest_withdrawal", {
				origin: context.guardians[2].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.eve.address,
					amount: MIN_DEPOSIT,
				},
			});
			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_manual_release from non-owner", async () => {
			const attempt = await contract.query("admin_manual_release", {
				origin: context.accounts.bob.address,
				data: {
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT,
					deposit_request_id: undefined,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_cancel_withdrawal from non-owner", async () => {
			const withdrawalId = randomHash("cancel-non-owner");

			// Create a pending withdrawal
			const attestTx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attestTx, context.guardians[0].signer);

			// Non-owner tries to cancel
			const attempt = await contract.query("admin_cancel_withdrawal", {
				origin: context.accounts.bob.address,
				data: {
					request_id: withdrawalId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}

			// Clean up
			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			await signAndFinalize(cancelTx, context.accounts.alice.signer);
		});

		it("rejects pause and unpause from non-owner", async () => {
			const pauseAttempt = await contract.query("pause", {
				origin: context.accounts.bob.address,
				data: {},
			});
			expect(pauseAttempt.success).toBe(false);
			if (!pauseAttempt.success) {
				expect(pauseAttempt.value.type).toBe("FlagReverted");
			}

			const unpauseAttempt = await contract.query("unpause", {
				origin: context.accounts.bob.address,
				data: {},
			});
			expect(unpauseAttempt.success).toBe(false);
			if (!unpauseAttempt.success) {
				expect(unpauseAttempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects set_cleanup_ttl from non-owner", async () => {
			const attempt = await contract.query("set_cleanup_ttl", {
				origin: context.accounts.bob.address,
				data: {
					ttl_blocks: 1,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects set_min_deposit_amount from non-owner", async () => {
			const attempt = await contract.query("set_min_deposit_amount", {
				origin: context.accounts.bob.address,
				data: {
					amount: 1n,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Admin Edge Cases", () => {
		it("rejects admin_fail_deposit_request on already-failed request", async () => {
			if (!charlieDepositRequestId) {
				throw new Error("Charlie deposit not available");
			}

			// Charlie's deposit was already failed in the Admin Recovery tests
			const attempt = await contract.query("admin_fail_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: charlieDepositRequestId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_fail_deposit_request on non-existent request", async () => {
			const attempt = await contract.query("admin_fail_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: randomHash("nonexistent-deposit"),
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_cancel_withdrawal on already-cancelled withdrawal", async () => {
			const withdrawalId = randomHash("double-cancel");

			// Create and cancel a withdrawal
			const attestTx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attestTx, context.guardians[0].signer);

			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			await signAndFinalize(cancelTx, context.accounts.alice.signer);

			// Try to cancel again
			const attempt = await contract.query("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_cancel_withdrawal on completed withdrawal", async () => {
			// Ensure contract has enough stake on the contract hotkey for this withdrawal
			await seedBridgeStake(
				context.api,
				netuid,
				context.accounts.alice.signer,
				context.contractAddress!,
				context.contractHotkey.address,
				taoToRao(5),
			);

			const withdrawalId = randomHash("cancel-completed");

			// Create and complete a withdrawal
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.eve.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			const attest2Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.eve.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest2Tx, context.guardians[1].signer);

			// Verify the withdrawal actually completed
			const statusQuery = await contract.query("get_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			expect(statusQuery.success).toBe(true);
			if (statusQuery.success) {
				expect(statusQuery.value.response.status.type).toBe("Completed");
			}

			// Try to cancel a completed withdrawal
			const attempt = await contract.query("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects admin_cancel_withdrawal on non-existent withdrawal", async () => {
			const attempt = await contract.query("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: randomHash("nonexistent-withdrawal"),
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Cleanup Flows", () => {
		const SMALL_TTL = 5;

		it("cleans up a deposit request after TTL expires", async () => {
			// Set a small TTL for testing
			const setTtlTx = contract.send("set_cleanup_ttl", {
				origin: context.accounts.alice.address,
				data: {
					ttl_blocks: SMALL_TTL,
				},
			});
			await signAndFinalize(setTtlTx, context.accounts.alice.signer);

			// Create a deposit
			const depositTx = contract.send("deposit", {
				origin: context.accounts.dave.address,
				data: {
					amount: MIN_DEPOSIT * 2n,
					hotkey: daveHotkey.address,
					netuid,
				},
			});
			const depositFinalized = await signAndFinalize(depositTx, context.accounts.dave.signer);
			const depositEvent = expectDepositRequestCreatedEvent(contract, depositFinalized);
			const depositRequestId = toBinary(depositEvent.deposit_request_id);

			// Verify the deposit request exists
			const existsQuery = await contract.query("get_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: depositRequestId,
				},
			});
			expect(existsQuery.success).toBe(true);
			if (existsQuery.success) {
				expect(existsQuery.value.response).toBeDefined();
			}

			// Wait for TTL to expire
			await waitForBlocks(context.api, SMALL_TTL + 2);

			// Cleanup should succeed (anyone can call it)
			const cleanupTx = contract.send("cleanup_deposit_request", {
				origin: context.accounts.eve.address,
				data: {
					request_id: depositRequestId,
				},
			});
			const cleanupFinalized = await signAndFinalize(cleanupTx, context.accounts.eve.signer);
			const cleanupEvent = findContractEvent(contract, cleanupFinalized, "DepositRequestCleanedUp");
			expect(cleanupEvent).toBeDefined();

			// Verify the deposit request is removed
			const removedQuery = await contract.query("get_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: depositRequestId,
				},
			});
			expect(removedQuery.success).toBe(true);
			if (removedQuery.success) {
				expect(removedQuery.value.response).toBeUndefined();
			}
		}, 120000);

		it("rejects cleanup of deposit request before TTL expires", async () => {
			// Create a deposit (TTL is still set to SMALL_TTL from previous test)
			const depositTx = contract.send("deposit", {
				origin: context.accounts.eve.address,
				data: {
					amount: MIN_DEPOSIT * 2n,
					hotkey: eveHotkey.address,
					netuid,
				},
			});
			const depositFinalized = await signAndFinalize(depositTx, context.accounts.eve.signer);
			const depositEvent = expectDepositRequestCreatedEvent(contract, depositFinalized);
			const depositRequestId = toBinary(depositEvent.deposit_request_id);

			// Try to cleanup immediately (before TTL)
			const attempt = await contract.query("cleanup_deposit_request", {
				origin: context.accounts.alice.address,
				data: {
					request_id: depositRequestId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});

		it("cleans up a completed withdrawal after TTL expires", async () => {
			const withdrawalId = randomHash("cleanup-completed");

			// Create and complete a withdrawal
			const attest1Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest1Tx, context.guardians[0].signer);

			const attest2Tx = contract.send("attest_withdrawal", {
				origin: context.guardians[1].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.bob.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attest2Tx, context.guardians[1].signer);

			// Wait for TTL to expire
			await waitForBlocks(context.api, SMALL_TTL + 2);

			// Cleanup should succeed
			const cleanupTx = contract.send("cleanup_withdrawal", {
				origin: context.accounts.eve.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			const cleanupFinalized = await signAndFinalize(cleanupTx, context.accounts.eve.signer);
			const cleanupEvent = findContractEvent(contract, cleanupFinalized, "WithdrawalCleanedUp");
			expect(cleanupEvent).toBeDefined();

			// Verify the withdrawal is removed
			const removedQuery = await contract.query("get_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			expect(removedQuery.success).toBe(true);
			if (removedQuery.success) {
				expect(removedQuery.value.response).toBeUndefined();
			}
		}, 120000);

		it("cleans up a cancelled withdrawal after TTL expires", async () => {
			const withdrawalId = randomHash("cleanup-cancelled");

			// Create and cancel a withdrawal
			const attestTx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attestTx, context.guardians[0].signer);

			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			await signAndFinalize(cancelTx, context.accounts.alice.signer);

			// Wait for TTL to expire
			await waitForBlocks(context.api, SMALL_TTL + 2);

			// Cleanup should succeed
			const cleanupTx = contract.send("cleanup_withdrawal", {
				origin: context.accounts.eve.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});
			const cleanupFinalized = await signAndFinalize(cleanupTx, context.accounts.eve.signer);
			const cleanupEvent = findContractEvent(contract, cleanupFinalized, "WithdrawalCleanedUp");
			expect(cleanupEvent).toBeDefined();
		}, 120000);

		it("rejects cleanup of a pending (non-finalized) withdrawal", async () => {
			const withdrawalId = randomHash("cleanup-pending");

			// Create a pending withdrawal (not finalized)
			const attestTx = contract.send("attest_withdrawal", {
				origin: context.guardians[0].address,
				data: {
					request_id: withdrawalId,
					recipient: context.accounts.dave.address,
					amount: MIN_DEPOSIT,
				},
			});
			await signAndFinalize(attestTx, context.guardians[0].signer);

			// Wait for TTL
			await waitForBlocks(context.api, SMALL_TTL + 2);

			// Cleanup should fail because withdrawal is still pending
			const attempt = await contract.query("cleanup_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					withdrawal_id: withdrawalId,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}

			// Clean up: cancel the pending withdrawal and restore TTL
			const cancelTx = contract.send("admin_cancel_withdrawal", {
				origin: context.accounts.alice.address,
				data: {
					request_id: withdrawalId,
				},
			});
			await signAndFinalize(cancelTx, context.accounts.alice.signer);

			// Restore default TTL
			const restoreTtlTx = contract.send("set_cleanup_ttl", {
				origin: context.accounts.alice.address,
				data: {
					ttl_blocks: 100800,
				},
			});
			await signAndFinalize(restoreTtlTx, context.accounts.alice.signer);
		}, 120000);
	});
});
