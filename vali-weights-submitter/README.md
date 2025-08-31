# Bittensor Weight Submitter for Subnet 75

This Python script fetches rankings from the Hippius chain and submits corresponding weights to Bittensor subnet 75. It implements a complete workflow for automated weight submission based on external ranking data.

## Features

- **Configurable**: All parameters are easily configurable via `config.yaml`
- **Multi-step Process**: Implements all 4 required steps:
  1. Fetch metagraph for subnet 75 and get active hotkeys with UIDs
  2. Fetch ranking data from Hippius chain
  3. Match SS58 addresses from rankings with Bittensor UIDs
  4. Submit weights using `bittensor.core.extrinsics.set_weights.set_weights_extrinsic`

## Installation

1. Clone this repository:
```bash
git clone <repository-url>
cd vali-weights-submitter
```

2. Install dependencies:
```bash
pip install -r requirements.txt
```

3. Configure your wallet:
```bash
# Option 1: Create a new wallet (if using local wallet)
btcli wallet new_coldkey --wallet.name default
btcli wallet new_hotkey --wallet.name default --wallet.hotkey default

# Option 2: Use mnemonic phrase (recommended)
# Edit config.yaml and set use_mnemonic: true with your mnemonic phrase

# Option 3: Use SS58 address for querying (edit config.yaml)
# Set use_ss58_address: true and provide your validator SS58 address
```

## Configuration

Edit `config.yaml` to customize the behavior:

### Bittensor Configuration
- `subnet_id`: Target subnet (default: 75)
- `rpc_endpoint`: Bittensor RPC endpoint
- `chain_endpoint`: Bittensor chain endpoint

### Hippius Chain Configuration
- `rpc_endpoint`: Hippius chain RPC endpoint
- `ranking_storage_key`: Storage key for rankings

### Wallet Configuration
- `name`: Wallet name (for local wallet)
- `hotkey`: Hotkey name (for local wallet)
- `path`: Wallet path (for local wallet)
- `use_ss58_address`: Set to `true` to use SS58 address for querying
- `ss58_address`: Validator SS58 address (when `use_ss58_address` is `true`)
- `use_mnemonic`: Set to `true` to use mnemonic phrase for wallet creation
- `mnemonic`: Your 12-word mnemonic phrase (set via `BITTENSOR_MNEMONIC` environment variable)
- `mnemonic_path`: Bittensor derivation path (default: `m/44'/354'/0'/0'/0'`)

### Weight Submission Configuration
- `version_key`: Version key for validator
- `wait_for_inclusion`: Wait for transaction inclusion
- `wait_for_finalization`: Wait for transaction finalization
- `period`: Transaction validity period

## Usage

### Basic Usage
```bash
python weight_submitter.py
```

### With Custom Config
```bash
python weight_submitter.py --config custom_config.yaml
```

### Using SS58 Address
To use a validator's SS58 address for querying (wallet still required for submission):

1. Edit `config.yaml`:
```yaml
wallet:
  use_ss58_address: true
  ss58_address: "5F...your_validator_ss58_address_here..."
```

2. Create a local wallet for submission:
```bash
python setup_wallet.py
```

3. Run the script:
```bash
python weight_submitter.py
```

**Note**: A local wallet is always required for weight submission due to Bittensor's security requirements. The SS58 address is used for querying and balance checking.

### Using Mnemonic Wallet
To use a mnemonic phrase for wallet creation (recommended for production):

1. Edit `config.yaml`:
```yaml
wallet:
  use_mnemonic: true
  # mnemonic: "your twelve word mnemonic phrase here"  # Set via environment variable instead
  mnemonic_path: "m/44'/354'/0'/0'/0'"
```

2. Set your mnemonic as environment variable:
```bash
export BITTENSOR_MNEMONIC="your twelve word mnemonic phrase here"
```

3. Test your mnemonic wallet:
```bash
python mnemonic_wallet_helper.py test
```

4. Run the script:
```bash
python weight_submitter.py
```

**Security Note**: Store your mnemonic phrase securely and never share it. Using environment variables is the recommended approach for production deployments.

## How It Works

### Step 1: Fetch Metagraph
The script connects to Bittensor and fetches the metagraph for subnet 75, extracting all active neurons and their corresponding UIDs and hotkey addresses.

### Step 2: Fetch Hippius Rankings
Connects to the Hippius chain and queries the `rankingStorage.rankedList` storage to get the current rankings.

### Step 3: Match Rankings with UIDs
Matches SS58 addresses from the Hippius rankings with the UIDs from the Bittensor metagraph. The script handles different possible field names in the ranking data.

### Step 4: Submit Weights
Uses the `set_weights_extrinsic` function from the Bittensor SDK to submit the calculated weights to the network.

## Weight Calculation

The script normalizes the weights to u16 range (0-65535) for each individual weight:
1. Extracts ranking scores from Hippius data
2. Matches with Bittensor UIDs
3. Normalizes weights to u16 integers by scaling to 65535
4. Submits u16 weights to Bittensor (required by protocol)

**Note**: Each individual weight is submitted as a u16 integer (0-65535) as required by the Bittensor protocol.

## Logging

The script provides comprehensive logging with configurable levels:
- `INFO`: General progress information
- `DEBUG`: Detailed matching information
- `WARNING`: Non-critical issues
- `ERROR`: Critical errors

## Error Handling

The script includes robust error handling:
- Network connection failures
- Invalid configuration
- Missing or malformed data
- Transaction submission failures

## Security Considerations

- Store your wallet securely
- Use environment variables for sensitive data
- Regularly update dependencies
- Monitor transaction logs

## Troubleshooting

### Common Issues

1. **Connection Errors**: Check RPC endpoints in config
2. **Wallet Issues**: Ensure wallet is properly configured
3. **No Rankings Found**: Verify Hippius chain connectivity
4. **Weight Submission Fails**: Check wallet balance and permissions

### Debug Mode
Enable debug logging by changing the log level in `config.yaml`:
```yaml
logging:
  level: "DEBUG"
```

## Dependencies

- `bittensor`: Bittensor SDK for network interaction
- `substrate-interface`: Substrate chain interaction
- `pyyaml`: Configuration file parsing
- `numpy`: Numerical operations
- `asyncio`: Asynchronous programming support

## License

This project is licensed under the MIT License.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## Support

For issues and questions:
1. Check the troubleshooting section
2. Review the logs for error messages
3. Open an issue on GitHub
