"""Signing-wallet helpers shared by the weight-submitter scripts (bittensor SDK 11.x)."""

import os
from types import SimpleNamespace
from typing import Any, Tuple

import bittensor as bt
from bittensor.sp_core import Keypair

MNEMONIC_ENV_VAR = "BITTENSOR_MNEMONIC"

# Values shipped in example configs that must never be accepted as a real address.
_SS58_PLACEHOLDERS = {
    "",
    "YOUR VALI SS58 ADDRESS",
    "5F...your_validator_ss58_address_here...",
}


def keypair_from_mnemonic(mnemonic: str) -> Keypair:
    """sr25519 keypair for a mnemonic, with no derivation path.

    Derives the same address as substrate-interface's
    ``Keypair.create_from_mnemonic(mnemonic, ss58_format=42)``, which earlier
    versions of these scripts used, so existing operators keep their hotkey.
    """
    return Keypair.create_from_mnemonic(mnemonic.strip(), bt.CRYPTO_SR25519)


def mnemonic_wallet(mnemonic: str) -> Any:
    """An in-memory, wallet-shaped object (``bt.KeyedWallet``) for one keypair.

    The SDK accepts it anywhere it accepts a ``bt.Wallet``; SetWeights signs
    with ``.hotkey``.
    """
    keypair = keypair_from_mnemonic(mnemonic)
    return SimpleNamespace(coldkey=keypair, coldkeypub=keypair, hotkey=keypair)


def load_signing_wallet(wallet_cfg: dict) -> Tuple[Any, str]:
    """Build the wallet used to sign weight submissions from the ``wallet`` config section.

    ``use_mnemonic: true`` derives the hotkey from the BITTENSOR_MNEMONIC env var;
    otherwise the on-disk btcli wallet (``name`` / ``hotkey`` / ``path``) is loaded.
    When ``use_ss58_address`` is true, the resolved hotkey must equal ``ss58_address``.

    Returns ``(wallet, hotkey_ss58)``.
    """
    if wallet_cfg.get("use_mnemonic", False):
        mnemonic = os.getenv(MNEMONIC_ENV_VAR)
        if not mnemonic:
            raise ValueError(
                f"{MNEMONIC_ENV_VAR} environment variable is required when wallet.use_mnemonic is true"
            )
        wallet = mnemonic_wallet(mnemonic)
    else:
        wallet = bt.Wallet(
            name=wallet_cfg.get("name", "default"),
            hotkey=wallet_cfg.get("hotkey", "default"),
            path=wallet_cfg.get("path", "~/.bittensor/wallets/"),
        )

    hotkey_ss58 = wallet.hotkey.ss58_address

    if wallet_cfg.get("use_ss58_address", False):
        expected = str(wallet_cfg.get("ss58_address") or "").strip()
        if expected in _SS58_PLACEHOLDERS:
            raise ValueError(
                "wallet.use_ss58_address is true but wallet.ss58_address is not set to a real address"
            )
        if expected != hotkey_ss58:
            raise ValueError(
                f"Signing hotkey {hotkey_ss58} does not match wallet.ss58_address {expected}"
            )

    return wallet, hotkey_ss58
