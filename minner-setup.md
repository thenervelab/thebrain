# Hippius Substrate Node

This repository contains the necessary steps and commands to set up and run a Hippius Substrate-based blockchain node. It supports both validator and miner nodes. Follow the appropriate instructions based on your role.

## Prerequisites

Before starting, ensure you have the following installed on your machine:

1. **Rust Toolchain**
   - Install Rust via [rustup](https://rustup.rs/).
   - Ensure the nightly toolchain is installed: `rustup install nightly`
2. **Git**
   - Install Git from [git-scm.com](https://git-scm.com/).
3. **Cargo**
   - Comes with Rust installation. Ensure it’s up-to-date: `rustup update`
4. **A Key Pair**
   - Generate or securely acquire a key pair for the role you intend to perform (validator or miner).

## Steps for Miner Nodes

### 1. Clone the Repository and Build the Node

```bash
cd brain && git pull
cargo build --release
```

### 2. Download Custom Chain Specification

```bash
wget <url_to_get_customSpec_file>
```

### 3. Run the Miner Node

Run the node with the following command:

```bash
chmod 777 ./target/release/hippius
./target/release/hippius \
  --bootnodes /ip4/57.128.82.161/tcp/30333/p2p/<Node_Identity_of_Validator> \
  --chain customSpec.json \
  --offchain-worker Always
```

### 4. Insert Keys

Use the `author.InsertKeys` RPC to insert your key pair. The key type ID for miners is `hips`:

```bash
# Example JSON-RPC payload
{
  "jsonrpc": "2.0",
  "method": "author_insertKey",
  "params": ["hips", "<mnemonic or seed>", "<public_key>"],
  "id": 1
}
```

This key pair will be used by miner nodes to sign transactions.

### 5. Register Your Node

Register your node using the registration pallet by calling the appropriate extrinsic, signing account should be the one which is submited via rpc earlier. Provide the following information:

- **Node Identity:** This is logged on the console when the node starts.
- **IPFS Node ID:** Provide the IPFS node ID.
- **Node Type:** Specify the type as `miner`.

---

## Additional Notes

- **Telemetry:** Ensure telemetry is correctly configured to monitor node performance.
- **Runtime Parameters:** Review the runtime parameters for your specific role.

## License

This project is licensed under the [MIT License](LICENSE).

## Contributing

Feel free to open issues or pull requests to improve this repository.

