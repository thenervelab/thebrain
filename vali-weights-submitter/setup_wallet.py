#!/usr/bin/env python3
"""
Setup script to create a local wallet for weight submission
"""

import bittensor as bt
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
        # Use btcli command instead
        import subprocess
        subprocess.run([
            "btcli", "wallet", "new_coldkey", 
            "--wallet.name", "default",
            "--wallet.path", "~/.bittensor/wallets/"
        ], check=True)
        
        # Create hotkey
        print("Creating hotkey...")
        subprocess.run([
            "btcli", "wallet", "new_hotkey", 
            "--wallet.name", "default",
            "--wallet.hotkey", "default",
            "--wallet.path", "~/.bittensor/wallets/"
        ], check=True)
        
        print("✓ Wallet created successfully!")
        print("Wallet location: ~/.bittensor/wallets/default/")
        
        # Load and display wallet info
        wallet = bt.wallet(name="default", hotkey="default")
        print(f"Hotkey SS58 address: {wallet.hotkey.ss58_address}")
        
        return True
        
    except Exception as e:
        print(f"✗ Error creating wallet: {e}")
        return False

if __name__ == "__main__":
    setup_wallet()
