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
      console.log(`Calling add_available_ip with sudo for IP: ${ip}`);
  
      // Check if the provided account is the Sudo account
      const sudoKey = (await api.query.sudo.key()).toString();
      if (sudoKey !== sudoKeyPair.address) {
        throw new Error("The provided account is not the Sudo account");
      }

      // Create the add_available_ip extrinsic call
      const call = api.tx.palletIp.addAvailableIp(ip);

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
    const startIp = (10 << 24) | (0 << 16) | (80 << 8) | 1; // 10.0.80.1
    const endIp = (10 << 24) | (0 << 16) | (127 << 8) | 255; // 10.0.127.255

    for (let ip = startIp; ip <= endIp; ip++) {
        const ipStr = `${(ip >> 24) & 0xFF}.${(ip >> 16) & 0xFF}.${(ip >> 8) & 0xFF}.${ip & 0xFF}`;

        // Convert nodeId to proper format
        console.log(`Adding IP: ${ipStr}`);
        const ipBytes = api.createType("Vec<u8>", ipStr);
        
        // Ensure the byte array is in the correct format (Vec<u8>)
        await addAvailableIp(api, seedPhrase, ipBytes);
        
    }
}


// Load environment variables from the .env file
dotenv.config();

// Example usage
(async () => {
  const seedPhrase = "brick end genuine caution author bulk school rose trap ramp garden milk";
  const wsProvider = new WsProvider("wss://testnet.hippius.com"); // Replace with your node's WebSocket URL
  console.log("Connecting to the local Substrate chain...");
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