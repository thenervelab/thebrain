// Import required libraries
const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");
const dotenv = require("dotenv");

async function sendForceTransfer(api, seedPhrase, recipientAddress, amount) {
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

    // Create the `balances.forceTransfer` extrinsic call
    const call = api.tx.balances.forceTransfer(
      sudoKeyPair.address,
      recipientAddress,
      BigInt(amount)
    );

    // Wrap the call in a sudo extrinsic
    const sudoCall = api.tx.sudo.sudo(call);

    console.log(
      `Calling balances.forceTransfer with sudo for ${recipientAddress}`
    );

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
    console.error("Error in sendForceTransfer:", error);
    throw error; // Propagate the error to handle it in the caller function
  }
}

async function addPackage(
  api,
  seedPhrase,
  tier,
  storageGb,
  bandwidthGb,
  requestsLimit,
  priceUsd
) {
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

    // Create the marketplace.addPackage extrinsic call
    const call = api.tx.marketplace.addPackage(
      tier, // PackageTier enum value
      storageGb, // u32
      bandwidthGb, // u32
      requestsLimit, // u32
      priceUsd // u32
    );

    // Wrap the call in a sudo extrinsic since it's restricted to root
    const sudoCall = api.tx.sudo.sudo(call);

    console.log(`Calling marketplace.addPackage with sudo for tier: ${tier}`);

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
    console.error("Error in addPackage:", error);
    throw error;
  }
}

async function setTaoUsdValue(api, seedPhrase, price) {
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

    // Create the oracles.setTaoUsdValue extrinsic call
    const call = api.tx.oracle.setTaoUsdValue(price);

    // Wrap the call in a sudo extrinsic since it's restricted to root
    const sudoCall = api.tx.sudo.sudo(call);

    console.log(`Calling oracles.setTaoUsdValue with sudo for price: ${price}`);

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
    console.error("Error in setTaoUsdValue:", error);
    throw error;
  }
}

async function forceRegisterNode(
  api,
  seedPhrase,
  ownerAddress,
  nodeType,
  nodeId,
  ipfsNodeId = null
) {
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

    // Convert nodeId to proper format
    const nodeIdBytes = api.createType("Vec<u8>", nodeId);

    // Convert ipfsNodeId to proper format if provided
    const ipfsNodeIdOption = ipfsNodeId
      ? api.createType("Option<Vec<u8>>", api.createType("Vec<u8>", ipfsNodeId))
      : api.createType("Option<Vec<u8>>", null);

    // Create the registration.forceRegisterNode extrinsic call
    const call = api.tx.registration.forceRegisterNode(
      ownerAddress,
      { [nodeType]: null }, // Enum value: 'Validator', 'miner', or 'Relay'
      nodeIdBytes,
      ipfsNodeIdOption
    );

    // Wrap the call in a sudo extrinsic since it's restricted to root
    const sudoCall = api.tx.sudo.sudo(call);

    console.log(
      `Calling registration.forceRegisterNode with sudo for node: ${nodeId}`
    );

    // Send the sudo call and wait for it to be finalized
    return new Promise((resolve, reject) => {
      sudoCall
        .signAndSend(sudoKeyPair, ({ status, dispatchError }) => {
          if (status.isInBlock) {
            console.log(`Extrinsic included in block: ${status.asInBlock}`);
          } else if (status.isFinalized) {
            console.log(`Extrinsic finalized in block: ${status.asFinalized}`);
            resolve();
          }

          if (dispatchError) {
            if (dispatchError.isModule) {
              const decoded = api.registry.findMetaError(
                dispatchError.asModule
              );
              const { docs, method, section } = decoded;
              const error = `${section}.${method}: ${docs.join(" ")}`;
              console.error(`Error: ${error}`);
              reject(new Error(error));
            } else {
              const error = dispatchError.toString();
              console.error(`Error: ${error}`);
              reject(new Error(error));
            }
          }
        })
        .catch(reject);
    });
  } catch (error) {
    console.error("Error in forceRegisterNode:", error);
    throw error;
  }
}

async function registerNode(api, seedPhrase, nodeType, nodeId, ipfsNodeId) {
  try {
    // Create a keyring instance
    const keyring = new Keyring({ type: "sr25519" });

    // Create keypair from the seed phrase
    const keypair = keyring.addFromUri(seedPhrase);

    console.log(`Account address: ${keypair.address}`);

    // Convert nodeId to proper format
    const nodeIdBytes = api.createType("Vec<u8>", nodeId);

    // Convert ipfsNodeId to proper format
    const ipfsNodeIdOption = api.createType(
      "Option<Vec<u8>>",
      api.createType("Vec<u8>", ipfsNodeId)
    );

    // Create the registration.registerNode extrinsic call
    const call = api.tx.registration.registerNode(
      nodeType, // Enum value: 'Validator', 'miner', or 'Relay'
      nodeIdBytes,
      ipfsNodeIdOption
    );

    console.log(`Calling registration.registerNode for node: ${nodeId}`);

    // Send the call and wait for it to be finalized
    return new Promise((resolve, reject) => {
      call
        .signAndSend(keypair, ({ status, dispatchError }) => {
          if (status.isInBlock) {
            console.log(`Extrinsic included in block: ${status.asInBlock}`);
          } else if (status.isFinalized) {
            console.log(`Extrinsic finalized in block: ${status.asFinalized}`);
            resolve();
          }

          if (dispatchError) {
            if (dispatchError.isModule) {
              const decoded = api.registry.findMetaError(
                dispatchError.asModule
              );
              const { docs, method, section } = decoded;
              const error = `${section}.${method}: ${docs.join(" ")}`;
              console.error(`Error: ${error}`);
              reject(new Error(error));
            } else {
              const error = dispatchError.toString();
              console.error(`Error: ${error}`);
              reject(new Error(error));
            }
          }
        })
        .catch(reject);
    });
  } catch (error) {
    console.error("Error in registerNode:", error);
    throw error;
  }
}

async function bondStake(
    api,
    seedPhrase,
    amount,
    rewardDestination = 'Stash' // Default to Staked, could be 'Staked', 'Stash', 'Account', or 'None'
  ) {
    try {
      // Create a keyring instance
      const keyring = new Keyring({ type: "sr25519" });
  
      // Create keypair from seed phrase
      const keyPair = keyring.addFromUri(seedPhrase);
  
      console.log(`Account address: ${keyPair.address}`);
  
      // Convert amount to the proper format if needed
      const bondAmount = api.createType('Balance', amount);
  
      // Create the reward destination
      let rewardDest;
      if (typeof rewardDestination === 'string') {
        // Handle string enum cases
        rewardDest = rewardDestination;
      } else if (typeof rewardDestination === 'object' && rewardDestination.Account) {
        // Handle Account(accountId) case
        rewardDest = { Account: rewardDestination.Account };
      } else {
        rewardDest = 'Stash'; // Default case
      }
  
      // Create the staking.bond extrinsic call
      const bondCall = api.tx.staking.bond(
        bondAmount,
        rewardDest
      );
  
      console.log(`Calling staking.bond with amount: ${amount}`);
  
      // Sign and send the transaction
      await new Promise((resolve, reject) => {
        bondCall.signAndSend(keyPair, ({ status, dispatchError }) => {
          if (status.isInBlock) {
            console.log(`Transaction included in block: ${status.asInBlock}`);
          } else if (status.isFinalized) {
            console.log(`Transaction finalized in block: ${status.asFinalized}`);
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
      console.error("Error in bondStake:", error);
      throw error;
    }
  }

// Load environment variables from the .env file
// const dotenv = require('dotenv');
dotenv.config();

// Example usage
(async () => {
  const seedPhrase = process.env.seedPhrase;
  const miner1seedPhrase = process.env.miner1seedPhrase;
  const miner2seedPhrase = process.env.miner2seedPhrase;
  const miner3seedPhrase = process.env.miner3seedPhrase;
  const miner4seedPhrase = process.env.miner4seedPhrase;
  const miner1 = process.env.miner1;
  const miner2 = process.env.miner2;
  const miner3 = process.env.miner3;
  const dubsAccount = process.env.dubsAccount;
  const miner4 = process.env.miner4;
  const amount = BigInt("80000000000000000000");
  const stakeAmount = BigInt("40000000000000000000");

  const wsProvider = new WsProvider("ws://127.0.0.1:9944"); // Replace with your node's WebSocket URL
  const api = await ApiPromise.create({ provider: wsProvider });

  console.log("Connected to the local Substrate chain");

  try {
    // Perform transfers sequentially
    await sendForceTransfer(api, seedPhrase, miner1, amount);
    await sendForceTransfer(api, seedPhrase, miner2, amount);
    await sendForceTransfer(api, seedPhrase, miner3, amount);
    await sendForceTransfer(api, seedPhrase, miner4, amount);
    await sendForceTransfer(api, seedPhrase, dubsAccount, amount);

    console.log("All transfers completed successfully");

    // add packages of marketplace stoarge items
    await addPackage(
      api,
      seedPhrase,
      "Starter", // package enum variant
      100, // 100GB storage
      1000, // 1TB bandwidth
      10000, // 10k requests
      10 // $10 USD
    );
    await addPackage(
      api,
      seedPhrase,
      "Quantum", // package enum variant
      200, // 100GB storage
      2000, // 1TB bandwidth
      20000, // 10k requests
      20 // $10 USD
    );
    await addPackage(
      api,
      seedPhrase,
      "Pinnacle", // package enum variant
      300, // 100GB storage
      3000, // 1TB bandwidth
      30000, // 10k requests
      30 // $10 USD
    );

    console.log("Package added successfully!");

    await setTaoUsdValue(
      api,
      seedPhrase,
      600 // price in USD (as u32)
    );

    console.log("Tao Price is Set Successfully!");

    // register validator
    await forceRegisterNode(
      api,
      seedPhrase,
      "5CcSgTwvLZ7YPx8wYkhPWQBB9Wmc4VKm8w53Rs58FoQFA9LR", // owner address
      "Validator", // or whatever node type you're using
      "12D3KooWKmvC5Wq5wDE7axGnvHBwwKeb59RYRoFEokf2mugBuM32", // node ID
      "12D3KooWCziNpNYLpcYJPyi9e84djTwnYGgXLnbfypEtUcuhB9w1" // optional IPFS node ID
    );
    
    // registrer miners
    await forceRegisterNode(
        api,
        seedPhrase,
        "5Ceq7X969gaobMBBn5uKnr3GHcy9HC2NirRTjoG3H8ghGkgQ", // owner address
        "miner", // or whatever node type you're using
        "12D3KooWFqu2yQybo1qqXpaVLbu6y7ibgpmu3G27jiJ31EkhuFmm", // node ID
        "12D3KooWCziNpNYLpcYJPyi9e84djTwnYGgXLnbfypEtUcuhB9wk" // IPFS node ID (required for miner type)
    );
    await forceRegisterNode(
        api,
        seedPhrase,
        "5DcjX1rVkTgZtHbNSWh2zySXWLnpyYgSHrvmc5QmqbVAsh8d", // owner address
        "miner", // or whatever node type you're using
        "12D3KooWPa6UNsMJ1CpYrJATaD3e3jEz8zEzG3BtfCuufCh9dLPt", // node ID
        "12D3KooWBBfYC86inw9v9fzWNFUiKcczsdSAq2mG9CmEvHHhgBUN" // IPFS node ID (required for miner type)
    );
    await forceRegisterNode(
        api,
        seedPhrase,
        "5EHucbEoZ3fxXHkT7xfiDd2Hj1kLvERoiRcTHTdPwG7XtiVE", // owner address
        "miner", // or whatever node type you're using
        "12D3KooWNXseSiAUpvGJ4diQuwceFirjKkjNCwFL1mHipZPzDyHU", // node ID
        "12D3KooWHVeE4s7p7tbCixeoPM26LY7bqrLXtaBoaqzStnxVL2s7" // IPFS node ID (required for miner type)
    );
    await forceRegisterNode(
        api,
        seedPhrase,
        "5GEudEYMVWJr64Y3599urXfG1tg4u7iNFWmBYZUET2YTdPkn", // owner address
        "miner", // or whatever node type you're using
        "12D3KooWCdiAQzGAZhhw2MpbJLtEiuXes1jMQVR2qYxK5ypxXhxh", // node ID
        "12D3KooWDicea33t559ETmWUJEL2ToGxZA4CkgYpYgf6KBuRgjSh" // IPFS node ID (required for miner type)
    );
    console.log("miners registered successfully!");

    await bondStake(
        api,
        miner1seedPhrase,
        stakeAmount, // amount in smallest units (e.g., Planck)
        'Stash' // or { Account: 'specific_account_address' } for paying rewards to different account
    );
    await bondStake(
        api,
        miner2seedPhrase,
        stakeAmount, // amount in smallest units (e.g., Planck)
        'Stash' // or { Account: 'specific_account_address' } for paying rewards to different account
    );
    await bondStake(
        api,
        miner3seedPhrase,
        stakeAmount, // amount in smallest units (e.g., Planck)
        'Stash' // or { Account: 'specific_account_address' } for paying rewards to different account
    );
    await bondStake(
        api,
        miner4seedPhrase,
        stakeAmount, // amount in smallest units (e.g., Planck)
        'Stash' // or { Account: 'specific_account_address' } for paying rewards to different account
    );

    console.log("Bonded Tokens for miners successfully!");

  } catch (error) {
    console.error("Error during transfers:", error);
  } finally {
    await api.disconnect(); // Disconnect the API
    console.log("Disconnected from the Substrate chain");
  }
})();

function stringToU8a(str) {
  return new TextEncoder().encode(str);
}
