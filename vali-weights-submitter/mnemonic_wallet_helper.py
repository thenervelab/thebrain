#!/usr/bin/env python3
"""
Helper script for mnemonic-based wallet operations
"""

import bittensor as bt
import yaml
import os

def load_config():
    with open("config.yaml", 'r') as file:
        return yaml.safe_load(file)

def create_wallet_from_mnemonic(mnemonic, path="m/44'/354'/0'/0'/0'"):
    """Create a wallet from mnemonic phrase and return wallet info."""
    try:
        # Create keypair from mnemonic using substrate-interface
        from substrateinterface import Keypair
        keypair = Keypair.create_from_mnemonic(mnemonic, ss58_format=42)
        
        # Create a minimal wallet-like object
        class MnemonicWallet:
            def __init__(self, keypair):
                self.hotkey = type('Hotkey', (), {
                    'ss58_address': keypair.ss58_address,
                    'keypair': keypair
                })()
                self.keypair = keypair
                self.coldkey = type('Coldkey', (), {
                    'ss58_address': keypair.ss58_address,
                    'keypair': keypair
                })()
        
        wallet = MnemonicWallet(keypair)
        
        print(f"✓ Wallet created successfully!")
        print(f"Hotkey SS58 address: {wallet.hotkey.ss58_address}")
        print(f"Coldkey SS58 address: {wallet.coldkey.ss58_address}")
        print(f"Derivation path: {path}")
        
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
    
    path = config['wallet'].get('mnemonic_path', "m/44'/354'/0'/0'/0'")
    
    if not mnemonic or mnemonic == "your twelve word mnemonic phrase here":
        print("Please set BITTENSOR_MNEMONIC environment variable or valid mnemonic phrase in config.yaml")
        return
    
    print("Testing mnemonic wallet...")
    wallet = create_wallet_from_mnemonic(mnemonic, path)
    
    if wallet:
        # Test connection to Bittensor
        try:
            subtensor = bt.subtensor(network=config['bittensor']['chain_endpoint'])
            balance = subtensor.get_balance(wallet.hotkey.ss58_address)
            print(f"✓ Wallet balance: {balance} TAO")
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
  mnemonic_path: "m/44'/354'/0'/0'/0'"
  
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
                path = sys.argv[3] if len(sys.argv) > 3 else "m/44'/354'/0'/0'/0'"
                create_wallet_from_mnemonic(mnemonic, path)
            else:
                print("Usage: python mnemonic_wallet_helper.py create 'your mnemonic phrase' [path]")
    else:
        print("Usage:")
        print("  python mnemonic_wallet_helper.py test          # Test current config")
        print("  python mnemonic_wallet_helper.py template      # Show config template")
        print("  python mnemonic_wallet_helper.py create 'mnemonic' [path]  # Create wallet")
