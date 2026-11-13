#!/usr/bin/env python3
"""
Helper script for mnemonic-based wallet operations
"""

import bittensor as bt
import yaml
import os

from wallet_utils import mnemonic_wallet

def load_config():
    with open("config.yaml", 'r') as file:
        return yaml.safe_load(file)

def create_wallet_from_mnemonic(mnemonic, path=None):
    """Create a wallet from mnemonic phrase and return wallet info.

    The key is derived with no derivation path (same as the weight submitter);
    `path` is accepted for backwards compatibility and ignored.
    """
    try:
        # Same in-memory wallet the weight submitter signs with
        wallet = mnemonic_wallet(mnemonic)

        print(f"✓ Wallet created successfully!")
        print(f"Hotkey SS58 address: {wallet.hotkey.ss58_address}")
        if path:
            print(f"Note: derivation path {path!r} is ignored; the key is derived without a path")

        return wallet
        
    except Exception as e:
        print(f"✗ Error creating wallet: {e}")
        return None

def test_mnemonic_wallet():
    """Test the mnemonic wallet configuration."""
    config = load_config()
    
    if not config['wallet'].get('use_mnemonic', False):
        print("Mnemonic wallet is not enabled in config.yaml")
        print("Set 'use_mnemonic: true' and provide your mnemonic phrase")
        return
    
    # Try to get mnemonic from environment variable first, then config
    mnemonic = os.getenv('BITTENSOR_MNEMONIC')
    if not mnemonic:
        mnemonic = config['wallet'].get('mnemonic')
    
    path = config['wallet'].get('mnemonic_path')
    
    if not mnemonic or mnemonic == "your twelve word mnemonic phrase here":
        print("Please set BITTENSOR_MNEMONIC environment variable or valid mnemonic phrase in config.yaml")
        return
    
    print("Testing mnemonic wallet...")
    wallet = create_wallet_from_mnemonic(mnemonic, path)
    
    if wallet:
        # Test connection to Bittensor
        try:
            # bittensor 11.x: blocking Subtensor, balances namespace
            subtensor = bt.Subtensor(network=config['bittensor']['chain_endpoint'])
            balance = subtensor.balances.get(wallet.hotkey.ss58_address)
            print(f"✓ Wallet balance: {balance} TAO")
            if hasattr(subtensor, "close"):
                subtensor.close()
        except Exception as e:
            print(f"✗ Error checking balance: {e}")

def generate_config_template():
    """Generate a template for mnemonic wallet configuration."""
    template = """
# Example mnemonic wallet configuration
wallet:
  use_mnemonic: true
  # mnemonic: "your twelve word mnemonic phrase here"  # Set via environment variable instead
  
  # Alternative: use local wallet files
  # use_mnemonic: false
  # name: "default"
  # hotkey: "default"
  # path: "~/.bittensor/wallets/"

# Environment variable setup:
# export BITTENSOR_MNEMONIC="your twelve word mnemonic phrase here"
"""
    print("Mnemonic wallet configuration template:")
    print(template)

if __name__ == "__main__":
    import sys
    
    if len(sys.argv) > 1:
        if sys.argv[1] == "test":
            test_mnemonic_wallet()
        elif sys.argv[1] == "template":
            generate_config_template()
        elif sys.argv[1] == "create":
            if len(sys.argv) > 2:
                mnemonic = sys.argv[2]
                path = sys.argv[3] if len(sys.argv) > 3 else None
                create_wallet_from_mnemonic(mnemonic, path)
            else:
                print("Usage: python mnemonic_wallet_helper.py create 'your mnemonic phrase' [path]")
    else:
        print("Usage:")
        print("  python mnemonic_wallet_helper.py test          # Test current config")
        print("  python mnemonic_wallet_helper.py template      # Show config template")
        print("  python mnemonic_wallet_helper.py create 'mnemonic' [path]  # Create wallet")
