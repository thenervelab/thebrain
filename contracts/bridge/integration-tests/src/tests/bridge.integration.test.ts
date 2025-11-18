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
	expectDepositEvent,
	expectRefundedEvent,
	expectReleasedEvent,
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
const SIGNATURE_TTL_BLOCKS = 100;

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
	// If already a Binary, return as-is
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
	// Fallback: try to convert to Uint8Array first
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

	let lastBurnCheckpoint = 0n;
	let lastRefundCheckpoint = 0n;

	let bobDepositId: Binary | undefined;
	let bobDepositAmount: bigint = 0n;

	let charlieDepositId: Binary | undefined;
	let charlieDepositAmount: bigint = 0n;

	let daveDepositId: Binary | undefined;
	let daveDepositAmount: bigint = 0n;

	let eveDepositId: Binary | undefined;
	let eveDepositAmount: bigint = 0n;

	beforeAll(async () => {
		// Setup API and accounts (without deploying contract yet)
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

		// Provide initial stake liquidity to the contract for release tests
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
		});
	});

	describe("Guardian Management", () => {
		it("allows the owner to configure guardians and thresholds", async () => {
			const guardianAddresses = context.guardians.map((guardian) => guardian.address);

			const tx = contract.send("set_guardians_and_thresholds", {
				origin: context.accounts.alice.address,
				data: {
					guardians: guardianAddresses,
					approve_threshold: 2,
					deny_threshold: 1,
				},
			});
			const finalized = await signAndFinalize(tx, context.accounts.alice.signer);
			const allEvents = contract.filterEvents(finalized.events);
			console.log("Events emitted:", JSON.stringify(allEvents, (_, v) => typeof v === 'bigint' ? v.toString() : v, 2));
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
		});

		it("rejects guardian configuration from non-owner accounts", async () => {
			const attempt = await contract.query("set_guardians_and_thresholds", {
				origin: context.accounts.bob.address,
				data: {
					guardians: context.guardians.map((guardian) => guardian.address),
					approve_threshold: 2,
					deny_threshold: 1,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				// Query dry-runs that revert have type "FlagReverted"
				// The specific error (Unauthorized) is encoded in value.value.message
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Lock Flow", () => {
		it("locks stake and records deposit metadata", async () => {
			bobDepositAmount = MIN_DEPOSIT * 2n;

			const bobStakeBefore = await getStakeBalance(
				context.api,
				bobHotkey.address,
				netuid,
				context.accounts.bob.address
			);

			const tx = contract.send("lock", {
				origin: context.accounts.bob.address,
				data: {
					amount: bobDepositAmount,
					hotkey: bobHotkey.address,
					netuid,
				},
			});
			const finalized = await signAndFinalize(tx, context.accounts.bob.signer);
			const depositEvent = expectDepositEvent(contract, finalized);

			bobDepositId = toBinary(depositEvent.deposit_id);
			expect(depositEvent.amount).toBe(bobDepositAmount);

			const depositIdByNonce = await contract.query("get_deposit_id_by_nonce", {
				origin: context.accounts.alice.address,
				data: {
					deposit_nonce: depositEvent.deposit_nonce,
				},
			});

			expect(depositIdByNonce.success).toBe(true);
			if (depositIdByNonce.success) {
				expect(toBinary(depositIdByNonce.value.response?.asHex() || '0x').toString()).toBe(
					toBinary(bobDepositId).toString(),
				);
			}

			const lockedQuery = await contract.query("get_locked_amount", {
				origin: context.accounts.alice.address,
				data: {
					deposit_id: bobDepositId,
				},
			});

			expect(lockedQuery.success).toBe(true);
			if (lockedQuery.success) {
				expect(lockedQuery.value.response).toBe(bobDepositAmount);
			}

			const depositsResult = await contract.query("get_user_deposits", {
				origin: context.accounts.alice.address,
				data: {
					user: context.accounts.bob.address,
				},
			});
			expect(depositsResult.success).toBe(true);
			if (depositsResult.success) {
				const deposits = depositsResult.value.response.map((id) => id.toString());
				expect(deposits).toContain(depositEvent.deposit_id.toString());
			}

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
			const result = await contract.query("lock", {
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

			const result = await contract.query("lock", {
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

		it("returns none for an unknown deposit nonce", async () => {
			const unknownNonce = 9_999_999n;
			const result = await contract.query("get_deposit_id_by_nonce", {
				origin: context.accounts.alice.address,
				data: {
					deposit_nonce: unknownNonce,
				},
			});

			expect(result.success).toBe(true);
			if (result.success) {
				expect(result.value.response).toBeUndefined();
			}
		});
	});

	describe("Release Flow", () => {
		it("handles guardian approvals and finalizes releases", async () => {
			if (!bobDepositId) {
				throw new Error("Bob's deposit was not recorded");
			}

			const burnId = randomHash("bob-release");
			const checkpointNonce = lastBurnCheckpoint + 1n;
			const burns = [{
				burn_id: burnId,
				recipient: context.accounts.bob.address,
				hotkey: bobHotkey.address,
				netuid,
				amount: bobDepositAmount,
			}];

			const proposeTx = contract.send("propose_releases", {
				origin: context.guardians[0].address,
				data: {
					burns,
					checkpoint_nonce: checkpointNonce,
				},
			});
			await signAndFinalize(proposeTx, context.guardians[0].signer);

			const pendingQuery = await contract.query("has_pending_burn", {
				origin: context.accounts.alice.address,
				data: {
					burn_id: burnId,
				},
			});
			expect(pendingQuery.success).toBe(true);
			if (pendingQuery.success) {
				expect(pendingQuery.value.response).toBe(true);
			}

			const contractStakeBefore = await getStakeBalance(
				context.api,
				context.contractHotkey.address,
				netuid,
				context.contractAddress!
			);

			const attestTx = contract.send("attest_release", {
				origin: context.guardians[1].address,
				data: {
					burn_id: burnId,
					approve: true,
				},
			});
			const finalized = await signAndFinalize(attestTx, context.guardians[1].signer);
			expectReleasedEvent(contract, finalized);

			const contractStakeAfter = await getStakeBalance(
				context.api,
				context.contractHotkey.address,
				netuid,
				context.contractAddress!
			);
			expect(contractStakeAfter).toBeLessThan(contractStakeBefore);

			const pendingAfter = await contract.query("has_pending_burn", {
				origin: context.accounts.alice.address,
				data: {
					burn_id: burnId,
				},
			});
			expect(pendingAfter.success).toBe(true);
			if (pendingAfter.success) {
				expect(pendingAfter.value.response).toBe(false);
			}

			lastBurnCheckpoint = checkpointNonce;
		});
	});

	describe("Refund Flow", () => {
		it("processes denied deposits and refunds stake", async () => {
			charlieDepositAmount = MIN_DEPOSIT * 3n;

			const lockTx = contract.send("lock", {
				origin: context.accounts.charlie.address,
				data: {
					amount: charlieDepositAmount,
					hotkey: charlieHotkey.address,
					netuid,
				},
			});
			const lockFinalized = await signAndFinalize(lockTx, context.accounts.charlie.signer);
			const depositEvent = expectDepositEvent(contract, lockFinalized);
			charlieDepositId = toBinary(depositEvent.deposit_id);

			const charlieLockedAmount = await contract.query("get_locked_amount", {
				origin: context.accounts.alice.address,
				data: {
					deposit_id: charlieDepositId,
				},
			});

			expect(charlieLockedAmount.success).toBe(true);
			if (charlieLockedAmount.success) {
				expect(charlieLockedAmount.value.response).toBe(charlieDepositAmount);
			}

			const checkpointNonce = lastRefundCheckpoint + 1n;
			const refunds = [{
				deposit_id: charlieDepositId,
				recipient: context.accounts.charlie.address,
				amount: charlieDepositAmount,
			}];

			const proposeTx = contract.send("propose_refunds", {
				origin: context.guardians[0].address,
				data: {
					refunds,
					checkpoint_nonce: checkpointNonce,
				},
			});
			await signAndFinalize(proposeTx, context.guardians[0].signer);

			const attestTx = contract.send("attest_refund", {
				origin: context.guardians[1].address,
				data: {
					deposit_id: charlieDepositId,
					approve: true,
				},
			});
			const finalized = await signAndFinalize(attestTx, context.guardians[1].signer);
			expectRefundedEvent(contract, finalized);

			const lockedAmount = await contract.query("get_locked_amount", {
				origin: context.accounts.alice.address,
				data: {
					deposit_id: charlieDepositId,
				},
			});
			expect(lockedAmount.success).toBe(true);
			if (lockedAmount.success) {
				expect(lockedAmount.value.response).toBeUndefined();
			}

			const deniedFlag = await contract.query("is_denied", {
				origin: context.accounts.alice.address,
				data: {
					deposit_id: charlieDepositId,
				},
			});
			expect(deniedFlag.success).toBe(true);
			if (deniedFlag.success) {
				expect(deniedFlag.value.response).toBe(true);
			}

			lastRefundCheckpoint = checkpointNonce;
		});

		it("rejects mismatched refund metadata", async () => {
			if (!bobDepositId) {
				throw new Error("Bob deposit not available for mismatch test");
			}

			const attempt = await contract.query("propose_refunds", {
				origin: context.guardians[0].address,
				data: {
					refunds: [{
						deposit_id: bobDepositId,
						recipient: context.accounts.charlie.address,
						amount: bobDepositAmount,
					}],
					checkpoint_nonce: lastRefundCheckpoint + 1n,
				},
			});

			expect(attempt.success).toBe(false);
			if (!attempt.success) {
				expect(attempt.value.type).toBe("FlagReverted");
			}
		});
	});

	describe("Expiry Paths", () => {
		it("expires stale releases and refunds after TTL", async () => {
			daveDepositAmount = MIN_DEPOSIT * 2n;
			eveDepositAmount = MIN_DEPOSIT * 4n;

			// Dave deposit for release expiry
			const daveLock = contract.send("lock", {
				origin: context.accounts.dave.address,
				data: {
					amount: daveDepositAmount,
					hotkey: daveHotkey.address,
					netuid,
				},
			});
			const daveFinalized = await signAndFinalize(daveLock, context.accounts.dave.signer);
			daveDepositId = toBinary(expectDepositEvent(contract, daveFinalized).deposit_id);

			// Eve deposit for refund expiry
			const eveLock = contract.send("lock", {
				origin: context.accounts.eve.address,
				data: {
					amount: eveDepositAmount,
					hotkey: eveHotkey.address,
					netuid,
				},
			});
			const eveFinalized = await signAndFinalize(eveLock, context.accounts.eve.signer);
			eveDepositId = toBinary(expectDepositEvent(contract, eveFinalized).deposit_id);

			// Pending release that will expire
			const expiringBurnId = randomHash("expire-release");
			const releaseCheckpoint = lastBurnCheckpoint + 1n;
			const releaseTx = contract.send("propose_releases", {
				origin: context.guardians[2].address,
				data: {
					burns: [{
						burn_id: expiringBurnId,
						recipient: context.accounts.dave.address,
						hotkey: daveHotkey.address,
						netuid,
						amount: daveDepositAmount,
					}],
					checkpoint_nonce: releaseCheckpoint,
				},
			});
			await signAndFinalize(releaseTx, context.guardians[2].signer);

			lastBurnCheckpoint = releaseCheckpoint;

			// Pending refund that will expire
			if (!eveDepositId) {
				throw new Error("Missing Eve deposit id");
			}

			const refundCheckpoint = lastRefundCheckpoint + 1n;
			const refundTx = contract.send("propose_refunds", {
				origin: context.guardians[1].address,
				data: {
					refunds: [{
						deposit_id: eveDepositId,
						recipient: context.accounts.eve.address,
						amount: eveDepositAmount,
					}],
					checkpoint_nonce: refundCheckpoint,
				},
			});
			await signAndFinalize(refundTx, context.guardians[1].signer);

			lastRefundCheckpoint = refundCheckpoint;

			await waitForBlocks(context.api, SIGNATURE_TTL_BLOCKS + 2);

			const expireReleaseTx = contract.send("expire_release", {
				origin: context.accounts.eve.address,
				data: {
					burn_id: expiringBurnId,
				},
			});
			const releaseExpired = await signAndFinalize(expireReleaseTx, context.accounts.eve.signer);
			const burnExpired = findContractEvent(contract, releaseExpired, "BurnExpired");
			expect(burnExpired).toBeDefined();

			const pendingRelease = await contract.query("has_pending_burn", {
				origin: context.accounts.alice.address,
				data: {
					burn_id: expiringBurnId,
				},
			});
			expect(pendingRelease.success).toBe(true);
			if (pendingRelease.success) {
				expect(pendingRelease.value.response).toBe(false);
			}

			const expireRefundTx = contract.send("expire_refund", {
				origin: context.accounts.eve.address,
				data: {
					deposit_id: eveDepositId,
				},
			});
			const refundExpired = await signAndFinalize(expireRefundTx, context.accounts.eve.signer);
			const refundExpiredEvent = findContractEvent(contract, refundExpired, "RefundExpired");
			expect(refundExpiredEvent).toBeDefined();

			const pendingRefund = await contract.query("has_pending_refund", {
				origin: context.accounts.alice.address,
				data: {
					deposit_id: eveDepositId,
				},
			});
			expect(pendingRefund.success).toBe(true);
			if (pendingRefund.success) {
				expect(pendingRefund.value.response).toBe(false);
			}
		}, 480000);
	});

	describe("Administrative Controls", () => {
		it("toggles pause state and blocks lock operations", async () => {
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

			const lockAttempt = await contract.query("lock", {
				origin: context.accounts.dave.address,
				data: {
					amount: MIN_DEPOSIT,
					hotkey: daveHotkey.address,
					netuid,
				},
			});
			expect(lockAttempt.success).toBe(false);
			if (!lockAttempt.success) {
				expect(lockAttempt.value.type).toBe("FlagReverted");
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
	});

	describe("Access Control Regression", () => {
		it("prevents non-guardians from proposing releases and refunds", async () => {
			const nonGuardian = context.accounts.eve;

			const releaseAttempt = await contract.query("propose_releases", {
				origin: nonGuardian.address,
				data: {
					burns: [{
						burn_id: randomHash("unauthorized-release"),
						recipient: nonGuardian.address,
						hotkey: eveHotkey.address,
						netuid,
						amount: MIN_DEPOSIT,
					}],
					checkpoint_nonce: lastBurnCheckpoint + 1n,
				},
			});
			expect(releaseAttempt.success).toBe(false);
			if (!releaseAttempt.success) {
				expect(releaseAttempt.value.type).toBe("FlagReverted");
			}

			const refundAttempt = await contract.query("propose_refunds", {
				origin: nonGuardian.address,
				data: {
					refunds: [{
						deposit_id: bobDepositId ?? randomHash("missing-deposit"),
						recipient: nonGuardian.address,
						amount: MIN_DEPOSIT,
					}],
					checkpoint_nonce: lastRefundCheckpoint + 1n,
				},
			});
			expect(refundAttempt.success).toBe(false);
			if (!refundAttempt.success) {
				expect(refundAttempt.value.type).toBe("FlagReverted");
			}
		});

		it("rejects duplicate guardian attestations before finalization", async () => {
			const checkpointNonce = lastBurnCheckpoint + 1n;
			const burnId = randomHash("duplicate-attest");

			const proposeTx = contract.send("propose_releases", {
				origin: context.guardians[0].address,
				data: {
					burns: [{
						burn_id: burnId,
						recipient: context.accounts.dave.address,
						hotkey: daveHotkey.address,
						netuid,
						amount: MIN_DEPOSIT,
					}],
					checkpoint_nonce: checkpointNonce,
				},
			});
			await signAndFinalize(proposeTx, context.guardians[0].signer);

			// Temporarily raise thresholds to keep burn pending
			const adjustTx = contract.send("set_guardians_and_thresholds", {
				origin: context.accounts.alice.address,
				data: {
					guardians: context.guardians.map((g) => g.address),
					approve_threshold: 3,
					deny_threshold: 2,
				},
			});
			await signAndFinalize(adjustTx, context.accounts.alice.signer);

			await signAndFinalize(contract.send("attest_release", {
				origin: context.guardians[1].address,
				data: {
					burn_id: burnId,
					approve: true,
				},
			}), context.guardians[1].signer);

			const secondAttest = await contract.query("attest_release", {
				origin: context.guardians[1].address,
				data: {
					burn_id: burnId,
					approve: true,
				},
			});
			expect(secondAttest.success).toBe(false);
			if (!secondAttest.success) {
				expect(secondAttest.value.type).toBe("FlagReverted");
			}

			// Clean up pending burn
			const expireTx = contract.send("expire_release", {
				origin: context.accounts.eve.address,
				data: {
					burn_id: burnId,
				},
			});
			await signAndFinalize(expireTx, context.accounts.eve.signer);

			// Restore thresholds
			const restoreTx = contract.send("set_guardians_and_thresholds", {
				origin: context.accounts.alice.address,
				data: {
					guardians: context.guardians.map((g) => g.address),
					approve_threshold: 2,
					deny_threshold: 1,
				},
			});
			await signAndFinalize(restoreTx, context.accounts.alice.signer);
		});
	});
});
