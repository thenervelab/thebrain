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

export function formatStakeAmount(amount: bigint): string {
    const alpha = Number(amount) / 1_000_000_000;
    return `${alpha.toFixed(4)} Alpha`;
}
