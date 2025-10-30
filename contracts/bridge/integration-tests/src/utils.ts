import { Enum, type TypedApi } from "polkadot-api";
import { devnet, MultiAddress } from "@polkadot-api/descriptors";
import { sr25519CreateDerive } from "@polkadot-labs/hdkd";
import { DEV_PHRASE, entropyToMiniSecret, mnemonicToEntropy, ss58Address, type KeyPair } from "@polkadot-labs/hdkd-helpers";
import { getPolkadotSigner, type PolkadotSigner } from "polkadot-api/signer";
import { randomBytes } from "crypto";
import type { ContractSdk } from "./setup";
import type { TxFinalized } from "polkadot-api";

export const TEST_CONFIG = {
	wsUrl: process.env.CONTRACTS_NODE_URL || "ws://127.0.0.1:9944",
	ss58Prefix: 42,
	timeout: 60000,
	blockTime: 4000,
};

export type Wallet = {
	address: string;
	signer: PolkadotSigner;
};

function getWalletFromKeypair(keypair: KeyPair): Wallet {
	const signer = getPolkadotSigner(
		keypair.publicKey,
		"Sr25519",
		keypair.sign
	);
	const address = ss58Address(keypair.publicKey, TEST_CONFIG.ss58Prefix);

	return { address, signer };
}

function getRandomKeypair(): KeyPair {
	const seed = randomBytes(32);
	const miniSecret = entropyToMiniSecret(seed);
	const derive = sr25519CreateDerive(miniSecret);
	return derive("");
}

export const getRandomWallet = (): Wallet => getWalletFromKeypair(getRandomKeypair());

export function createHotkey(coldkeyDerivePath: string): Wallet {
	const hotkeyDerivePath = `${coldkeyDerivePath}//bridge-hotkey`;
	const entropy = mnemonicToEntropy(DEV_PHRASE);
	const miniSecret = entropyToMiniSecret(entropy);
	const derive = sr25519CreateDerive(miniSecret);
	const keypair = derive(hotkeyDerivePath);

	const signer = getPolkadotSigner(
		keypair.publicKey,
		"Sr25519",
		keypair.sign
	);

	const address = ss58Address(keypair.publicKey, TEST_CONFIG.ss58Prefix);

	return { address, signer };
}

export function taoToRao(tao: number): bigint {
	return BigInt(Math.floor(tao * 1_000_000_000));
}

export function raoToTao(rao: bigint): number {
	return Number(rao) / 1_000_000_000;
}

export async function waitForBlocks(api: TypedApi<typeof devnet>, blocks: number): Promise<void> {
	const startBlock = await api.query.System.Number.getValue();
	let currentBlock = startBlock;

	while (Number(currentBlock) < Number(startBlock) + blocks) {
		await new Promise((resolve) => setTimeout(resolve, TEST_CONFIG.blockTime));
		currentBlock = await api.query.System.Number.getValue();
	}
}

export async function fundAccount(
	api: TypedApi<typeof devnet>,
	recipientAddress: string,
	amount: bigint,
	fundingSigner: PolkadotSigner
): Promise<void> {
	const tx = api.tx.Balances.transfer_keep_alive({
		dest: MultiAddress.Id(recipientAddress),
		value: amount,
	});

	await tx.signAndSubmit(fundingSigner);
}

export async function getBalance(
	api: TypedApi<typeof devnet>,
	address: string
): Promise<bigint> {
	const account = await api.query.System.Account.getValue(address);
	return account.data.free;
}

export async function registerSubnet(
	api: TypedApi<typeof devnet>,
	hotkey: string,
	signer: PolkadotSigner
): Promise<number> {
	const tx = api.tx.SubtensorModule.register_network({
		hotkey,
	});
	await tx.signAndSubmit(signer);

	const afterNetworks = await api.query.SubtensorModule.TotalNetworks.getValue() || 0;
	const netuid = afterNetworks - 1;
	console.log(`Registered subnet with netuid ${netuid}`);

	await startSubnet(api, netuid, signer);
	return netuid;
}

async function startSubnet(
	api: TypedApi<typeof devnet>,
	netuid: number,
	signer: PolkadotSigner
): Promise<void> {
	const registerBlock = await api.query.SubtensorModule.NetworkRegisteredAt.getValue(netuid);
	if (!registerBlock) {
		console.log(`Could not determine registration block for netuid ${netuid}`);
		return;
	}

	const duration = await api.constants.SubtensorModule.DurationOfStartCall();
	const durationNumber = Number(duration);

	let currentBlock = await api.query.System.Number.getValue();
	while (Number(currentBlock) - Number(registerBlock) <= durationNumber) {
		await new Promise((resolve) => setTimeout(resolve, 2000));
		currentBlock = await api.query.System.Number.getValue();
	}

	const tx = api.tx.SubtensorModule.start_call({
		netuid,
	});
	await tx.signAndSubmit(signer);
	console.log(`Started subnet ${netuid}`);
}

export async function registerValidator(
	api: TypedApi<typeof devnet>,
	netuid: number,
	hotkey: string,
	coldkeySigner: PolkadotSigner,
	stakeAmount: bigint
): Promise<void> {
	const registerTx = api.tx.SubtensorModule.burned_register({
		netuid,
		hotkey,
	});

	await registerTx.signAndSubmit(coldkeySigner);
	console.log(`Registered validator ${hotkey.slice(0, 10)}... on subnet ${netuid}`);

	if (stakeAmount > 0n) {
		const stakeTx = api.tx.SubtensorModule.add_stake({
			hotkey,
			netuid,
			amount_staked: stakeAmount,
		});
		await stakeTx.signAndSubmit(coldkeySigner);
		console.log(`Staked ${raoToTao(stakeAmount)} Alpha on subnet ${netuid}`);
	}
}

export async function elevateRegistrationLimits(
	api: TypedApi<typeof devnet>,
	netuid: number,
	targetPerInterval: number,
	maxPerBlock: number,
	sudoSigner: PolkadotSigner
): Promise<void> {
	console.log(`Elevating registration limits for netuid ${netuid}`);
	const setFreezeWindow = api.tx.AdminUtils.sudo_set_admin_freeze_window({ window: 0 });
	await api.tx.Sudo.sudo({ call: setFreezeWindow.decodedCall }).signAndSubmit(sudoSigner);

	const innerTarget = api.tx.AdminUtils.sudo_set_target_registrations_per_interval({
		netuid,
		target_registrations_per_interval: targetPerInterval,
	});
	await api.tx.Sudo.sudo({ call: innerTarget.decodedCall }).signAndSubmit(sudoSigner);

	const innerBlock = api.tx.AdminUtils.sudo_set_max_registrations_per_block({
		netuid,
		max_registrations_per_block: maxPerBlock,
	});
	await api.tx.Sudo.sudo({ call: innerBlock.decodedCall }).signAndSubmit(sudoSigner);
}

export async function addContractAsProxy(
	api: TypedApi<typeof devnet>,
	contractAddress: string,
	signer: PolkadotSigner
): Promise<void> {
	const tx = api.tx.Proxy.add_proxy({
		delegate: MultiAddress.Id(contractAddress),
		proxy_type: Enum("Any"),
		delay: 0,
	});

	await tx.signAndSubmit(signer);
}

export async function hasProxyPermission(
	api: TypedApi<typeof devnet>,
	delegator: string,
	delegate: string
): Promise<boolean> {
	try {
		const proxies = await api.query.Proxy.Proxies.getValue(delegator);
		if (!proxies || !proxies[0]) {
			return false;
		}

		return proxies[0].some((proxy) => {
			return proxy.delegate === delegate &&
				(proxy.proxy_type.type === "Staking" || proxy.proxy_type.type === "Any");
		});
	} catch (error) {
		console.error("Error checking proxy permissions:", error);
		return false;
	}
}

export async function seedBridgeStake(
	api: TypedApi<typeof devnet>,
	netuid: number,
	sourceSigner: PolkadotSigner,
	destinationColdkey: string,
	hotkey: string,
	amount: bigint
): Promise<void> {
	const tx = api.tx.SubtensorModule.transfer_stake({
		destination_coldkey: destinationColdkey,
		hotkey,
		origin_netuid: netuid,
		destination_netuid: netuid,
		alpha_amount: amount,
	});

	await tx.signAndSubmit(sourceSigner);
	console.log(`Seeded ${raoToTao(amount)} Alpha to ${destinationColdkey.slice(0, 10)}... via hotkey ${hotkey.slice(0, 10)}...`);
}

interface ContractEvent<T = unknown> {
	type: string;
	value: T;
}

export function findContractEvent<T>(
	contract: ReturnType<ContractSdk["getContract"]>,
	finalized: TxFinalized,
	eventName: string
): T | undefined {
	const decoded = contract.filterEvents(finalized.events);
	const matchedEvent = decoded.find((event: ContractEvent): event is ContractEvent<T> => event.type === eventName);
	return matchedEvent?.value as T | undefined;
}

export interface DepositMadeEvent {
	chain_id: number;
	escrow_contract: string;
	deposit_nonce: bigint;
	sender: string;
	amount: bigint;
	deposit_id: string | Uint8Array;
}

export interface ReleasedEvent {
	burn_id: string | Uint8Array;
	recipient: string;
	amount: bigint;
}

export interface RefundedEvent {
	deposit_id: string | Uint8Array;
	recipient: string;
	amount: bigint;
}

export function expectDepositEvent(
	contract: ReturnType<ContractSdk["getContract"]>,
	finalized: TxFinalized
): DepositMadeEvent {
	const event = findContractEvent<DepositMadeEvent>(contract, finalized, "DepositMade");
	if (!event) {
		throw new Error("DepositMade event not emitted");
	}
	return event;
}

export function expectReleasedEvent(
	contract: ReturnType<ContractSdk["getContract"]>,
	finalized: TxFinalized
): ReleasedEvent {
	const event = findContractEvent<ReleasedEvent>(contract, finalized, "Released");
	if (!event) {
		throw new Error("Released event not emitted");
	}
	return event;
}

export function expectRefundedEvent(
	contract: ReturnType<ContractSdk["getContract"]>,
	finalized: TxFinalized
): RefundedEvent {
	const event = findContractEvent<RefundedEvent>(contract, finalized, "Refunded");
	if (!event) {
		throw new Error("Refunded event not emitted");
	}
	return event;
}

export function formatAddress(address: string): string {
	if (address.length <= 16) {
		return address;
	}
	return `${address.slice(0, 8)}...${address.slice(-8)}`;
}
