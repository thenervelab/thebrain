#!/bin/bash
# Setup script for environment variables

echo "Setting up environment variables for Bittensor Weight Submitter..."

# Check if BITTENSOR_MNEMONIC is already set
if [ -n "$BITTENSOR_MNEMONIC" ]; then
    echo "✓ BITTENSOR_MNEMONIC is already set"
    echo "Current value: ${BITTENSOR_MNEMONIC:0:20}..."
else
    echo "BITTENSOR_MNEMONIC is not set"
    echo ""
    echo "To set your mnemonic phrase, run:"
    echo "export BITTENSOR_MNEMONIC=\"your twelve word mnemonic phrase here\""
    echo ""
    echo "Or add it to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    echo "echo 'export BITTENSOR_MNEMONIC=\"your twelve word mnemonic phrase here\"' >> ~/.bashrc"
    echo ""
fi

echo ""
echo "To test your setup, run:"
echo "python mnemonic_wallet_helper.py test"
echo ""
echo "To run the weight submitter:"
echo "python weight_submitter.py"
