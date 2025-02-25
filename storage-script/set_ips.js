// Import required libraries
const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");
const dotenv = require("dotenv");

async function addAvailableIp(api, seedPhrase, ip) {
    try {
        // Create a keyring instance
        const keyring = new Keyring({ type: "sr25519" });

        // Use the seed phrase to create the Sudo account keypair
        const sudoKeyPair = keyring.addFromUri(seedPhrase);

        console.log(`Sudo account address: ${sudoKeyPair.address}`);

        // Check if the provided account is the Sudo account
        const sudoKey = (await api.query.sudo.key()).toString();
        if (sudoKey !== sudoKeyPair.address) {
            throw new Error("The provided account is not the Sudo account");
        }

        // Convert IP to little-endian byte array
        const ipBytes = new Uint8Array([
            ip & 0xff,
            (ip >> 8) & 0xff,
            (ip >> 16) & 0xff,
            (ip >> 24) & 0xff
        ]);

        console.log(`Calling add_available_ip with sudo for IP: ${Array.from(ipBytes).join(".")}`);

        // Create the add_available_ip extrinsic call
        const call = api.tx.compute.addAvailableIp(ipBytes);

        // Wrap the call in a sudo extrinsic since it's restricted to root
        const sudoCall = api.tx.sudo.sudo(call);

        // Send the sudo call
        await new Promise((resolve, reject) => {
            sudoCall.signAndSend(sudoKeyPair, ({ status, dispatchError }) => {
                if (status.isInBlock) {
                    console.log(`Extrinsic included in block: ${status.asInBlock}`);
                } else if (status.isFinalized) {
                    console.log(`Extrinsic finalized in block: ${status.asFinalized}`);
                    resolve();
                }

                if (dispatchError) {
                    if (dispatchError.isModule) {
                        const decoded = api.registry.findMetaError(dispatchError.asModule);
                        const { docs, method, section } = decoded;
                        console.error(`Error: ${section}.${method}: ${docs.join(" ")}`);
                        reject(new Error(`Dispatch error: ${section}.${method}`));
                    } else {
                        console.error(`Error: ${dispatchError.toString()}`);
                        reject(new Error(`Dispatch error: ${dispatchError.toString()}`));
                    }
                }
            });
        });
    } catch (error) {
        console.error("Error in addAvailableIp:", error);
        throw error;
    }
}

async function setAvailableIps(api, seedPhrase) {
    const startIp = 0x0a000080; // 10.0.128.0
    const endIp = 0x0a00007fff; // 10.0.127.255

    for (let ip = startIp; ip <= endIp; ip++) {
        if (ip !== 0x0a000080 && ip !== 0x0a000081) {
            await addAvailableIp(api, seedPhrase, ip);
        }
    }
}

// Load environment variables from the .env file
dotenv.config();

// Example usage
(async () => {
  const seedPhrase = "brick end genuine caution author bulk school rose trap ramp garden milk";
  const wsProvider = new WsProvider("wss://testnet.hippius.com"); // Replace with your node's WebSocket URL
  const api = await ApiPromise.create({ provider: wsProvider });

  console.log("Connected to the local Substrate chain");

  try {
    // Set available IPs in the specified range
    await setAvailableIps(api, seedPhrase);
    console.log("All available IPs added successfully!");
  } catch (error) {
    console.error("Error during IP addition:", error);
  } finally {
    await api.disconnect(); // Disconnect the API
    console.log("Disconnected from the Substrate chain");
  }
})();