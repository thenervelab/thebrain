# Bittensor Weight Submitter for Subnet 75

This Python script fetches rankings from the Hippius chain and submits corresponding weights to Bittensor subnet 75. It implements a complete workflow for automated weight submission based on external ranking data.

## Features

- **Configurable**: All parameters are easily configurable via `config.yaml`
- **Multi-step Process**: Implements all 4 required steps:
  1. Fetch metagraph for subnet 75 and get active hotkeys with UIDs
  2. Fetch ranking data from Hippius chain
  3. Match SS58 addresses from rankings with Bittensor UIDs
  4. Submit weights through the bittensor SDK's `SetWeights` intent

## Installation

1. Clone this repository:
```bash
git clone <repository-url>
cd vali-weights-submitter
```

2. Install dependencies (requires Python 3.10+; installs bittensor SDK 11.x, 11.3.2 or newer):
```bash
pip install -r requirements.txt
```

3. Configure your wallet:
```bash
# Option 1: Create a new wallet (if using local wallet)
btcli wallet new-coldkey --wallet default
btcli wallet new-hotkey --wallet default --wallet-hotkey default

# Option 2: Use mnemonic phrase (recommended)
# Set use_mnemonic: true in config.yaml and export BITTENSOR_MNEMONIC

# Optional: pin the expected hotkey (edit config.yaml)
# Set use_ss58_address: true and ss58_address to your validator hotkey; startup fails on a mismatch
```

## Configuration

Edit `config.yaml` to customize the behavior:

### Bittensor Configuration
- `subnet_id`: Target subnet (default: 75)
- `chain_endpoint`: The single Bittensor endpoint used for block polling, metagraph reads and weight submission. A network name (`finney`, `test`, `local`) or a websocket URL such as `ws://127.0.0.1:9945`

### Hippius Chain Configuration
- `rpc_endpoint`: Hippius chain RPC endpoint
- `ranking_storage_key`: Storage key for rankings

### Wallet Configuration
- `name`: Wallet name (for local wallet)
- `hotkey`: Hotkey name (for local wallet)
- `path`: Wallet path (for local wallet)
- `use_mnemonic`: `true` derives the signing hotkey from the `BITTENSOR_MNEMONIC` environment variable; `false` loads the local wallet above
- `use_ss58_address`: Optional safety check. When `true`, startup fails unless the signing hotkey equals `ss58_address`
- `ss58_address`: Expected validator hotkey SS58 address (when `use_ss58_address` is `true`)
- The mnemonic is derived without a derivation path; `mnemonic_path` is not used

### Weight Submission Configuration
- `version_key`: Version key for validator
- `wait_for_inclusion`: Wait for transaction inclusion
- `wait_for_finalization`: Wait for transaction finalization
- `period`: Transaction validity period (blocks)
- `retries`: Resubmissions on transient tx-pool rejections (default: 2)
- `window_retries`: For `run_continuous.py`, extra attempts within one submission window after a failure, one block poll apart (default: 5). Covers e.g. the rate-limit preflight ("retry in ~1 blocks") or a Hippius RPC hiccup without losing the whole window
- `block_interval`: Blocks between submissions for `run_continuous.py` (default: 101; keep above the subnet's weights rate limit)
- `block_check_interval`: Seconds between block polls (default: 12)
- `use_modulo_timing`: `true` aligns submissions to multiples of `block_interval`; `false` waits `block_interval` blocks after the previous submission

## Usage

### Basic Usage
```bash
python weight_submitter.py
```

### With Custom Config
```bash
python weight_submitter.py --config custom_config.yaml
```

The process exits non-zero when the submission fails.

### Dry Run
Build and preflight the submission (registration, rate limit, weight conformance) without signing or submitting anything:
```bash
python weight_submitter.py --dry-run
```

### Continuous Operation
Submit every `block_interval` blocks:
```bash
python run_continuous.py --config config.yaml
```

### Pinning the Expected Hotkey
To make sure the script signs with the right validator hotkey, whichever signing method is used:

1. Edit `config.yaml`:
```yaml
wallet:
  use_ss58_address: true
  ss58_address: "5F...your_validator_hotkey_ss58..."
```

2. Run the script. It refuses to start if the signing hotkey differs from `ss58_address`:
```bash
python weight_submitter.py
```

To create a local wallet instead of using a mnemonic, run `python setup_wallet.py` and set `use_mnemonic: false`.

### Using Mnemonic Wallet
To use a mnemonic phrase for wallet creation (recommended for production):

1. Edit `config.yaml`:
```yaml
wallet:
  use_mnemonic: true
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
Submits through the bittensor SDK's `SetWeights` intent (`subtensor.execute(bt.SetWeights(...), wallet)`), signed by the hotkey. The SDK preflights registration and the rate limit, and uses plaintext or commit-reveal submission depending on the subnet's settings.

## Weight Calculation

1. Extracts ranking scores from Hippius data
2. Matches with Bittensor UIDs
3. Normalizes the matched weights to sum to 1.0
4. Submits them with `SetWeights`, which applies the subnet's max-weight clip and the u16 quantization required by the protocol

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

- `bittensor` (11.x, 11.3.2 or newer): Bittensor SDK for metagraph reads and weight submission
- `substrate-interface`: Hippius chain interaction
- `pyyaml`: Configuration file parsing
- `certifi`: CA certificates for TLS connections

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
