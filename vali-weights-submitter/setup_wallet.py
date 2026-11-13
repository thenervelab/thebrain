#!/usr/bin/env python3
"""
Setup script to create a local wallet for weight submission
"""

from bittensor.wallet import Wallet
import os

def setup_wallet():
    print("Setting up local wallet for weight submission...")
    
    try:
        # Check if wallet already exists
        wallet_path = os.path.expanduser("~/.bittensor/wallets/default/hotkeys/default")
        if os.path.exists(wallet_path):
            print("✓ Wallet already exists at:", wallet_path)
            return True
        
        print("Creating new wallet...")
        
        # Create coldkey
        print("Creating coldkey...")
        # Use btcli (bundled with bittensor 11.x) instead
        import subprocess
        wallets_path = os.path.expanduser("~/.bittensor/wallets/")
        subprocess.run([
            "btcli", "wallet", "new-coldkey",
            "--wallet", "default",
            "--wallet-path", wallets_path
        ], check=True)

        # Create hotkey
        print("Creating hotkey...")
        subprocess.run([
            "btcli", "wallet", "new-hotkey",
            "--wallet", "default",
            "--wallet-hotkey", "default",
            "--wallet-path", wallets_path
        ], check=True)
        
        print("✓ Wallet created successfully!")
        print("Wallet location: ~/.bittensor/wallets/default/")
        
        # Load and display wallet info
        wallet = Wallet(name="default", hotkey="default")
        print(f"Hotkey SS58 address: {wallet.hotkey.ss58_address}")
        
        return True
        
    except Exception as e:
        print(f"✗ Error creating wallet: {e}")
        return False

if __name__ == "__main__":
    setup_wallet()
