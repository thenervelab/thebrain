import { type TypedApi } from "polkadot-api";
import { devnet } from "@polkadot-api/descriptors";
import { type PolkadotSigner } from "polkadot-api/signer";

export async function getStakeBalance(
    api: TypedApi<typeof devnet>,
    hotkey: string,
    netuid: number,
    coldkey: string
): Promise<bigint> {
    try {
        const stakeInfo = await api.apis.StakeInfoRuntimeApi.get_stake_info_for_hotkey_coldkey_netuid(
            hotkey,
            coldkey,
            netuid
        );

        if (stakeInfo) {
            return stakeInfo.stake;
        }

        return 0n;
    } catch (error) {
        console.error(`Error getting stake balance for ${hotkey}/${coldkey} on netuid ${netuid}:`, error);
        return 0n;
    }
}

export async function transferStake(
    api: TypedApi<typeof devnet>,
    destinationColdkey: string,
    hotkey: string,
    netuid: number,
    amount: bigint,
    signer: PolkadotSigner
): Promise<void> {
    const tx = api.tx.SubtensorModule.transfer_stake({
        destination_coldkey: destinationColdkey,
        hotkey,
        origin_netuid: netuid,
        destination_netuid: netuid,
        alpha_amount: amount,
    });

    await tx.signAndSubmit(signer);
}

export async function moveStake(
    api: TypedApi<typeof devnet>,
    originHotkey: string,
    destinationHotkey: string,
    netuid: number,
    amount: bigint,
    signer: PolkadotSigner
): Promise<void> {
    const tx = api.tx.SubtensorModule.move_stake({
        origin_hotkey: originHotkey,
        destination_hotkey: destinationHotkey,
        origin_netuid: netuid,
        destination_netuid: netuid,
        alpha_amount: amount,
    });

    await tx.signAndSubmit(signer);
}

export async function lockStake(
    api: TypedApi<typeof devnet>,
    hotkey: string,
    netuid: number,
    amount: bigint,
    signer: PolkadotSigner
): Promise<void> {
    const lockStakeTx = (api.tx.SubtensorModule as any).lock_stake;
    if (!lockStakeTx) {
        throw new Error("SubtensorModule.lock_stake is unavailable; regenerate descriptors against a Subtensor localnet");
    }

    const tx = lockStakeTx({
        hotkey,
        netuid,
        amount,
    });

    await tx.signAndSubmit(signer);
}

export type StakeAvailability = {
    total: bigint;
    locked: bigint;
    available: bigint;
};

function getMapValue(mapLike: unknown, key: string | number): unknown {
    if (mapLike instanceof Map) {
        return mapLike.get(key);
    }

    if (Array.isArray(mapLike)) {
        const match = mapLike.find((entry) => {
            if (!Array.isArray(entry) || entry.length < 2) {
                return false;
            }
            return String(entry[0]) === String(key);
        });
        return match?.[1];
    }

    if (mapLike && typeof mapLike === "object") {
        return (mapLike as Record<string, unknown>)[String(key)];
    }

    return undefined;
}

export async function getStakeAvailability(
    api: TypedApi<typeof devnet>,
    coldkey: string,
    netuid: number
): Promise<StakeAvailability> {
    const runtimeApi = (api.apis.StakeInfoRuntimeApi as any).get_stake_availability_for_coldkeys;
    if (!runtimeApi) {
        throw new Error("StakeInfoRuntimeApi.get_stake_availability_for_coldkeys is unavailable; regenerate descriptors against a Subtensor localnet");
    }

    const response = await runtimeApi([coldkey], [netuid]);
    const coldkeyAvailability = getMapValue(response, coldkey);
    const subnetAvailability = getMapValue(coldkeyAvailability, netuid) as Partial<StakeAvailability> | undefined;

    return {
        total: BigInt(subnetAvailability?.total ?? 0),
        locked: BigInt(subnetAvailability?.locked ?? 0),
        available: BigInt(subnetAvailability?.available ?? 0),
    };
}

export function formatStakeAmount(amount: bigint): string {
    const alpha = Number(amount) / 1_000_000_000;
    return `${alpha.toFixed(4)} Alpha`;
}
