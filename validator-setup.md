# Substrate Validator Setup

This guide provides detailed instructions for setting up and running a Substrate validator node on the Hippius testnet. Follow these steps carefully to ensure your node is configured correctly and participates in the network.

## Prerequisites

- A Linux-based server with sufficient resources.
- Rust toolchain installed. [Install Rust](https://www.rust-lang.org/tools/install)
- `git` and `cargo` installed.
- Basic knowledge of Substrate-based nodes and staking operations.

---

## Steps to Set Up a Validator Node

### 1. Clone the Repository

```bash
git clone <repository_url>
cd <repository_name>
```

### 2. Build the Node

Run the following commands to build the node:

```bash
cargo build --release
```

This will generate the executable binary located at `./target/release/hippius`.

### 3. Clean Up Old Data

Remove any existing chain data and database files to avoid conflicts:

```bash
rm -rf /root/.local/share/hippius/chains/hippius-testnet/db
rm -rf /root/.local/share/hippius/chains/hippius-testnet/frontier
```

### 4. Generate a Custom Specification File

Use the following command to generate a custom chain specification file:

```bash
rm -rf customSpec.json
./target/release/hippius build-spec --disable-default-bootnode --chain testnet > customSpec.json
```

### 5. Prepare the Keypair

Create a folder to store your validator keypair. Place the keypair files in this folder. For example:

```bash
mkdir /path/to/keypair
cp <keypair_files> /path/to/keypair/
```

Make sure to use the correct path to this folder when specifying the base path in the command to run the node.

### 6. Insert Keys

Use the `author.InsertKeys` RPC to insert your key pair. The key type ID for validators is `hips`:

```bash
# Example JSON-RPC payload
{
  "jsonrpc": "2.0",
  "method": "author_insertKey",
  "params": ["hips", "<mnemonic or seed>", "<public_key>"],
  "id": 1
}
```

This key pair will be used by validator nodes to sign transactions.

### 7. Run the Validator Node

Use the following command to start the validator node:

```bash
./target/release/hippius \
  --chain testnet \
  --validator \
  --base-path /path/to/keypair \
  --unsafe-rpc-external \
  --rpc-cors all \
  --telemetry-url "wss://telemetry.polkadot.io/submit/ 1" \
  --offchain-worker Always \
  --pruning=archive
```

- Replace `/path/to/keypair` with the actual path to your keypair folder.
- The node identity will be logged in the terminal when the node starts. Keep this identity handy for registration.

### 8. Register Your Validator

After the node is running, register it using the Registration Pallet:

1. **Substrate Node Identity:** Use the identity logged during the node startup.
2. **Local IPFS Node ID:** Provide the ID of your local IPFS node.
3. **Node Type:** Set this to `validator`.

### 9. Staking and Additional Configuration

Ensure that you:

- Bond sufficient funds for staking.
- Set your validator as active.
- Monitor the telemetry and logs to confirm the validator is operating correctly.

---

## Notes

- This node uses BABE for block production.
- Ensure proper security measures are in place for your keypair and server.
- For further details about the Hippius testnet, refer to the official documentation or contact the network administrators.

Happy validating!